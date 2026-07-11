use std::collections::{HashMap, HashSet, VecDeque};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use backend_dx11::Dx11Backend;
use backend_dx12::Dx12Backend;
use backend_vulkan::VulkanBackend;
use bevy_ecs::prelude::*;
use engine_assets::{
    load_node_graph, load_scene_document, AssetBuildCache, AssetChange, AssetError, AssetHotReload,
    AssetKind, AudioEmitter, Collider2D, SceneComponents, SceneDocument, SceneObject,
    ScriptBinding, ShaderCompileOptions, ShaderSourceKind, ShaderTarget, Sprite2D, Transform2D,
};
use engine_audio::{audio_sync_system, AudioRuntime, AudioState};
use engine_core::{
    load_config_from_ron, BackendPreference, EngineConfig, EngineCoreError,
    SchedulerTopologyBias, SchedulerTuningConfig,
};
use engine_editor::{draw_overlay, EditorState, FrameTimings};
use engine_nodes::{
    compile_graph, CompileDiagnostic, CompiledGraphArtifact, DiagnosticAnchor, EcsJobDescriptor,
    NodeCompileError, NodeCompileOptions, NodeGraph, ScriptJobDescriptor,
};
use engine_physics::{physics_sync_system, PhysicsWorld};
use engine_platform::{
    available_backends_for_platform, choose_backend, PlatformError, RuntimePlatform,
};
use engine_render_api::{
    BackendCapabilities, BackendDiagnosticEvent, BackendDiagnosticLevel, BackendDiagnostics,
    BackendError, BackendKind, BlendMode, Camera2d, GraphicsBackend, RenderGraph,
    RenderGraphPass, SpriteBatchCommand, SpriteInstance, SurfaceConfig, SurfaceHandle,
    SurfaceWindowHandles, TextureHandle, ViewportReadback,
};
use rayon::prelude::*;
use rhai::{Dynamic, Engine as RhaiEngine, Scope, AST};
use rhai::Map as RhaiMap;
use thiserror::Error;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;
use winit::window::WindowAttributes;

#[derive(Debug, Clone, PartialEq)]
pub struct ViewportFrame {
    pub frame_index: u64,
    pub width: u32,
    pub height: u32,
    pub texture_id: Option<u64>,
    pub rgba8: Option<Vec<u8>>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeDiagnosticsSnapshot {
    pub compile_diagnostics: Vec<CompileDiagnostic>,
    pub diagnostic_anchors: Vec<DiagnosticAnchor>,
    pub backend_diagnostics: BackendDiagnostics,
    pub frame_timings: FrameTimings,
    pub telemetry: FallbackTelemetry,
    pub active_backend: BackendKind,
    pub script_scheduler_workers: usize,
    pub script_scheduler_min_parallel_jobs: usize,
    pub script_scheduler_topology_bias: SchedulerTopologyBias,
}

#[derive(Resource, Default)]
struct GraphRuntimeState {
    jobs: Vec<EcsJobDescriptor>,
    executed_frames: u64,
    executed_jobs: u64,
    executed_passes: u64,
}

#[derive(Resource, Default)]
struct FrameCounter(pub u64);

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct RuntimeInputState {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    mouse_x: f32,
    mouse_y: f32,
    mouse_left: bool,
    mouse_right: bool,
    mouse_middle: bool,
}

#[derive(Debug, Clone, Copy)]
struct UiCommand {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum UiCommandKind {
    Rect,
    Text,
}

impl UiCommand {
    fn rect(x: f32, y: f32, width: f32, height: f32, color: [u8; 4]) -> Self {
        Self {
            x,
            y,
            width,
            height,
            r: color[0],
            g: color[1],
            b: color[2],
            a: color[3],
        }
    }

    fn text(x: f32, y: f32, size: f32, color: [u8; 4]) -> Self {
        // Text is rendered as a small colored rectangle placeholder until a
        // real font atlas is available.
        Self {
            x,
            y,
            width: size * 4.0,
            height: size,
            r: color[0],
            g: color[1],
            b: color[2],
            a: color[3],
        }
    }
}


#[derive(Debug, Clone, Copy)]
struct ViewportReadbackBalancer {
    interval_frames: u32,
    frame_cursor: u32,
}

impl Default for ViewportReadbackBalancer {
    fn default() -> Self {
        Self {
            interval_frames: 1,
            frame_cursor: 0,
        }
    }
}

impl ViewportReadbackBalancer {
    fn should_capture(&mut self) -> bool {
        self.frame_cursor = self.frame_cursor.wrapping_add(1);
        self.frame_cursor.is_multiple_of(self.interval_frames.max(1))
    }

    fn update_from_frame_time(&mut self, frame_ms: f32, target_ms: f32) {
        if target_ms <= 0.0 {
            return;
        }

        let previous = self.interval_frames;
        if frame_ms > target_ms * 1.15 {
            self.interval_frames = (self.interval_frames + 1).min(4);
        } else if frame_ms < target_ms * 0.75 {
            self.interval_frames = self.interval_frames.saturating_sub(1).max(1);
        }

        if self.interval_frames != previous && self.frame_cursor >= self.interval_frames {
            self.frame_cursor %= self.interval_frames;
        }
    }

    fn interval_frames(self) -> u32 {
        self.interval_frames.max(1)
    }
}

impl RuntimeInputState {
    fn key_down(self, key: &str) -> bool {
        match key.trim().to_ascii_lowercase().as_str() {
            "left" | "a" | "arrowleft" => self.left,
            "right" | "d" | "arrowright" => self.right,
            "up" | "w" | "arrowup" => self.up,
            "down" | "s" | "arrowdown" => self.down,
            "mouse_left" | "mouseleft" | "lmb" => self.mouse_left,
            "mouse_right" | "mouseright" | "rmb" => self.mouse_right,
            "mouse_middle" | "mousemiddle" | "mmb" => self.mouse_middle,
            _ => false,
        }
    }

    fn mouse_down(self, button: &str) -> bool {
        match button.trim().to_ascii_lowercase().as_str() {
            "left" | "lmb" => self.mouse_left,
            "right" | "rmb" => self.mouse_right,
            "middle" | "mmb" => self.mouse_middle,
            _ => false,
        }
    }
}


#[derive(Debug, Clone, Default)]
struct ScriptHostState {
    scene: Option<SceneDocument>,
    events: Vec<String>,
    logs: Vec<String>,
    next_object_id: u64,
    input: RuntimeInputState,
    ui_commands: Vec<UiCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScriptSchedulerConfig {
    workers: usize,
    min_parallel_jobs: usize,
    topology_bias: SchedulerTopologyBias,
}

impl ScriptSchedulerConfig {
    fn from_engine_config(config: SchedulerTuningConfig) -> Self {
        let available = thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1)
            .max(1);

        if !config.enabled {
            return Self {
                workers: 1,
                min_parallel_jobs: usize::MAX,
                topology_bias: config.topology_bias,
            };
        }

        let usable = if config.reserve_main_thread {
            available.saturating_sub(1).max(1)
        } else {
            available
        };

        let biased = match config.topology_bias {
            SchedulerTopologyBias::Balanced => usable,
            SchedulerTopologyBias::PreferHighClock => usable.div_ceil(2).max(1),
            SchedulerTopologyBias::PreferManyCore => usable,
        };

        let min_workers = config.min_workers.max(1) as usize;
        let max_workers = config.max_workers.max(config.min_workers).max(1) as usize;
        let workers = biased.clamp(min_workers, max_workers);

        Self {
            workers,
            min_parallel_jobs: config.script_parallel_min_jobs.max(2) as usize,
            topology_bias: config.topology_bias,
        }
    }
}

struct ScriptRuntime {
    engine: RhaiEngine,
    ast_cache: HashMap<PathBuf, AST>,
    source_cache: HashMap<PathBuf, String>,
    host_state: Arc<Mutex<ScriptHostState>>,
    scheduler: ScriptSchedulerConfig,
    parallel_pool: Option<rayon::ThreadPool>,
}

impl ScriptHostState {
    fn scene_mut(&mut self) -> Option<&mut SceneDocument> {
        self.scene.as_mut()
    }

    fn with_object_mut<F>(&mut self, object_id: u64, mut update: F)
    where
        F: FnMut(&mut SceneObject),
    {
        if let Some(scene) = self.scene_mut() {
            if let Some(object) = scene
                .objects
                .iter_mut()
                .find(|object| object.object_id == object_id)
            {
                update(object);
            }
        }
    }

    fn push_ui_command(&mut self, command: UiCommand) {
        self.ui_commands.push(command);
    }

    fn drain_ui_commands(&mut self) -> Vec<UiCommand> {
        std::mem::take(&mut self.ui_commands)
    }
}

fn find_object_id_by_name(scene: &SceneDocument, name: &str) -> Option<u64> {
    scene
        .objects
        .iter()
        .find(|object| object.name == name)
        .map(|object| object.object_id)
}

fn collider_half_extents(collider: &Collider2D) -> (f32, f32) {
    if collider.shape.eq_ignore_ascii_case("circle") {
        let radius = if collider.radius > 0.0 {
            collider.radius
        } else {
            collider.width.abs().max(collider.height.abs()) * 0.5
        }
        .max(0.5);
        (radius, radius)
    } else {
        let half_w = if collider.width.abs() > 0.0 {
            collider.width.abs() * 0.5
        } else {
            collider.radius.max(0.5)
        };
        let half_h = if collider.height.abs() > 0.0 {
            collider.height.abs() * 0.5
        } else {
            collider.radius.max(0.5)
        };
        (half_w.max(0.5), half_h.max(0.5))
    }
}

struct Aabb {
    x: f32,
    y: f32,
    half_w: f32,
    half_h: f32,
}

impl Aabb {
    fn overlaps(self, other: Self) -> bool {
        (self.x - other.x).abs() <= self.half_w + other.half_w
            && (self.y - other.y).abs() <= self.half_h + other.half_h
    }
}

fn aabb_overlaps(a: Aabb, b: Aabb) -> bool {
    a.overlaps(b)
}

fn move_object_with_collision(scene: &mut SceneDocument, object_id: u64, dx: f32, dy: f32) -> bool {
    let Some(index) = scene
        .objects
        .iter()
        .position(|object| object.object_id == object_id)
    else {
        return false;
    };

    let current_transform = scene.objects[index].components.transform.clone();
    let Some(collider) = scene.objects[index].components.collider.clone() else {
        scene.objects[index].components.transform.x += dx;
        scene.objects[index].components.transform.y += dy;
        return true;
    };

    let next_x = current_transform.x + dx;
    let next_y = current_transform.y + dy;
    let (self_hx, self_hy) = collider_half_extents(&collider);

    let blocked = scene.objects.iter().enumerate().any(|(other_index, other)| {
        if other_index == index {
            return false;
        }
        let Some(other_collider) = &other.components.collider else {
            return false;
        };
        if other_collider.is_sensor {
            return false;
        }

        let (other_hx, other_hy) = collider_half_extents(other_collider);
        aabb_overlaps(
            Aabb {
                x: next_x,
                y: next_y,
                half_w: self_hx,
                half_h: self_hy,
            },
            Aabb {
                x: other.components.transform.x,
                y: other.components.transform.y,
                half_w: other_hx,
                half_h: other_hy,
            },
        )
    });

    if blocked {
        return false;
    }

    scene.objects[index].components.transform.x = next_x;
    scene.objects[index].components.transform.y = next_y;
    true
}

fn normalize_legacy_rhai_source(source: &str) -> String {
    // Older seeded templates used `let mut`, which Rhai does not support.
    source.replace("let mut ", "let ")
}

fn clamp_sprite_dimension(value: i64) -> u32 {
    // Ensure positive, non-zero dimensions and avoid truncation overflow.
    (value.max(1) as u64).min(u32::MAX as u64) as u32
}

fn parse_bool_setting(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptJobAccessProfile {
    /// Job may access arbitrary global state; it cannot run in parallel with any other job.
    Global,
    /// Job declares explicit read/write sets. Two jobs with scoped profiles conflict iff any
    /// write set intersects another job's read or write set.
    Scoped {
        reads: HashSet<String>,
        writes: HashSet<String>,
    },
}

impl ScriptJobAccessProfile {
    fn conflicts_with(&self,
        other: &ScriptJobAccessProfile,
    ) -> bool {
        match (self, other) {
            (ScriptJobAccessProfile::Global, _) | (_, ScriptJobAccessProfile::Global) => true,
            (
                ScriptJobAccessProfile::Scoped { reads: a_reads, writes: a_writes },
                ScriptJobAccessProfile::Scoped { reads: b_reads, writes: b_writes },
            ) => {
                // WAW, RAW, and WAR conflicts are all unsafe for parallel execution.
                !a_writes.is_disjoint(b_writes)
                    || !a_writes.is_disjoint(b_reads)
                    || !b_writes.is_disjoint(a_reads)
            }
        }
    }
}

fn parse_comma_separated_set(value: &str) -> HashSet<String> {
    value
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

#[derive(Debug, Clone)]
struct ParallelScriptInvocation {
    job: ScriptJobDescriptor,
    normalized_source: String,
}

fn create_rhai_engine(host_state: Arc<Mutex<ScriptHostState>>) -> RhaiEngine {
    let mut engine = RhaiEngine::new();

    {
        let host = host_state.clone();
        engine.on_print(move |message| {
            if let Ok(mut state) = host.lock() {
                state.logs.push(message.to_string());
            }
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("spawn_object", move |name: &str| -> i64 {
            let Ok(mut state) = host.lock() else {
                return -1;
            };
            let mut next_id = if state.next_object_id == 0 {
                1000
            } else {
                state.next_object_id
            };
            if let Some(scene) = state.scene.as_ref() {
                let max_existing = scene
                    .objects
                    .iter()
                    .map(|object| object.object_id)
                    .max()
                    .unwrap_or(0);
                next_id = next_id.max(max_existing.saturating_add(1));
            }
            state.next_object_id = next_id.saturating_add(1);
            let object = SceneObject {
                object_id: next_id,
                parent: None,
                layer_id: 1,
                name: name.to_string(),
                tags: Vec::new(),
                components: SceneComponents::default(),
            };
            if let Some(scene) = state.scene_mut() {
                scene.objects.push(object);
            }
            next_id as i64
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("despawn_object", move |id: i64| {
            if id < 0 {
                return;
            }
            if let Ok(mut state) = host.lock() {
                if let Some(scene) = state.scene_mut() {
                    scene.objects.retain(|object| object.object_id != id as u64);
                }
            }
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn(
            "set_transform",
            move |id: i64, x: f32, y: f32, rot: f32, sx: f32, sy: f32| {
                if let Ok(mut state) = host.lock() {
                    state.with_object_mut(id as u64, |object| {
                        object.components.transform = Transform2D {
                            x,
                            y,
                            rotation_radians: rot,
                            scale_x: sx,
                            scale_y: sy,
                        };
                    });
                }
            },
        );
    }

    {
        let host = host_state.clone();
        engine.register_fn("get_transform_x", move |id: i64| -> f64 {
            let Ok(state) = host.lock() else {
                return 0.0;
            };
            state
                .scene
                .as_ref()
                .and_then(|scene| scene.objects.iter().find(|object| object.object_id == id as u64))
                .map(|object| object.components.transform.x as f64)
                .unwrap_or(0.0)
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("get_transform_y", move |id: i64| -> f64 {
            let Ok(state) = host.lock() else {
                return 0.0;
            };
            state
                .scene
                .as_ref()
                .and_then(|scene| scene.objects.iter().find(|object| object.object_id == id as u64))
                .map(|object| object.components.transform.y as f64)
                .unwrap_or(0.0)
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn(
            "set_transform",
            move |id: i64, x: f64, y: f64, rot: f64, sx: f64, sy: f64| {
                if let Ok(mut state) = host.lock() {
                    state.with_object_mut(id as u64, |object| {
                        object.components.transform = Transform2D {
                            x: x as f32,
                            y: y as f32,
                            rotation_radians: rot as f32,
                            scale_x: sx as f32,
                            scale_y: sy as f32,
                        };
                    });
                }
            },
        );
    }

    {
        let host = host_state.clone();
        engine.register_fn(
            "set_sprite",
            move |id: i64, texture: &str, width: i64, height: i64| {
                if let Ok(mut state) = host.lock() {
                    state.with_object_mut(id as u64, |object| {
                        object.components.sprite = Some(Sprite2D {
                            texture_asset: texture.to_string(),
                            width: clamp_sprite_dimension(width),
                            height: clamp_sprite_dimension(height),
                            tint_rgba: [255, 255, 255, 255],
                            layer_order: 0,
                        });
                    });
                }
            },
        );
    }

    {
        let host = host_state.clone();
        engine.register_fn("set_custom", move |id: i64, key: &str, value: &str| {
            if let Ok(mut state) = host.lock() {
                state.with_object_mut(id as u64, |object| {
                    object
                        .components
                        .custom_properties
                        .insert(key.to_string(), value.to_string());
                });
            }
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("emit_event", move |name: &str, payload: &str| {
            if let Ok(mut state) = host.lock() {
                state.events.push(format!("{name}:{payload}"));
            }
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("set_audio", move |id: i64, asset: &str, volume: f32| {
            if let Ok(mut state) = host.lock() {
                state.with_object_mut(id as u64, |object| {
                    object.components.audio = Some(AudioEmitter {
                        asset: asset.to_string(),
                        volume,
                        looping: false,
                        spatial_blend: 0.0,
                    });
                });
            }
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("set_audio", move |id: i64, asset: &str, volume: f64| {
            if let Ok(mut state) = host.lock() {
                state.with_object_mut(id as u64, |object| {
                    object.components.audio = Some(AudioEmitter {
                        asset: asset.to_string(),
                        volume: volume as f32,
                        looping: false,
                        spatial_blend: 0.0,
                    });
                });
            }
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("set_collider_radius", move |id: i64, radius: f32| {
            if let Ok(mut state) = host.lock() {
                state.with_object_mut(id as u64, |object| {
                    object.components.collider = Some(Collider2D {
                        shape: "circle".to_string(),
                        radius,
                        width: radius * 2.0,
                        height: radius * 2.0,
                        is_sensor: false,
                    });
                });
            }
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("set_collider_radius", move |id: i64, radius: f64| {
            if let Ok(mut state) = host.lock() {
                state.with_object_mut(id as u64, |object| {
                    let radius = radius as f32;
                    object.components.collider = Some(Collider2D {
                        shape: "circle".to_string(),
                        radius,
                        width: radius * 2.0,
                        height: radius * 2.0,
                        is_sensor: false,
                    });
                });
            }
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn(
            "set_script",
            move |id: i64, script_asset: &str, entry: &str, frame_phase: &str| {
                if let Ok(mut state) = host.lock() {
                    state.with_object_mut(id as u64, |object| {
                        object.components.script = Some(ScriptBinding {
                            script_asset: script_asset.to_string(),
                            entry: entry.to_string(),
                            frame_phase: frame_phase.to_string(),
                        });
                    });
                }
            },
        );
    }

    {
        let host = host_state.clone();
        engine.register_fn("object_count", move || -> i64 {
            let Ok(state) = host.lock() else {
                return 0;
            };
            state
                .scene
                .as_ref()
                .map(|scene| scene.objects.len() as i64)
                .unwrap_or(0)
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("find_object", move |name: &str| -> i64 {
            let Ok(state) = host.lock() else {
                return -1;
            };
            state
                .scene
                .as_ref()
                .and_then(|scene| find_object_id_by_name(scene, name))
                .map(|id| id as i64)
                .unwrap_or(-1)
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("find_objects_with_tag", move |tag: &str| -> Vec<i64> {
            let Ok(state) = host.lock() else {
                return Vec::new();
            };
            let Some(scene) = state.scene.as_ref() else {
                return Vec::new();
            };
            let needle = tag.trim().to_ascii_lowercase();
            scene
                .objects
                .iter()
                .filter(|object| object.tags.iter().any(|t| t.to_ascii_lowercase() == needle))
                .map(|object| object.object_id as i64)
                .collect()
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("get_custom", move |id: i64, key: &str| -> String {
            let Ok(state) = host.lock() else {
                return String::new();
            };

            state
                .scene
                .as_ref()
                .and_then(|scene| {
                    scene
                        .objects
                        .iter()
                        .find(|object| object.object_id == id as u64)
                })
                .and_then(|object| object.components.custom_properties.get(key).cloned())
                .unwrap_or_default()
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("get_custom_f32", move |id: i64, key: &str, fallback: f32| -> f32 {
            let Ok(state) = host.lock() else {
                return fallback;
            };

            state
                .scene
                .as_ref()
                .and_then(|scene| {
                    scene
                        .objects
                        .iter()
                        .find(|object| object.object_id == id as u64)
                })
                .and_then(|object| object.components.custom_properties.get(key))
                .and_then(|value| value.trim().parse::<f32>().ok())
                .unwrap_or(fallback)
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("get_custom_f32", move |id: i64, key: &str, fallback: f64| -> f64 {
            let Ok(state) = host.lock() else {
                return fallback;
            };

            state
                .scene
                .as_ref()
                .and_then(|scene| {
                    scene
                        .objects
                        .iter()
                        .find(|object| object.object_id == id as u64)
                })
                .and_then(|object| object.components.custom_properties.get(key))
                .and_then(|value| value.trim().parse::<f64>().ok())
                .unwrap_or(fallback)
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("key_down", move |key: &str| -> bool {
            let Ok(state) = host.lock() else {
                return false;
            };
            state.input.key_down(key)
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("move_with_collision", move |id: i64, dx: f32, dy: f32| -> bool {
            let Ok(mut state) = host.lock() else {
                return false;
            };
            let Some(scene) = state.scene_mut() else {
                return false;
            };
            move_object_with_collision(scene, id as u64, dx, dy)
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("move_with_collision", move |id: i64, dx: f64, dy: f64| -> bool {
            let Ok(mut state) = host.lock() else {
                return false;
            };
            let Some(scene) = state.scene_mut() else {
                return false;
            };
            move_object_with_collision(scene, id as u64, dx as f32, dy as f32)
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("mouse_x", move || -> f64 {
            let Ok(state) = host.lock() else {
                return 0.0;
            };
            state.input.mouse_x as f64
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("mouse_y", move || -> f64 {
            let Ok(state) = host.lock() else {
                return 0.0;
            };
            state.input.mouse_y as f64
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("mouse_down", move |button: &str| -> bool {
            let Ok(state) = host.lock() else {
                return false;
            };
            state.input.mouse_down(button)
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("draw_rect", move |x: f32, y: f32, width: f32, height: f32, r: i64, g: i64, b: i64, a: i64| {
            let Ok(mut state) = host.lock() else {
                return;
            };
            let color = [
                r.clamp(0, 255) as u8,
                g.clamp(0, 255) as u8,
                b.clamp(0, 255) as u8,
                a.clamp(0, 255) as u8,
            ];
            state.push_ui_command(UiCommand::rect(x, y, width, height, color));
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("draw_rect", move |x: f64, y: f64, width: f64, height: f64, r: i64, g: i64, b: i64, a: i64| {
            let Ok(mut state) = host.lock() else {
                return;
            };
            let color = [
                r.clamp(0, 255) as u8,
                g.clamp(0, 255) as u8,
                b.clamp(0, 255) as u8,
                a.clamp(0, 255) as u8,
            ];
            state.push_ui_command(UiCommand::rect(x as f32, y as f32, width as f32, height as f32, color));
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("draw_text", move |_text: &str, x: f32, y: f32, size: f32, r: i64, g: i64, b: i64, a: i64| {
            let Ok(mut state) = host.lock() else {
                return;
            };
            let color = [
                r.clamp(0, 255) as u8,
                g.clamp(0, 255) as u8,
                b.clamp(0, 255) as u8,
                a.clamp(0, 255) as u8,
            ];
            // Text is rendered as a colored rectangle placeholder until a font
            // atlas is integrated into the renderer.
            state.push_ui_command(UiCommand::text(x, y, size, color));
        });
    }

    {
        let host = host_state.clone();
        engine.register_fn("draw_text", move |_text: &str, x: f64, y: f64, size: f64, r: i64, g: i64, b: i64, a: i64| {
            let Ok(mut state) = host.lock() else {
                return;
            };
            let color = [
                r.clamp(0, 255) as u8,
                g.clamp(0, 255) as u8,
                b.clamp(0, 255) as u8,
                a.clamp(0, 255) as u8,
            ];
            state.push_ui_command(UiCommand::text(x as f32, y as f32, size as f32, color));
        });
    }

    engine
}

impl ScriptRuntime {
    fn new(config: SchedulerTuningConfig) -> Self {
        let host_state = Arc::new(Mutex::new(ScriptHostState {
            scene: None,
            events: Vec::new(),
            logs: Vec::new(),
            next_object_id: 1000,
            input: RuntimeInputState::default(),
            ui_commands: Vec::new(),
        }));

        let engine = create_rhai_engine(host_state.clone());
        let scheduler = ScriptSchedulerConfig::from_engine_config(config);
        let parallel_pool = if scheduler.workers > 1 {
            rayon::ThreadPoolBuilder::new()
                .num_threads(scheduler.workers)
                .thread_name(|index| format!("script-worker-{index}"))
                .build()
                .ok()
        } else {
            None
        };

        Self {
            engine,
            ast_cache: HashMap::new(),
            source_cache: HashMap::new(),
            host_state,
            scheduler,
            parallel_pool,
        }
    }

    fn set_scene(&mut self, scene: Option<SceneDocument>) {
        if let Ok(mut state) = self.host_state.lock() {
            state.scene = scene;
        }
    }

    fn set_input(&self, input: RuntimeInputState) {
        if let Ok(mut state) = self.host_state.lock() {
            state.input = input;
        }
    }

    fn drain_ui_commands(&self,
    ) -> Vec<UiCommand> {
        self.host_state
            .lock()
            .ok()
            .map(|mut state| state.drain_ui_commands())
            .unwrap_or_default()
    }

    fn take_scene(&self) -> Option<SceneDocument> {
        self.host_state
            .lock()
            .ok()
            .and_then(|state| state.scene.clone())
    }

    fn scheduler_workers(&self) -> usize {
        self.scheduler.workers
    }

    fn scheduler_min_parallel_jobs(&self) -> usize {
        self.scheduler.min_parallel_jobs
    }

    fn scheduler_topology_bias(&self) -> SchedulerTopologyBias {
        self.scheduler.topology_bias
    }

    fn drain_host_state(&mut self) {
        if let Ok(mut state) = self.host_state.lock() {
            state.events.clear();
            state.logs.clear();
        }
    }

    fn execute_jobs(&mut self, jobs: &[ScriptJobDescriptor]) -> Result<(), EngineAppError> {
        if jobs.is_empty() {
            return Ok(());
        }

        for wave in Self::build_script_waves(jobs) {
            let wave_jobs = wave
                .iter()
                .filter_map(|index| jobs.get(*index))
                .collect::<Vec<_>>();

            if self.can_parallelize_wave(&wave_jobs) {
                self.execute_wave_parallel(&wave_jobs)?;
            } else {
                for job in wave_jobs {
                    self.execute_single_job(job)?;
                }
            }
        }

        Ok(())
    }

    fn execute_single_job(&mut self, job: &ScriptJobDescriptor) -> Result<(), EngineAppError> {
        let script_path = PathBuf::from(&job.script_asset);
        let ast = self.compile_cached_ast(&script_path)?;
        Self::execute_job_with_ast(&self.engine, &ast, job)
    }

    fn execute_wave_parallel(
        &mut self,
        wave_jobs: &[&ScriptJobDescriptor],
    ) -> Result<(), EngineAppError> {
        let mut invocations = Vec::with_capacity(wave_jobs.len());
        for job in wave_jobs {
            let script_path = PathBuf::from(&job.script_asset);
            let source = self.normalized_script_source(&script_path)?.to_string();
            invocations.push(ParallelScriptInvocation {
                job: (*job).clone(),
                normalized_source: source,
            });
        }

        let host_state = self.host_state.clone();

        let run_wave = || {
            invocations
                .par_iter()
                .map(|invocation| {
                    let engine = create_rhai_engine(host_state.clone());
                    let ast = engine
                        .compile(&invocation.normalized_source)
                        .map_err(|err| {
                            EngineAppError::Runtime(format!(
                                "script compile error in {}: {err}",
                                invocation.job.script_asset
                            ))
                        })?;
                    Self::execute_job_with_ast(&engine, &ast, &invocation.job)
                })
                .collect::<Vec<_>>()
        };

        let results = if let Some(pool) = &self.parallel_pool {
            pool.install(run_wave)
        } else {
            run_wave()
        };

        for result in results {
            result?;
        }

        Ok(())
    }

    fn compile_cached_ast(&mut self, script_path: &Path) -> Result<AST, EngineAppError> {
        if let Some(cached) = self.ast_cache.get(script_path) {
            return Ok(cached.clone());
        }

        let source = self.normalized_script_source(script_path)?.to_string();
        let compiled = self.engine.compile(&source).map_err(|err| {
            EngineAppError::Runtime(format!(
                "script compile error in {}: {err}",
                script_path.display()
            ))
        })?;
        self.ast_cache
            .insert(script_path.to_path_buf(), compiled.clone());
        Ok(compiled)
    }

    fn normalized_script_source(&mut self, script_path: &Path) -> Result<&str, EngineAppError> {
        if !self.source_cache.contains_key(script_path) {
            let source = fs::read_to_string(script_path).map_err(|err| {
                EngineAppError::Runtime(format!(
                    "failed to read script {}: {err}",
                    script_path.display()
                ))
            })?;
            let normalized = normalize_legacy_rhai_source(&source);
            self.source_cache
                .insert(script_path.to_path_buf(), normalized);
        }

        Ok(self
            .source_cache
            .get(script_path)
            .map(String::as_str)
            .unwrap_or_default())
    }

    fn execute_job_with_ast(
        engine: &RhaiEngine,
        ast: &AST,
        job: &ScriptJobDescriptor,
    ) -> Result<(), EngineAppError> {
        let mut scope = Scope::new();
        scope.push("node_id", job.node_id as i64);
        scope.push("node_name", job.node_name.clone());
        scope.push("frame_phase", job.frame_phase.clone());

        let mut settings = RhaiMap::new();
        for (key, value) in &job.settings {
            settings.insert(key.clone().into(), Dynamic::from(value.clone()));
        }
        scope.push("settings", settings);

        let _ = engine
            .call_fn::<Dynamic>(&mut scope, ast, &job.entry, ())
            .map_err(|err| EngineAppError::Runtime(format!("script runtime error: {err}")))?;

        Ok(())
    }

    fn can_parallelize_wave(&self, wave_jobs: &[&ScriptJobDescriptor]) -> bool {
        if self.parallel_pool.is_none() {
            return false;
        }

        if wave_jobs.len() < self.scheduler.min_parallel_jobs {
            return false;
        }

        let profiles: Vec<_> = wave_jobs
            .iter()
            .map(|job| Self::job_access_profile(job))
            .collect();

        for i in 0..profiles.len() {
            for j in (i + 1)..profiles.len() {
                if profiles[i].conflicts_with(&profiles[j]) {
                    return false;
                }
            }
        }

        true
    }

    fn job_access_profile(job: &ScriptJobDescriptor) -> ScriptJobAccessProfile {
        let explicitly_safe = job
            .settings
            .get("parallel_safe")
            .and_then(|raw| parse_bool_setting(raw))
            .unwrap_or(false);

        if !explicitly_safe {
            return ScriptJobAccessProfile::Global;
        }

        // Prefer explicit read/write sets when available.
        let read_set = job
            .settings
            .get("read_set")
            .map(|value| parse_comma_separated_set(value))
            .unwrap_or_default();
        let write_set = job
            .settings
            .get("write_set")
            .map(|value| parse_comma_separated_set(value))
            .unwrap_or_default();

        if !read_set.is_empty() || !write_set.is_empty() {
            return ScriptJobAccessProfile::Scoped { reads: read_set, writes: write_set };
        }

        // Fall back to a single synthetic write key for the legacy parallel-safe
        // markers. This preserves behavior for existing nodes that rely on
        // script_parallel_key / object_id / object_name / layer_id.
        let legacy_key = job
            .settings
            .get("script_parallel_key")
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                job.settings
                    .get("object_id")
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .map(|id| format!("object:{id}"))
            })
            .or_else(|| {
                job.settings
                    .get("object_name")
                    .map(|value| value.trim().to_ascii_lowercase())
                    .filter(|value| !value.is_empty())
                    .map(|name| format!("object_name:{name}"))
            })
            .or_else(|| {
                job.settings
                    .get("layer_id")
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .map(|id| format!("layer:{id}"))
            });

        if let Some(key) = legacy_key {
            let mut writes = HashSet::new();
            writes.insert(key);
            return ScriptJobAccessProfile::Scoped {
                reads: HashSet::new(),
                writes,
            };
        }

        ScriptJobAccessProfile::Global
    }

    fn build_script_waves(jobs: &[ScriptJobDescriptor]) -> Vec<Vec<usize>> {
        if jobs.len() <= 1 {
            return vec![(0..jobs.len()).collect()];
        }

        let script_index = jobs
            .iter()
            .enumerate()
            .map(|(index, job)| (job.node_id, index))
            .collect::<HashMap<_, _>>();

        let mut indegree = vec![0usize; jobs.len()];
        let mut edges = vec![Vec::<usize>::new(); jobs.len()];

        for (index, job) in jobs.iter().enumerate() {
            for dependency in &job.dependencies {
                if let Some(&dep_index) = script_index.get(dependency) {
                    indegree[index] += 1;
                    edges[dep_index].push(index);
                }
            }
        }

        let mut ready = indegree
            .iter()
            .enumerate()
            .filter_map(|(index, degree)| (*degree == 0).then_some(index))
            .collect::<VecDeque<_>>();

        let mut waves = Vec::new();
        let mut processed = 0usize;

        while !ready.is_empty() {
            let wave_len = ready.len();
            let mut wave = Vec::with_capacity(wave_len);

            for _ in 0..wave_len {
                if let Some(index) = ready.pop_front() {
                    wave.push(index);
                }
            }

            wave.sort_unstable();

            for index in &wave {
                processed += 1;
                for dependent in &edges[*index] {
                    indegree[*dependent] = indegree[*dependent].saturating_sub(1);
                    if indegree[*dependent] == 0 {
                        ready.push_back(*dependent);
                    }
                }
            }

            waves.push(wave);
        }

        if processed != jobs.len() {
            tracing::warn!(
                "script dependency graph contains a cycle or missing dependency; \
                 falling back to sequential execution"
            );
            vec![(0..jobs.len()).collect()]
        } else {
            waves
        }
    }

    fn invalidate_script(&mut self, script_path: &Path) {
        self.ast_cache.remove(script_path);
        self.source_cache.remove(script_path);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FallbackTelemetry {
    pub compile_fallback_events: u64,
    pub compile_failures: u64,
    pub shader_rebuild_errors: u64,
    pub recovery_events: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HotReloadReport {
    pub changed_assets: usize,
    pub shaders_rebuilt: usize,
    pub scene_recompiled: bool,
    pub had_errors: bool,
}

fn frame_counter_system(mut counter: ResMut<FrameCounter>) {
    counter.0 += 1;
}

fn graph_runtime_system(mut state: ResMut<GraphRuntimeState>) {
    state.executed_frames += 1;
    state.executed_jobs += state.jobs.len() as u64;
}

#[derive(Debug, Error)]
pub enum EngineAppError {
    #[error(transparent)]
    Backend(#[from] BackendError),

    #[error(transparent)]
    Platform(#[from] PlatformError),

    #[error(transparent)]
    Asset(#[from] AssetError),

    #[error(transparent)]
    NodeCompile(#[from] NodeCompileError),

    #[error(transparent)]
    Core(#[from] EngineCoreError),

    #[error("audio runtime init failed: {0}")]
    Audio(String),

    #[error("no scene loaded")]
    NoSceneLoaded,

    #[error("backend recovery failed after {0} attempts")]
    RecoveryAttemptsExceeded(u32),

    #[error("runtime error: {0}")]
    Runtime(String),
}

pub struct EngineApp {
    config: EngineConfig,
    backend_override: Option<BackendPreference>,
    platform: RuntimePlatform,
    active_backend: BackendKind,
    backend: Box<dyn GraphicsBackend>,
    surface: SurfaceHandle,

    fixed_schedule: Schedule,
    gameplay_schedule: Schedule,
    audio_schedule: Schedule,
    world: World,

    assets: AssetHotReload,
    build_cache: AssetBuildCache,
    scene_path: Option<PathBuf>,
    scene_graph: Option<NodeGraph>,
    authored_scene: Option<SceneDocument>,
    current_scene: Option<SceneDocument>,
    compiled_graph: Option<CompiledGraphArtifact>,
    last_viewport_readback: Option<ViewportReadback>,

    editor_state: EditorState,
    frame_timings: FrameTimings,
    backend_diagnostics: BackendDiagnostics,
    egui_context: egui::Context,
    surface_window: Option<SurfaceWindowHandles>,

    last_frame_instant: Instant,
    fixed_step_accumulator: f32,
    recovery_attempts: u32,
    telemetry: FallbackTelemetry,
    is_play_mode: bool,
    input_state: RuntimeInputState,
    viewport_camera: Camera2d,
    viewport_readback_balancer: ViewportReadbackBalancer,
    script_runtime: ScriptRuntime,

    #[allow(dead_code)]
    audio_runtime: AudioRuntime,
}

impl EngineApp {
    pub fn from_config_path(path: impl AsRef<Path>) -> Result<Self, EngineAppError> {
        let config = load_config_from_ron(path)?;
        Self::new(config)
    }

    pub fn new(config: EngineConfig) -> Result<Self, EngineAppError> {
        let platform = RuntimePlatform::current();
        let available = available_backends_for_platform(platform);
        let active_backend = choose_backend(config.backend_preference, &available, platform)?;

        let mut backend = create_backend(active_backend);
        backend.initialize(&config)?;
        let surface = backend.create_surface(SurfaceConfig::from_engine_config(&config), None)?;

        let audio_runtime = match AudioRuntime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                tracing::warn!("falling back to silent audio runtime: {err}");
                AudioRuntime::silent()
            }
        };

        let mut world = World::new();
        world.insert_resource(PhysicsWorld::default());
        world.insert_resource(AudioState::default());
        world.insert_resource(GraphRuntimeState::default());
        world.insert_resource(FrameCounter::default());

        let mut fixed_schedule = Schedule::default();
        fixed_schedule.add_systems(physics_sync_system);

        let mut gameplay_schedule = Schedule::default();
        gameplay_schedule.add_systems((graph_runtime_system, frame_counter_system));

        let mut audio_schedule = Schedule::default();
        audio_schedule.add_systems(audio_sync_system);

        let backend_diagnostics = backend.diagnostics();
        let script_runtime = ScriptRuntime::new(config.scheduler_tuning);

        Ok(Self {
            config,
            backend_override: None,
            platform,
            active_backend,
            backend,
            surface,
            fixed_schedule,
            gameplay_schedule,
            audio_schedule,
            world,
            assets: AssetHotReload::new(),
            build_cache: AssetBuildCache::new(),
            scene_path: None,
            scene_graph: None,
            authored_scene: None,
            current_scene: None,
            compiled_graph: None,
            last_viewport_readback: None,
            editor_state: EditorState::default(),
            frame_timings: FrameTimings::default(),
            backend_diagnostics,
            egui_context: egui::Context::default(),
            surface_window: None,
            last_frame_instant: Instant::now(),
            fixed_step_accumulator: 0.0,
            recovery_attempts: 0,
            telemetry: FallbackTelemetry::default(),
            is_play_mode: false,
            input_state: RuntimeInputState::default(),
            viewport_camera: Camera2d::default(),
            viewport_readback_balancer: ViewportReadbackBalancer::default(),
            script_runtime,
            audio_runtime,
        })
    }

    pub fn start_play_mode(&mut self) {
        if !self.is_play_mode {
            self.reset_runtime_scene_from_authored();
        }
        self.is_play_mode = true;
        self.editor_state.is_playing = true;
    }

    pub fn stop_play_mode(&mut self) {
        self.is_play_mode = false;
        self.editor_state.is_playing = false;
        self.reset_runtime_scene_from_authored();
    }

    pub fn restart_play_mode(&mut self) {
        self.reset_runtime_scene_from_authored();
        self.is_play_mode = true;
        self.editor_state.is_playing = true;
    }

    pub fn is_play_mode(&self) -> bool {
        self.is_play_mode
    }

    pub fn set_keyboard_input(&mut self, left: bool, right: bool, up: bool, down: bool) {
        self.input_state.left = left;
        self.input_state.right = right;
        self.input_state.up = up;
        self.input_state.down = down;
        self.sync_input_to_script_runtime();
    }

    pub fn set_mouse_input(
        &mut self,
        x: f32,
        y: f32,
        left: bool,
        right: bool,
        middle: bool,
    ) {
        self.input_state.mouse_x = x;
        self.input_state.mouse_y = y;
        self.input_state.mouse_left = left;
        self.input_state.mouse_right = right;
        self.input_state.mouse_middle = middle;
        self.sync_input_to_script_runtime();
    }

    fn sync_input_to_script_runtime(&self,
    ) {
        self.script_runtime.set_input(self.input_state);
    }


    pub fn set_viewport_camera(&mut self, x: f32, y: f32, zoom: f32) {
        self.viewport_camera = Camera2d {
            x,
            y,
            zoom: zoom.max(0.05),
        };
    }

    pub fn step_play_frame(&mut self) -> Result<(), EngineAppError> {
        let was_playing = self.is_play_mode;
        if !was_playing {
            self.reset_runtime_scene_from_authored();
        }
        self.is_play_mode = true;
        let result = self.run_for_frames(1);
        self.is_play_mode = was_playing;
        self.editor_state.is_playing = self.is_play_mode;
        if !self.is_play_mode {
            self.reset_runtime_scene_from_authored();
        }
        result
    }

    pub fn set_active_scene_graph(&mut self, graph: NodeGraph) -> Result<bool, EngineAppError> {
        let scene = SceneDocument::from_graph(graph);
        self.set_active_scene(scene)
    }

    pub fn set_active_scene(&mut self, scene: SceneDocument) -> Result<bool, EngineAppError> {
        self.authored_scene = Some(scene.clone());
        self.current_scene = Some(scene.clone());
        self.scene_graph = Some(scene.graph.clone());
        self.script_runtime.set_scene(Some(scene.clone()));
        self.compile_and_apply_graph(scene.graph)
    }

    fn reset_runtime_scene_from_authored(&mut self) {
        if let Some(scene) = self.authored_scene.clone() {
            self.current_scene = Some(scene.clone());
            self.scene_graph = Some(scene.graph.clone());
            self.script_runtime.set_scene(Some(scene));
        }
    }

    pub fn viewport_frame(&self) -> ViewportFrame {
        let frame_index = self.runtime_tick();

        let (width, height) = self.viewport_dimensions();

        ViewportFrame {
            frame_index,
            width,
            height,
            texture_id: None,
            rgba8: self
                .last_viewport_readback
                .as_ref()
                .map(|readback| readback.rgba8.clone()),
            source: self.viewport_source().to_string(),
        }
    }

    pub fn runtime_tick(&self) -> u64 {
        self.world
            .get_resource::<FrameCounter>()
            .map(|counter| counter.0)
            .unwrap_or(0)
    }

    pub fn viewport_dimensions(&self) -> (u32, u32) {
        self.last_viewport_readback
            .as_ref()
            .map(|readback| (readback.width, readback.height))
            .unwrap_or((self.config.window.width, self.config.window.height))
    }

    pub fn viewport_source(&self) -> &'static str {
        if self.last_viewport_readback.is_some() {
            "backend_readback"
        } else {
            "none"
        }
    }

    pub fn viewport_readback(&self) -> Option<&ViewportReadback> {
        self.last_viewport_readback.as_ref()
    }

    pub fn viewport_readback_interval_frames(&self) -> u32 {
        self.viewport_readback_balancer.interval_frames()
    }

    pub fn diagnostics_snapshot(&self) -> RuntimeDiagnosticsSnapshot {
        let compile_diagnostics = self
            .compiled_graph
            .as_ref()
            .map(|artifact| artifact.diagnostics.clone())
            .unwrap_or_default();

        RuntimeDiagnosticsSnapshot {
            compile_diagnostics,
            diagnostic_anchors: self
                .compiled_graph
                .as_ref()
                .map(|artifact| artifact.diagnostic_anchors.clone())
                .unwrap_or_default(),
            backend_diagnostics: self.backend_diagnostics.clone(),
            frame_timings: self.frame_timings.clone(),
            telemetry: self.telemetry.clone(),
            active_backend: self.active_backend,
            script_scheduler_workers: self.script_runtime.scheduler_workers(),
            script_scheduler_min_parallel_jobs: self.script_runtime.scheduler_min_parallel_jobs(),
            script_scheduler_topology_bias: self.script_runtime.scheduler_topology_bias(),
        }
    }

    pub fn attach_window(&mut self, window: &Window) -> Result<(), EngineAppError> {
        let window_handle = window
            .window_handle()
            .map_err(|err| BackendError::Surface(format!("window handle error: {err}")))?;
        let display_handle = window
            .display_handle()
            .map_err(|err| BackendError::Surface(format!("display handle error: {err}")))?;

        let handles = SurfaceWindowHandles {
            window_handle: window_handle.as_raw(),
            display_handle: display_handle.as_raw(),
        };

        let config = SurfaceConfig::from_engine_config(&self.config);
        let surface = self.backend.create_surface(config, Some(handles))?;
        self.surface = surface;
        self.surface_window = Some(handles);

        Ok(())
    }

    pub fn load_scene(&mut self, path: impl AsRef<Path>) -> Result<(), EngineAppError> {
        let scene_path = path.as_ref().to_path_buf();
        let compiled = match load_scene_document(&scene_path) {
            Ok(scene) => self.set_active_scene(scene)?,
            Err(_) => {
                let graph = load_node_graph(&scene_path)?;
                self.set_active_scene_graph(graph)?
            }
        };
        if !compiled {
            self.push_backend_event(
                BackendDiagnosticLevel::Warning,
                None,
                None,
                format!(
                    "scene {} failed to compile; keeping previous valid runtime graph",
                    scene_path.display()
                ),
            );
        }
        self.scene_path = Some(scene_path);
        Ok(())
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn set_backend_override(
        &mut self,
        backend_preference: BackendPreference,
    ) -> Result<(), EngineAppError> {
        let effective_preference = if backend_preference == BackendPreference::Auto {
            self.config.backend_preference
        } else {
            self.backend_override = Some(backend_preference);
            backend_preference
        };

        let available = available_backends_for_platform(self.platform);
        let selected = choose_backend(effective_preference, &available, self.platform)?;

        if selected != self.active_backend {
            self.backend.destroy()?;
            let mut backend = create_backend(selected);
            backend.initialize(&self.config)?;
            let surface = backend.create_surface(
                SurfaceConfig::from_engine_config(&self.config),
                self.surface_window,
            )?;

            self.backend = backend;
            self.active_backend = selected;
            self.surface = surface;

            if let Some(scene_graph) = self.scene_graph.clone() {
                let _ = self.compile_and_apply_graph(scene_graph)?;
            }
        }

        Ok(())
    }

    pub fn resize_surface(&mut self, width: u32, height: u32) -> Result<(), EngineAppError> {
        self.backend.resize(self.surface, width, height)?;
        Ok(())
    }

    pub fn run(&mut self) -> Result<(), EngineAppError> {
        self.run_for_frames(1)
    }

    pub fn run_for_frames(&mut self, frame_count: u32) -> Result<(), EngineAppError> {
        for _ in 0..frame_count {
            if let Err(err) = self.run_single_frame() {
                if self.try_recover(&err)? {
                    continue;
                }
                return Err(err);
            }
        }

        Ok(())
    }

    fn run_single_frame(&mut self) -> Result<(), EngineAppError> {
        let frame_start = Instant::now();
        let delta = self.consume_delta_time();

        self.run_input_phase();
        self.run_fixed_update_phase(delta);
        self.run_gameplay_phase()?;
        self.run_audio_phase();
        self.run_render_phase()?;

        self.frame_timings.cpu_frame_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        self.viewport_readback_balancer
            .update_from_frame_time(self.frame_timings.cpu_frame_ms, self.target_frame_ms());
        self.backend_diagnostics = self.backend.diagnostics();
        self.frame_timings.gpu_frame_ms = self.backend_diagnostics.last_gpu_frame_ms;

        self.draw_editor_overlay();
        self.apply_frame_pacing(frame_start);

        Ok(())
    }

    fn consume_delta_time(&mut self) -> f32 {
        let now = Instant::now();
        let delta = now
            .saturating_duration_since(self.last_frame_instant)
            .as_secs_f32();
        self.last_frame_instant = now;
        delta
    }

    fn run_input_phase(&mut self) {
        // Input is collected in the editor host and synchronized into script runtime state here.
        self.script_runtime.set_input(self.input_state);
    }

    fn run_fixed_update_phase(&mut self, delta_seconds: f32) {
        let fixed = &self.config.fixed_step;
        let fixed_dt = 1.0_f32 / fixed.hz.max(1.0);
        self.fixed_step_accumulator =
            (self.fixed_step_accumulator + delta_seconds).min(fixed.max_catch_up_seconds);

        let mut executed_steps = 0_u32;
        while self.fixed_step_accumulator >= fixed_dt && executed_steps < fixed.max_steps_per_frame
        {
            self.fixed_schedule.run(&mut self.world);
            self.fixed_step_accumulator -= fixed_dt;
            executed_steps += 1;
        }

        if self.fixed_step_accumulator >= fixed_dt {
            self.fixed_step_accumulator = 0.0;
            self.push_backend_event(
                BackendDiagnosticLevel::Warning,
                None,
                None,
                "fixed-step clamp triggered; accumulator reset to prevent spiral-of-death",
            );
        }
    }

    fn run_gameplay_phase(&mut self) -> Result<(), EngineAppError> {
        if self.is_play_mode {
            self.gameplay_schedule.run(&mut self.world);
            if let Some(artifact) = &self.compiled_graph {
                self.script_runtime.execute_jobs(&artifact.script_jobs)?;
                self.script_runtime.drain_host_state();
                if let Some(scene) = self.script_runtime.take_scene() {
                    self.current_scene = Some(scene);
                }
            }
        }
        Ok(())
    }

    fn run_audio_phase(&mut self) {
        self.audio_schedule.run(&mut self.world);
    }

    fn run_render_phase(&mut self) -> Result<(), EngineAppError> {
        let capture_viewport_readback = self.viewport_readback_balancer.should_capture();
        let frame = self.backend.acquire_frame(self.surface)?;

        if let Some(artifact) = &self.compiled_graph {
            let mut submission_graph = artifact.render_graph.clone();
            self.inject_scene_sprites_into_render_graph(&mut submission_graph);
            self.inject_ui_commands_into_render_graph(&mut submission_graph);
            self.apply_viewport_camera_to_render_graph(&mut submission_graph);
            optimize_submission_graph(&mut submission_graph);
            self.preload_shaders_for_graph(&submission_graph)?;
            self.backend.record_render_graph(frame, &submission_graph)?;

            if let Some(mut runtime_state) = self.world.get_resource_mut::<GraphRuntimeState>() {
                runtime_state.executed_passes += submission_graph.passes.len() as u64;
            }
        } else {
            self.backend
                .record_render_graph(frame, &RenderGraph::empty())?;
        }

        self.backend.submit(frame)?;
        self.backend.present(frame)?;
        if capture_viewport_readback {
            match self.backend.readback_viewport() {
                Ok(readback) => {
                    self.last_viewport_readback = readback;
                }
                Err(err) => {
                    self.push_backend_event(
                        BackendDiagnosticLevel::Warning,
                        Some(frame.0),
                        None,
                        format!("viewport readback failed: {err}"),
                    );
                }
            }
        }
        Ok(())
    }

    fn preload_shaders_for_graph(
        &mut self,
        graph: &RenderGraph,
    ) -> Result<(), EngineAppError> {
        let target = shader_target_for_backend(self.active_backend);
        let mut seen = HashSet::<String>::new();

        for pass in &graph.passes {
            let material = match pass {
                RenderGraphPass::Render(node) => node.material.as_ref(),
                RenderGraphPass::Compute(node) => Some(&node.material),
            };
            let Some(material) = material else {
                continue;
            };
            if material.shader_asset.is_empty() || !seen.insert(material.shader_asset.clone()) {
                continue;
            }

            let path = Path::new(&material.shader_asset);
            let source_kind = shader_source_kind(path).unwrap_or(ShaderSourceKind::Hlsl);
            let options = ShaderCompileOptions {
                toolchain: self.config.shader_toolchain.clone(),
                optimization: "O2".to_string(),
                include_dirs: vec![path.parent().unwrap_or(Path::new(".")).to_path_buf()],
            };

            match self.build_cache.build_or_reuse_shader(path, source_kind, target, &options) {
                Ok((artifact, _)) => {
                    self.backend
                        .preload_shader_bytecode(
                            &material.shader_asset,
                            &artifact.metadata.entry_point,
                            &artifact.bytecode,
                        )
                        .map_err(EngineAppError::Backend)?;
                }
                Err(err) => {
                    self.push_backend_event(
                        BackendDiagnosticLevel::Warning,
                        None,
                        None,
                        format!(
                            "shader preload failed for {}: {err}; using built-in fallback",
                            material.shader_asset
                        ),
                    );
                }
            }
        }

        Ok(())
    }

    fn target_frame_ms(&self) -> f32 {
        if self.config.frame_pacing.target_fps > 0 {
            1000.0 / self.config.frame_pacing.target_fps as f32
        } else {
            self.config.perf_gate.max_frame_ms.max(1.0)
        }
    }

    fn inject_scene_sprites_into_render_graph(&self, graph: &mut RenderGraph) {
        let Some(scene) = self.current_scene.as_ref() else {
            return;
        };

        let scene_sprites = sprites_from_scene(scene);
        if scene_sprites.is_empty() {
            return;
        }

        for pass in &mut graph.passes {
            if let RenderGraphPass::Render(render_pass) = pass {
                if let Some(batch) = render_pass.batches.first_mut() {
                    batch.sprites = scene_sprites.clone();
                } else {
                    render_pass.batches.push(SpriteBatchCommand {
                        label: "scene::sprites".to_string(),
                        blend: BlendMode::Alpha,
                        target: render_pass.target,
                        sprites: scene_sprites.clone(),
                    });
                }
            }
        }
    }

    fn apply_viewport_camera_to_render_graph(&self, graph: &mut RenderGraph) {
        for pass in &mut graph.passes {
            if let RenderGraphPass::Render(render_pass) = pass {
                render_pass.camera = self.viewport_camera;
            }
        }
    }

    fn inject_ui_commands_into_render_graph(&self, graph: &mut RenderGraph) {
        let ui_commands = self.script_runtime.drain_ui_commands();
        if ui_commands.is_empty() {
            return;
        }

        let dummy_texture = TextureHandle(0);
        let sprites: Vec<SpriteInstance> = ui_commands
            .into_iter()
            .map(|cmd| SpriteInstance {
                texture: dummy_texture,
                x: cmd.x,
                y: cmd.y,
                width: cmd.width,
                height: cmd.height,
                rotation_radians: 0.0,
                tint: [
                    cmd.r as f32 / 255.0,
                    cmd.g as f32 / 255.0,
                    cmd.b as f32 / 255.0,
                    cmd.a as f32 / 255.0,
                ],
            })
            .collect();

        if sprites.is_empty() {
            return;
        }

        for pass in &mut graph.passes {
            if let RenderGraphPass::Render(render_pass) = pass {
                render_pass.batches.push(SpriteBatchCommand {
                    label: "ui_overlay".to_string(),
                    blend: BlendMode::Alpha,
                    target: render_pass.target,
                    sprites,
                });
                return;
            }
        }
    }

    fn apply_frame_pacing(&self, frame_start: Instant) {
        if !self.config.frame_pacing.sleep_enabled {
            return;
        }

        if self.config.frame_pacing.target_fps == 0 {
            return;
        }

        let target = Duration::from_secs_f32(1.0 / self.config.frame_pacing.target_fps as f32);
        let spin_threshold = Duration::from_millis(2);

        // Sleep for the bulk of the remaining time, then spin-yield for the
        // last couple of milliseconds to avoid OS sleep granularity overshoot
        // (especially noticeable on Windows, where Sleep can drift by 10-15 ms).
        loop {
            let elapsed = frame_start.elapsed();
            if elapsed >= target {
                return;
            }
            let remaining = target - elapsed;
            if remaining >= spin_threshold {
                thread::sleep(remaining - spin_threshold);
            } else {
                thread::yield_now();
            }
        }
    }

    fn try_recover(&mut self, err: &EngineAppError) -> Result<bool, EngineAppError> {
        let policy = self.config.recovery_policy;

        match err {
            EngineAppError::Backend(backend_err) if backend_err.is_recoverable_surface() => {
                if !policy.recover_surface_out_of_date {
                    return Ok(false);
                }
                self.recover_surface_out_of_date()?;
                Ok(true)
            }
            EngineAppError::Backend(backend_err) if backend_err.is_recoverable_device() => {
                if !policy.recover_device_loss {
                    return Ok(false);
                }
                self.recover_backend_loss()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn recover_surface_out_of_date(&mut self) -> Result<(), EngineAppError> {
        self.recovery_attempts += 1;
        if self.recovery_attempts > self.config.recovery_policy.max_recovery_attempts {
            return Err(EngineAppError::RecoveryAttemptsExceeded(
                self.recovery_attempts,
            ));
        }

        let surface = self.backend.create_surface(
            SurfaceConfig::from_engine_config(&self.config),
            self.surface_window,
        )?;
        self.surface = surface;
        self.telemetry.recovery_events += 1;
        self.backend_diagnostics.mark_swapchain_recreate();
        self.push_backend_event(
            BackendDiagnosticLevel::Warning,
            None,
            None,
            "surface out-of-date recovered by recreating surface/swapchain",
        );

        Ok(())
    }

    fn recover_backend_loss(&mut self) -> Result<(), EngineAppError> {
        self.recovery_attempts += 1;
        if self.recovery_attempts > self.config.recovery_policy.max_recovery_attempts {
            return Err(EngineAppError::RecoveryAttemptsExceeded(
                self.recovery_attempts,
            ));
        }

        let mut backend = create_backend(self.active_backend);
        backend.initialize(&self.config)?;
        let surface = backend.create_surface(
            SurfaceConfig::from_engine_config(&self.config),
            self.surface_window,
        )?;
        self.backend = backend;
        self.surface = surface;

        if let Some(scene_graph) = self.scene_graph.clone() {
            let _ = self.compile_and_apply_graph(scene_graph)?;
        }

        self.telemetry.recovery_events += 1;
        self.backend_diagnostics.mark_device_loss();
        self.push_backend_event(
            BackendDiagnosticLevel::Error,
            None,
            None,
            "device loss recovered by backend reinitialization",
        );

        Ok(())
    }

    fn compile_and_apply_graph(&mut self, graph: NodeGraph) -> Result<bool, EngineAppError> {
        let compile_options = NodeCompileOptions {
            strict_gpu: self.config.shader_toolchain.strict,
            ..NodeCompileOptions::default()
        };

        let compile_start = Instant::now();
        let compile_result = compile_graph(
            &graph,
            &compile_options,
            self.active_backend,
            self.backend.capabilities(),
        );
        self.frame_timings.node_compile_ms = compile_start.elapsed().as_secs_f32() * 1000.0;

        let artifact = match compile_result {
            Ok(artifact) => artifact,
            Err(err) => {
                self.telemetry.compile_failures += 1;
                self.push_backend_event(
                    BackendDiagnosticLevel::Error,
                    None,
                    None,
                    format!("graph compile failed: {err}"),
                );
                if self.compiled_graph.is_some() {
                    return Ok(false);
                }
                return Err(EngineAppError::NodeCompile(err));
            }
        };

        let fallback_count = artifact.diagnostics.len() as u64;
        if fallback_count > 0 {
            self.telemetry.compile_fallback_events += fallback_count;
            for _ in 0..fallback_count {
                self.backend_diagnostics.mark_fallback();
            }
        }

        if let Some(mut runtime_state) = self.world.get_resource_mut::<GraphRuntimeState>() {
            runtime_state.jobs = artifact.ecs_jobs.clone();
        }

        self.scene_graph = Some(graph.clone());
        if let Some(scene) = self.current_scene.as_mut() {
            scene.graph = graph;
            self.script_runtime.set_scene(Some(scene.clone()));
        }
        self.compiled_graph = Some(artifact);
        self.editor_state
            .mark_graph_compiled(self.telemetry.compile_fallback_events as usize);

        Ok(true)
    }

    pub fn mark_graph_dirty(&mut self) {
        self.editor_state.mark_graph_dirty();
    }

    pub fn hot_recompile_if_needed(&mut self) -> Result<bool, EngineAppError> {
        if !self.editor_state.graph_dirty {
            return Ok(false);
        }

        let path = self
            .scene_path
            .clone()
            .ok_or(EngineAppError::NoSceneLoaded)?;
        let graph = load_node_graph(path)?;
        self.compile_and_apply_graph(graph)
    }

    pub fn poll_asset_changes(&mut self, root: impl AsRef<Path>) -> Result<usize, EngineAppError> {
        let changes = self.assets.scan_changes(root)?;
        Ok(changes.len())
    }

    pub fn apply_hot_reload(
        &mut self,
        root: impl AsRef<Path>,
    ) -> Result<HotReloadReport, EngineAppError> {
        let changes = self.assets.scan_changes(root)?;
        let mut report = HotReloadReport {
            changed_assets: changes.len(),
            ..HotReloadReport::default()
        };

        for change in &changes {
            self.process_asset_change(change, &mut report)?;
        }

        Ok(report)
    }

    fn process_asset_change(
        &mut self,
        change: &AssetChange,
        report: &mut HotReloadReport,
    ) -> Result<(), EngineAppError> {
        if change
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rhai"))
        {
            self.script_runtime.invalidate_script(&change.path);
            self.push_backend_event(
                BackendDiagnosticLevel::Info,
                None,
                None,
                format!("script cache invalidated for {}", change.path.display()),
            );
        }

        if matches!(change.kind, AssetKind::Shader) {
            self.build_cache.invalidate(&change.path);
            let source_kind = shader_source_kind(&change.path).unwrap_or(ShaderSourceKind::Hlsl);
            let target = shader_target_for_backend(self.active_backend);
            let options = ShaderCompileOptions {
                toolchain: self.config.shader_toolchain.clone(),
                optimization: "O2".to_string(),
                include_dirs: vec![change.path.parent().unwrap_or(Path::new(".")).to_path_buf()],
            };

            match self.build_cache.build_or_reuse_shader(
                &change.path,
                source_kind,
                target,
                &options,
            ) {
                Ok((_artifact, _reused)) => {
                    report.shaders_rebuilt += 1;
                }
                Err(err) => {
                    report.had_errors = true;
                    self.telemetry.shader_rebuild_errors += 1;
                    self.push_backend_event(
                        BackendDiagnosticLevel::Error,
                        None,
                        None,
                        format!("shader rebuild failed for {}: {err}", change.path.display()),
                    );
                }
            }
        }

        if matches!(change.kind, AssetKind::Graph) {
            if let Some(scene_path) = &self.scene_path {
                if *scene_path == change.path {
                    match load_scene_document(scene_path) {
                        Ok(scene) => {
                            let recompiled = self.set_active_scene(scene)?;
                            report.scene_recompiled = recompiled;
                        }
                        Err(_) => {
                            let graph = load_node_graph(scene_path)?;
                            let recompiled = self.compile_and_apply_graph(graph)?;
                            report.scene_recompiled = recompiled;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn draw_editor_overlay(&mut self) {
        let diagnostics = self
            .compiled_graph
            .as_ref()
            .map(|artifact| artifact.diagnostics.as_slice())
            .unwrap_or(&[]);

        let raw_input = egui::RawInput::default();
        let active_backend = self.active_backend;
        let capabilities = self.backend.capabilities();
        let frame_timings = self.frame_timings.clone();
        let backend_diagnostics = self.backend_diagnostics.clone();

        let _ = self.egui_context.run(raw_input, |ctx| {
            draw_overlay(
                ctx,
                &mut self.editor_state,
                &frame_timings,
                active_backend,
                capabilities,
                diagnostics,
                &backend_diagnostics,
            );
        });
    }

    fn push_backend_event(
        &mut self,
        level: BackendDiagnosticLevel,
        frame: Option<u64>,
        pass: Option<String>,
        message: impl Into<String>,
    ) {
        self.backend_diagnostics.push_event(BackendDiagnosticEvent {
            level,
            frame,
            pass,
            message: message.into(),
        });
    }

    pub fn window_attributes(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title(self.config.window.title.clone())
            .with_inner_size(winit::dpi::PhysicalSize::new(
                self.config.window.width,
                self.config.window.height,
            ))
            .with_resizable(self.config.window.resizable)
    }

    pub fn active_backend(&self) -> BackendKind {
        self.active_backend
    }

    pub fn compiled_graph(&self) -> Option<&CompiledGraphArtifact> {
        self.compiled_graph.as_ref()
    }

    pub fn backend_diagnostics(&self) -> &BackendDiagnostics {
        &self.backend_diagnostics
    }

    pub fn cpu_frame_ms(&self) -> f32 {
        self.frame_timings.cpu_frame_ms
    }

    pub fn backend_capabilities(&self) -> BackendCapabilities {
        self.backend.capabilities()
    }

    pub fn editor_state(&self) -> &EditorState {
        &self.editor_state
    }

    pub fn telemetry(&self) -> &FallbackTelemetry {
        &self.telemetry
    }

    pub fn set_frame_pacing_sleep_enabled(&mut self, enabled: bool) {
        self.config.frame_pacing.sleep_enabled = enabled;
    }
}

fn create_backend(kind: BackendKind) -> Box<dyn GraphicsBackend> {
    match kind {
        BackendKind::Vulkan => Box::new(VulkanBackend::new()),
        BackendKind::Dx12 => Box::new(Dx12Backend::new()),
        BackendKind::Dx11 => Box::new(Dx11Backend::new()),
    }
}

fn sprites_from_scene(scene: &SceneDocument) -> Vec<SpriteInstance> {
    let layer_meta = scene
        .layers
        .iter()
        .map(|layer| (layer.layer_id, (layer.order, layer.visible)))
        .collect::<HashMap<_, _>>();

    let mut ordered = Vec::new();

    for object in &scene.objects {
        let (layer_order, layer_visible) = layer_meta
            .get(&object.layer_id)
            .copied()
            .unwrap_or((0, true));
        if !layer_visible {
            continue;
        }

        let Some(sprite) = object.components.sprite.as_ref() else {
            continue;
        };

        let transform = &object.components.transform;
        let tint = if sprite.tint_rgba == [255, 255, 255, 255] {
            fallback_tint_from_asset(&sprite.texture_asset)
        } else {
            tint_from_rgba(sprite.tint_rgba)
        };
        ordered.push((
            layer_order,
            sprite.layer_order,
            object.object_id,
            SpriteInstance {
                texture: texture_handle_from_asset(&sprite.texture_asset),
                x: transform.x,
                y: transform.y,
                width: (sprite.width as f32 * transform.scale_x.abs()).max(1.0),
                height: (sprite.height as f32 * transform.scale_y.abs()).max(1.0),
                rotation_radians: transform.rotation_radians,
                tint,
            },
        ));
    }

    ordered.sort_by_key(|(layer_order, sprite_order, object_id, _)| {
        (*layer_order, *sprite_order, *object_id)
    });

    ordered.into_iter().map(|(_, _, _, sprite)| sprite).collect()
}

fn tint_from_rgba(rgba: [u8; 4]) -> [f32; 4] {
    [
        rgba[0] as f32 / 255.0,
        rgba[1] as f32 / 255.0,
        rgba[2] as f32 / 255.0,
        rgba[3] as f32 / 255.0,
    ]
}

fn fallback_tint_from_asset(asset: &str) -> [f32; 4] {
    let lower = asset.to_ascii_lowercase();
    if lower.contains("square") {
        [0.92, 0.34, 0.32, 1.0]
    } else if lower.contains("circle") {
        [0.32, 0.74, 0.95, 1.0]
    } else if lower.contains("triangle") {
        [0.96, 0.82, 0.28, 1.0]
    } else {
        [0.86, 0.86, 0.86, 1.0]
    }
}

fn texture_handle_from_asset(asset: &str) -> TextureHandle {
    let lower = asset.to_ascii_lowercase();
    if lower.contains("square") {
        return TextureHandle(0);
    }
    if lower.contains("circle") {
        return TextureHandle(1);
    }
    if lower.contains("triangle") {
        return TextureHandle(2);
    }

    let mut hasher = DefaultHasher::new();
    lower.hash(&mut hasher);
    TextureHandle((hasher.finish() % 32) + 3)
}

fn optimize_submission_graph(graph: &mut RenderGraph) {
    for pass in &mut graph.passes {
        if let RenderGraphPass::Render(render) = pass {
            for batch in &mut render.batches {
                let blend_key = blend_sort_key(batch.blend);
                batch
                    .sprites
                    .sort_by_key(|sprite| (sprite.texture.0, blend_key));
            }
            render.batches.sort_by_key(|batch| {
                let texture = batch
                    .sprites
                    .first()
                    .map(|sprite| sprite.texture.0)
                    .unwrap_or(0);
                (texture, blend_sort_key(batch.blend))
            });
        }
    }
}

fn blend_sort_key(blend: engine_render_api::BlendMode) -> u8 {
    match blend {
        engine_render_api::BlendMode::Alpha => 0,
        engine_render_api::BlendMode::Additive => 1,
        engine_render_api::BlendMode::Multiply => 2,
    }
}

fn shader_source_kind(path: &Path) -> Option<ShaderSourceKind> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "glsl" | "vert" | "frag" | "comp" => Some(ShaderSourceKind::Glsl),
        "hlsl" => Some(ShaderSourceKind::Hlsl),
        _ => None,
    }
}

fn shader_target_for_backend(backend: BackendKind) -> ShaderTarget {
    match backend {
        BackendKind::Vulkan => ShaderTarget::VulkanSpirv,
        BackendKind::Dx12 => ShaderTarget::Dx12Dxil,
        BackendKind::Dx11 => ShaderTarget::Dx11Dxbc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_assets::save_node_graph;
    use engine_nodes::{
        ComputeDispatchConfig, GpuResourceAccess, Node, NodeExecutionTarget, NodeFallbackPolicy,
        NodeGraph, NodeKind, NodePayload, ScriptJobDescriptor,
    };
    use std::collections::BTreeMap;

    fn sample_graph() -> NodeGraph {
        NodeGraph {
            version: engine_nodes::CURRENT_GRAPH_VERSION,
            nodes: vec![
                Node {
                    id: 1,
                    name: "start".to_string(),
                    kind: NodeKind::GameplayEvent,
                    target: NodeExecutionTarget::Cpu,
                    dependencies: vec![],
                    settings: BTreeMap::new(),
                    gpu_bindings: vec![],
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![],
                    shader_entry: None,
                    shader_profile: None,
                    payload: Some(NodePayload::GameplayEvent(Default::default())),
                },
                Node {
                    id: 2,
                    name: "compute_particles".to_string(),
                    kind: NodeKind::ComputePass,
                    target: NodeExecutionTarget::Hybrid,
                    dependencies: vec![1],
                    settings: BTreeMap::from([
                        ("shader".to_string(), "particles.hlsl".to_string()),
                        (
                            "write_resources".to_string(),
                            "particles_buffer".to_string(),
                        ),
                    ]),
                    gpu_bindings: vec![],
                    compute: Some(ComputeDispatchConfig { x: 4, y: 4, z: 1 }),
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![engine_nodes::NodeGpuResourceState {
                        resource: "particles_buffer".to_string(),
                        access: GpuResourceAccess::Write,
                    }],
                    shader_entry: Some("cs_main".to_string()),
                    shader_profile: Some("cs_6_6".to_string()),
                    payload: Some(NodePayload::ComputePass(Default::default())),
                },
                Node {
                    id: 3,
                    name: "render".to_string(),
                    kind: NodeKind::RenderPass,
                    target: NodeExecutionTarget::Gpu,
                    dependencies: vec![2],
                    settings: BTreeMap::from([
                        ("sprite_count".to_string(), "4".to_string()),
                        ("blend".to_string(), "alpha".to_string()),
                    ]),
                    gpu_bindings: vec![],
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![],
                    shader_entry: Some("vs_main".to_string()),
                    shader_profile: Some("ps_6_0".to_string()),
                    payload: Some(NodePayload::RenderPass(Default::default())),
                },
            ],
        }
    }

    #[test]
    fn backend_contract_has_required_features() {
        let backends: Vec<Box<dyn GraphicsBackend>> = vec![
            Box::new(VulkanBackend::new()),
            Box::new(Dx12Backend::new()),
            Box::new(Dx11Backend::new()),
        ];

        for backend in backends {
            assert!(
                backend.capabilities().supports_required_2d(),
                "backend {:?} must satisfy required 2D features",
                backend.kind(),
            );
        }
    }

    #[test]
    fn linux_platform_selects_vulkan_by_default() {
        let selected = choose_backend(
            BackendPreference::Auto,
            &[BackendKind::Dx11, BackendKind::Vulkan],
            RuntimePlatform::Linux,
        )
        .expect("backend should be selected");
        assert_eq!(selected, BackendKind::Vulkan);
    }

    #[test]
    fn hot_recompile_updates_compiled_artifact() {
        let temp_dir = std::env::temp_dir().join("rusty_engine_app_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let scene_path = temp_dir.join("scene.ron");

        let mut app = EngineApp::new(EngineConfig::default()).expect("app should initialize");

        let graph_a = sample_graph();
        save_node_graph(&scene_path, &graph_a).expect("graph should save");
        app.load_scene(&scene_path).expect("scene should load");
        let first_pass_count = app
            .compiled_graph()
            .expect("compiled graph should exist")
            .render_graph
            .passes
            .len();

        let mut graph_b = sample_graph();
        graph_b.nodes.push(Node {
            id: 4,
            name: "build".to_string(),
            kind: NodeKind::BuildExport,
            target: NodeExecutionTarget::Hybrid,
            dependencies: vec![3],
            settings: BTreeMap::new(),
            gpu_bindings: vec![],
            compute: None,
            fallback_policy: NodeFallbackPolicy::Cpu,
            gpu_resource_states: vec![],
            shader_entry: None,
            shader_profile: None,
            payload: Some(NodePayload::BuildExport(Default::default())),
        });

        save_node_graph(&scene_path, &graph_b).expect("graph should save");
        app.mark_graph_dirty();
        let recompiled = app
            .hot_recompile_if_needed()
            .expect("hot recompile should run");

        assert!(recompiled);
        let second_pass_count = app
            .compiled_graph()
            .expect("compiled graph should exist")
            .render_graph
            .passes
            .len();
        assert!(second_pass_count > first_pass_count);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn run_updates_backend_diagnostics() {
        let mut app = EngineApp::new(EngineConfig::default()).expect("app should initialize");
        app.run_for_frames(1).expect("frame should run");
        assert!(
            !app.backend_diagnostics().events.is_empty()
                || app.backend_diagnostics().last_cpu_frame_ms >= 0.0
        );
    }

    #[test]
    fn apply_hot_reload_reports_changes() {
        let temp_dir = std::env::temp_dir().join("rusty_engine_hot_reload");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");

        let shader_path = temp_dir.join("test.hlsl");
        std::fs::write(&shader_path, "//@entry main\n").expect("shader should write");

        let mut app = EngineApp::new(EngineConfig::default()).expect("app should initialize");
        let report = app
            .apply_hot_reload(&temp_dir)
            .expect("hot reload should succeed");

        assert!(report.changed_assets >= 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn runtime_input_state_maps_directional_keys() {
        let input = RuntimeInputState {
            left: true,
            right: false,
            up: true,
            down: false,
            ..Default::default()
        };
        assert!(input.key_down("left"));
        assert!(input.key_down("a"));
        assert!(input.key_down("ArrowLeft"));
        assert!(input.key_down("up"));
        assert!(input.key_down("w"));
        assert!(!input.key_down("right"));
        assert!(!input.key_down("unknown"));
    }

    #[test]
    fn viewport_readback_balancer_defaults_to_one() {
        let mut balancer = ViewportReadbackBalancer::default();
        assert!(balancer.should_capture());
        assert!(balancer.should_capture());
    }

    #[test]
    fn viewport_readback_balancer_slows_down_when_over_budget() {
        let mut balancer = ViewportReadbackBalancer::default();
        balancer.update_from_frame_time(20.0, 16.67);
        assert_eq!(balancer.interval_frames(), 2);
    }

    #[test]
    fn viewport_readback_balancer_speeds_up_when_under_budget() {
        let mut balancer = ViewportReadbackBalancer {
            interval_frames: 4,
            frame_cursor: 0,
        };
        balancer.update_from_frame_time(10.0, 16.67);
        assert_eq!(balancer.interval_frames(), 3);
    }

    #[test]
    fn collider_half_extents_for_circle_uses_radius() {
        let collider = Collider2D {
            shape: "circle".into(),
            radius: 10.0,
            width: 0.0,
            height: 0.0,
            is_sensor: false,
        };
        assert_eq!(collider_half_extents(&collider), (10.0, 10.0));
    }

    #[test]
    fn collider_half_extents_for_box_uses_width_height() {
        let collider = Collider2D {
            shape: "box".into(),
            radius: 0.0,
            width: 20.0,
            height: 40.0,
            is_sensor: false,
        };
        assert_eq!(collider_half_extents(&collider), (10.0, 20.0));
    }

    #[test]
    fn aabb_overlap_detects_intersection() {
        let a = Aabb {
            x: 0.0,
            y: 0.0,
            half_w: 5.0,
            half_h: 5.0,
        };
        let b = Aabb {
            x: 8.0,
            y: 0.0,
            half_w: 5.0,
            half_h: 5.0,
        };
        assert!(aabb_overlaps(a, b));
    }

    #[test]
    fn aabb_overlap_respects_separation() {
        let a = Aabb {
            x: 0.0,
            y: 0.0,
            half_w: 5.0,
            half_h: 5.0,
        };
        let b = Aabb {
            x: 20.0,
            y: 0.0,
            half_w: 5.0,
            half_h: 5.0,
        };
        assert!(!aabb_overlaps(a, b));
    }

    #[test]
    fn move_object_with_collision_allows_free_move() {
        let mut scene = SceneDocument::new_default();
        scene.objects.push(SceneObject {
            object_id: 10,
            parent: None,
            layer_id: 1,
            name: "player".into(),
            tags: Vec::new(),
            components: SceneComponents {
                transform: Transform2D {
                    x: 0.0,
                    y: 0.0,
                    rotation_radians: 0.0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                },
                collider: Some(Collider2D {
                    shape: "box".into(),
                    radius: 0.0,
                    width: 10.0,
                    height: 10.0,
                    is_sensor: false,
                }),
                ..Default::default()
            },
        });

        assert!(move_object_with_collision(&mut scene, 10, 5.0, 0.0));
        assert_eq!(
            scene
                .objects
                .iter()
                .find(|o| o.object_id == 10)
                .unwrap()
                .components
                .transform
                .x,
            5.0
        );
    }

    #[test]
    fn move_object_with_collision_blocks_when_overlapping() {
        let mut scene = SceneDocument::new_default();
        scene.objects.push(SceneObject {
            object_id: 10,
            parent: None,
            layer_id: 1,
            name: "player".into(),
            tags: Vec::new(),
            components: SceneComponents {
                transform: Transform2D {
                    x: 0.0,
                    y: 0.0,
                    rotation_radians: 0.0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                },
                collider: Some(Collider2D {
                    shape: "box".into(),
                    radius: 0.0,
                    width: 10.0,
                    height: 10.0,
                    is_sensor: false,
                }),
                ..Default::default()
            },
        });
        scene.objects.push(SceneObject {
            object_id: 11,
            parent: None,
            layer_id: 1,
            name: "wall".into(),
            tags: Vec::new(),
            components: SceneComponents {
                transform: Transform2D {
                    x: 15.0,
                    y: 0.0,
                    rotation_radians: 0.0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                },
                collider: Some(Collider2D {
                    shape: "box".into(),
                    radius: 0.0,
                    width: 10.0,
                    height: 10.0,
                    is_sensor: false,
                }),
                ..Default::default()
            },
        });

        assert!(!move_object_with_collision(&mut scene, 10, 15.0, 0.0));
        assert_eq!(
            scene
                .objects
                .iter()
                .find(|o| o.object_id == 10)
                .unwrap()
                .components
                .transform
                .x,
            0.0
        );
    }

    #[test]
    fn build_script_waves_detects_cycle_and_falls_back() {
        let jobs = vec![
            ScriptJobDescriptor {
                node_id: 1,
                node_name: "a".into(),
                script_asset: "a.rhai".into(),
                entry: "update".into(),
                frame_phase: "gameplay".into(),
                dependencies: vec![2],
                settings: BTreeMap::new(),
            },
            ScriptJobDescriptor {
                node_id: 2,
                node_name: "b".into(),
                script_asset: "b.rhai".into(),
                entry: "update".into(),
                frame_phase: "gameplay".into(),
                dependencies: vec![1],
                settings: BTreeMap::new(),
            },
        ];

        let waves = ScriptRuntime::build_script_waves(&jobs);
        assert_eq!(waves.len(), 1);
        assert_eq!(waves[0].len(), 2);
    }

    #[test]
    fn clamp_sprite_dimension_rejects_zero_and_negative() {
        assert_eq!(clamp_sprite_dimension(0), 1);
        assert_eq!(clamp_sprite_dimension(-5), 1);
    }

    #[test]
    fn clamp_sprite_dimension_clamps_to_u32_max() {
        assert_eq!(clamp_sprite_dimension(i64::MAX), u32::MAX);
    }

    fn job_with_settings(settings: BTreeMap<String, String>) -> ScriptJobDescriptor {
        ScriptJobDescriptor {
            node_id: 1,
            node_name: "test".into(),
            script_asset: "test.rhai".into(),
            entry: "update".into(),
            frame_phase: "gameplay".into(),
            dependencies: vec![],
            settings,
        }
    }

    #[test]
    fn access_profile_disjoint_read_write_sets_can_parallelize() {
        let a = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("read_set".into(), "health".into()),
                ("write_set".into(), "player".into()),
            ])),
        );
        let b = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("read_set".into(), "enemies".into()),
                ("write_set".into(), "score".into()),
            ])),
        );
        assert!(!a.conflicts_with(&b));
    }

    #[test]
    fn access_profile_write_write_conflict_blocks_parallelism() {
        let a = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("write_set".into(), "score".into()),
            ])),
        );
        let b = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("write_set".into(), "score".into()),
            ])),
        );
        assert!(a.conflicts_with(&b));
    }

    #[test]
    fn access_profile_read_write_conflict_blocks_parallelism() {
        let a = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("write_set".into(), "health".into()),
            ])),
        );
        let b = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("read_set".into(), "health".into()),
            ])),
        );
        assert!(a.conflicts_with(&b));
    }

    #[test]
    fn access_profile_legacy_parallel_key_still_works() {
        let a = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("script_parallel_key".into(), "group_a".into()),
            ])),
        );
        let b = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("script_parallel_key".into(), "group_b".into()),
            ])),
        );
        assert!(!a.conflicts_with(&b));

        let c = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("script_parallel_key".into(), "group_a".into()),
            ])),
        );
        assert!(a.conflicts_with(&c));
    }

    #[test]
    fn access_profile_unsafe_job_conflicts_with_everything() {
        let safe = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("read_set".into(), "x".into()),
            ])),
        );
        let unsafe_job = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::new()),
        );
        assert!(safe.conflicts_with(&unsafe_job));
        assert!(unsafe_job.conflicts_with(&safe));
    }

    #[test]
    fn access_profile_case_insensitive_keys() {
        let a = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("write_set".into(), "Score".into()),
            ])),
        );
        let b = ScriptRuntime::job_access_profile(
            &job_with_settings(BTreeMap::from([
                ("parallel_safe".into(), "true".into()),
                ("write_set".into(), "score".into()),
            ])),
        );
        assert!(a.conflicts_with(&b));
    }
}
