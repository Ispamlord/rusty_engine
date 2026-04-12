use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui;
use engine_app::{EngineApp, EngineAppError, RuntimeDiagnosticsSnapshot};
use engine_assets::{
    load_scene_document, Camera2DComponent, Collider2D, SceneComponents, SceneLayer, SceneObject,
    Sprite2D, Transform2D,
};
use engine_core::EngineConfig;
use engine_editor::{
    apply_inspector_node_change, node_input_pin_type, node_output_pin_type, node_workspace,
    pin_types_compatible, sync_document_dependencies_from_snarl, workspace_kinds, workspace_label,
    EditorCommand, EditorError, EditorProjectState, EditorWorkspaceMode, GraphCanvasState,
};
use engine_nodes::{
    Node, NodeExecutionTarget, NodeFallbackPolicy, NodeGraph, NodeId, NodeKind, NodePayload,
    ScriptBehaviorPayload, CURRENT_GRAPH_VERSION,
};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct EditorAppConfig {
    pub config_path: Option<PathBuf>,
}

impl Default for EditorAppConfig {
    fn default() -> Self {
        Self {
            config_path: Some(PathBuf::from("config/default.ron")),
        }
    }
}

#[derive(Debug, Error)]
pub enum EditorAppError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Editor(#[from] EditorError),

    #[error(transparent)]
    Asset(#[from] engine_assets::AssetError),

    #[error(transparent)]
    Engine(#[from] EngineAppError),

    #[error("ui runtime error: {0}")]
    Ui(String),
}

pub struct EditorApp {
    project: EditorProjectState,
    runtime: EngineApp,
    canvas: GraphCanvasState,
    status_line: String,
}

impl EditorApp {
    pub fn new(
        project_path: impl AsRef<Path>,
        config: EditorAppConfig,
    ) -> Result<Self, EditorAppError> {
        let project_path = project_path.as_ref().to_path_buf();
        let project = EditorProjectState::open(&project_path, None)?;

        let mut runtime = if let Some(path) = config.config_path.as_ref() {
            if path.exists() {
                EngineApp::from_config_path(path)?
            } else {
                EngineApp::new(EngineConfig::default())?
            }
        } else {
            EngineApp::new(EngineConfig::default())?
        };

        let _ = runtime.set_active_scene(project.document.scene.clone())?;

        let mut canvas = GraphCanvasState::default();
        canvas.rebuild_from_document(&project.document);

        Ok(Self {
            project,
            runtime,
            canvas,
            status_line: "ready".to_string(),
        })
    }

    pub fn run(self) -> Result<(), EditorAppError> {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_title("Rusty Engine Editor"),
            ..Default::default()
        };

        eframe::run_native(
            "Rusty Engine Editor",
            options,
            Box::new(move |_cc| Ok(Box::new(EditorUi::new(self)))),
        )
        .map_err(|err| EditorAppError::Ui(err.to_string()))
    }

    pub fn run_smoke(&mut self, frames: u32) -> Result<(), EditorAppError> {
        self.runtime.run_for_frames(frames)?;
        Ok(())
    }

    pub fn open_scene(&mut self, path: impl AsRef<Path>) -> Result<(), EditorAppError> {
        let scene_path = path.as_ref().to_path_buf();
        let scene = engine_assets::load_scene_document(&scene_path)?;
        self.project.scene_path = scene_path;
        self.project.document = engine_editor::EditorDocument::from_scene(scene.clone());
        self.project.next_node_id = self
            .project
            .document
            .scene
            .graph
            .nodes
            .iter()
            .map(|node| node.id)
            .max()
            .unwrap_or(0)
            + 1;
        self.project.next_layer_id = self
            .project
            .document
            .scene
            .layers
            .iter()
            .map(|layer| layer.layer_id)
            .max()
            .unwrap_or(0)
            + 1;
        self.project.next_object_id = self
            .project
            .document
            .scene
            .objects
            .iter()
            .map(|object| object.object_id)
            .max()
            .unwrap_or(0)
            + 1;
        self.project.dirty = false;
        self.canvas.rebuild_from_document(&self.project.document);

        let compiled = self.runtime.set_active_scene(scene)?;
        self.status_line = if compiled {
            "scene opened and compiled".to_string()
        } else {
            "scene opened; compile failed, previous runtime artifact kept".to_string()
        };
        Ok(())
    }

    pub fn save_scene(&mut self, path: impl AsRef<Path>) -> Result<(), EditorAppError> {
        self.project.save_scene_as(path)?;
        self.status_line = "scene saved".to_string();
        Ok(())
    }

    pub fn open_project(
        &mut self,
        project_path: impl AsRef<Path>,
        scene_override: Option<PathBuf>,
    ) -> Result<(), EditorAppError> {
        let project_path = project_path.as_ref().to_path_buf();
        let project = EditorProjectState::open(&project_path, scene_override)?;
        let scene = project.document.scene.clone();

        self.project = project;
        self.canvas.rebuild_from_document(&self.project.document);

        let compiled = self.runtime.set_active_scene(scene)?;
        self.status_line = if compiled {
            format!("opened project {}", self.project.project_root.display())
        } else {
            format!(
                "opened {}, compile failed; previous runtime artifact kept",
                self.project.project_root.display()
            )
        };

        Ok(())
    }

    pub fn create_project_minimal(
        &mut self,
        project_path: impl AsRef<Path>,
        scene_override: Option<PathBuf>,
    ) -> Result<(), EditorAppError> {
        let project_path = project_path.as_ref();
        fs::create_dir_all(project_path.join("assets"))?;
        self.ensure_default_shape_assets(project_path)?;

        let scene_path = scene_override
            .unwrap_or_else(|| project_path.join("assets").join("sample_scene.scene.ron"));

        if !scene_path.exists() {
            if let Some(parent) = scene_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let scene = engine_assets::SceneDocument::from_graph(Self::default_test_game_graph());
            engine_assets::save_scene_document(&scene_path, &scene)?;
        }

        self.open_project(project_path, Some(scene_path))
    }

    fn save_active_scene(&mut self) -> Result<(), EditorAppError> {
        self.project.save_scene()?;
        self.status_line = "scene saved".to_string();
        Ok(())
    }

    fn ensure_default_shape_assets(&self, project_path: &Path) -> Result<(), EditorAppError> {
        let shapes_dir = project_path.join("assets").join("basic_shapes");
        fs::create_dir_all(&shapes_dir)?;

        let shapes = [
            (
                "square.ron",
                "(name: \"square\", kind: \"shape\", color: \"#8bd3ff\", size: 1.0)",
            ),
            (
                "circle.ron",
                "(name: \"circle\", kind: \"shape\", color: \"#ffb36b\", size: 1.0)",
            ),
            (
                "triangle.ron",
                "(name: \"triangle\", kind: \"shape\", color: \"#9cff8b\", size: 1.0)",
            ),
        ];

        for (file_name, content) in shapes {
            let path = shapes_dir.join(file_name);
            if !path.exists() {
                fs::write(path, content)?;
            }
        }

        Ok(())
    }

    fn default_test_game_graph() -> NodeGraph {
        NodeGraph {
            version: CURRENT_GRAPH_VERSION,
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
                    name: "test_logic".to_string(),
                    kind: NodeKind::GameplayFlow,
                    target: NodeExecutionTarget::Cpu,
                    dependencies: vec![1],
                    settings: BTreeMap::new(),
                    gpu_bindings: vec![],
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![],
                    shader_entry: None,
                    shader_profile: None,
                    payload: Some(NodePayload::GameplayFlow(Default::default())),
                },
                Node {
                    id: 3,
                    name: "test_render".to_string(),
                    kind: NodeKind::RenderPass,
                    target: NodeExecutionTarget::Gpu,
                    dependencies: vec![2],
                    settings: BTreeMap::from([
                        ("sprite_count".to_string(), "12".to_string()),
                        ("blend".to_string(), "alpha".to_string()),
                        ("target_resource".to_string(), "frame_color".to_string()),
                        ("target_width".to_string(), "1280".to_string()),
                        ("target_height".to_string(), "720".to_string()),
                        (
                            "shape_assets".to_string(),
                            "square,circle,triangle,diamond".to_string(),
                        ),
                        ("sprite_spacing".to_string(), "72".to_string()),
                    ]),
                    gpu_bindings: vec![],
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![],
                    shader_entry: None,
                    shader_profile: None,
                    payload: Some(NodePayload::RenderPass(Default::default())),
                },
                Node {
                    id: 4,
                    name: "script_update".to_string(),
                    kind: NodeKind::ScriptBehavior,
                    target: NodeExecutionTarget::Cpu,
                    dependencies: vec![2],
                    settings: BTreeMap::new(),
                    gpu_bindings: vec![],
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![],
                    shader_entry: None,
                    shader_profile: None,
                    payload: Some(NodePayload::ScriptBehavior(ScriptBehaviorPayload::default())),
                },
            ],
        }
    }
}

struct EditorUi {
    app: EditorApp,
    selected_node: Option<NodeId>,
    selected_object: Option<u64>,
    selected_layer: Option<u64>,
    inspector_name_buffer: String,
    inspector_setting_key_buffer: String,
    inspector_setting_value_buffer: String,
    object_name_buffer: String,
    layer_name_buffer: String,
    object_custom_key_buffer: String,
    object_custom_value_buffer: String,
    project_modal_open: bool,
    project_path_input: String,
    scene_path_input: String,
    scene_path_linked: bool,
    dragged_asset: Option<engine_editor::ProjectAssetEntry>,
    node_clipboard: Option<Node>,
    object_clipboard: Option<SceneObject>,
    viewport_texture: Option<egui::TextureHandle>,
    autosave_restore_modal_open: bool,
    dirty_guard_modal_open: bool,
    pending_project_action: Option<PendingProjectAction>,
}

#[derive(Debug, Clone)]
enum PendingProjectAction {
    OpenProject {
        project_path: PathBuf,
        scene_override: Option<PathBuf>,
    },
    CreateProject {
        project_path: PathBuf,
        scene_override: Option<PathBuf>,
    },
    SwitchRecent {
        project_path: PathBuf,
    },
}

impl EditorUi {
    fn new(app: EditorApp) -> Self {
        let project_path_input = app.project.project_root.display().to_string();
        let scene_path_input = app.project.scene_path.display().to_string();
        let autosave_restore_modal_open = app.project.autosave_path().exists();
        Self {
            app,
            selected_node: None,
            selected_object: None,
            selected_layer: None,
            inspector_name_buffer: String::new(),
            inspector_setting_key_buffer: String::new(),
            inspector_setting_value_buffer: String::new(),
            object_name_buffer: String::new(),
            layer_name_buffer: String::new(),
            object_custom_key_buffer: String::new(),
            object_custom_value_buffer: String::new(),
            project_modal_open: true,
            project_path_input,
            scene_path_input,
            scene_path_linked: true,
            dragged_asset: None,
            node_clipboard: None,
            object_clipboard: None,
            viewport_texture: None,
            autosave_restore_modal_open,
            dirty_guard_modal_open: false,
            pending_project_action: None,
        }
    }

    fn default_scene_path_for_project(project_path: &Path) -> PathBuf {
        project_path.join("assets").join("sample_scene.scene.ron")
    }

    fn sync_scene_path_from_project_path(&mut self) {
        let project_path = PathBuf::from(self.project_path_input.trim());
        if project_path.as_os_str().is_empty() {
            return;
        }

        self.scene_path_input = Self::default_scene_path_for_project(&project_path)
            .display()
            .to_string();
    }

    fn set_project_inputs_from_active(&mut self) {
        self.project_path_input = self.app.project.project_root.display().to_string();
        self.scene_path_input = self.app.project.scene_path.display().to_string();
        self.scene_path_linked = true;
    }

    fn after_project_switch(&mut self) {
        self.selected_node = None;
        self.selected_object = None;
        self.selected_layer = None;
        self.app.project.session.selected_node = None;
        self.inspector_name_buffer.clear();
        self.inspector_setting_key_buffer.clear();
        self.inspector_setting_value_buffer.clear();
        self.object_name_buffer.clear();
        self.layer_name_buffer.clear();
        self.object_custom_key_buffer.clear();
        self.object_custom_value_buffer.clear();
        self.dragged_asset = None;
        self.node_clipboard = None;
        self.object_clipboard = None;
        self.viewport_texture = None;
        self.autosave_restore_modal_open = self.app.project.autosave_path().exists();
        self.set_project_inputs_from_active();
    }

    fn apply_project_commands(
        &mut self,
        label: &str,
        commands: Vec<EditorCommand>,
        refresh_graph_runtime: bool,
    ) {
        if commands.is_empty() {
            return;
        }

        let _tx = self.app.project.begin_transaction();
        let apply = self.app.project.apply_command_batch(label, commands);
        self.app.project.end_transaction();

        match apply {
            Ok(()) => {
                if refresh_graph_runtime {
                    self.canvas_refresh_and_compile();
                } else {
                    self.app.status_line = "scene updated".to_string();
                }
            }
            Err(err) => {
                self.app.status_line = format!("command failed: {err}");
            }
        }
    }

    fn parse_optional_scene_path(&self) -> Option<PathBuf> {
        if self.scene_path_linked {
            let project_path = PathBuf::from(self.project_path_input.trim());
            if project_path.as_os_str().is_empty() {
                return None;
            }
            return Some(Self::default_scene_path_for_project(&project_path));
        }

        let trimmed = self.scene_path_input.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    }

    fn open_project_from_inputs(&mut self) {
        let project_raw = self.project_path_input.trim();
        if project_raw.is_empty() {
            self.app.status_line = "project path is required".to_string();
            return;
        }

        let project_path = PathBuf::from(project_raw);
        let scene_override = self.parse_optional_scene_path();
        self.queue_or_run_project_action(PendingProjectAction::OpenProject {
            project_path,
            scene_override,
        });
    }

    fn create_project_from_inputs(&mut self) {
        let project_raw = self.project_path_input.trim();
        if project_raw.is_empty() {
            self.app.status_line = "project path is required".to_string();
            return;
        }

        let project_path = PathBuf::from(project_raw);
        if project_path.is_file() {
            self.app.status_line = "project path points to a file; select a directory".to_string();
            return;
        }
        let scene_override = self.parse_optional_scene_path();
        self.queue_or_run_project_action(PendingProjectAction::CreateProject {
            project_path,
            scene_override,
        });
    }

    fn queue_or_run_project_action(&mut self, action: PendingProjectAction) {
        if self.app.project.dirty {
            self.pending_project_action = Some(action);
            self.dirty_guard_modal_open = true;
            return;
        }

        self.run_project_action(action);
    }

    fn run_project_action(&mut self, action: PendingProjectAction) {
        let result = match action {
            PendingProjectAction::OpenProject {
                project_path,
                scene_override,
            } => self.app.open_project(project_path, scene_override),
            PendingProjectAction::CreateProject {
                project_path,
                scene_override,
            } => self
                .app
                .create_project_minimal(project_path, scene_override),
            PendingProjectAction::SwitchRecent { project_path } => {
                self.app.open_project(project_path, None)
            }
        };

        match result {
            Ok(()) => {
                self.project_modal_open = false;
                self.after_project_switch();
            }
            Err(err) => {
                self.app.status_line = format!("project action failed: {err}");
            }
        }
    }

    fn draw_project_modal(&mut self, ctx: &egui::Context) {
        if !self.project_modal_open {
            return;
        }

        let mut open = self.project_modal_open;
        egui::Window::new("Project Manager")
            .open(&mut open)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label("Choose where to create/open a project before entering the editor.");
                ui.separator();
                ui.label("Project path");
                ui.horizontal(|ui| {
                    let response = ui.text_edit_singleline(&mut self.project_path_input);
                    if response.changed() && self.scene_path_linked {
                        self.sync_scene_path_from_project_path();
                    }
                    if ui.button("Browse Folder...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.project_path_input = path.display().to_string();
                            if self.scene_path_linked {
                                self.sync_scene_path_from_project_path();
                            }
                        }
                    }
                });
                ui.checkbox(&mut self.scene_path_linked, "Link scene path to project");
                ui.label("Scene path (optional)");
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!self.scene_path_linked, |ui| {
                        ui.text_edit_singleline(&mut self.scene_path_input);
                    });
                    if ui.button("Browse Scene...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Scene", &["ron"])
                            .pick_file()
                        {
                            self.scene_path_input = path.display().to_string();
                            self.scene_path_linked = false;
                        }
                    }
                });

                ui.horizontal(|ui| {
                    if ui.button("Create Minimal Project").clicked() {
                        self.create_project_from_inputs();
                    }
                    if ui.button("Open Project").clicked() {
                        self.open_project_from_inputs();
                    }
                    if ui.button("Use Current Project").clicked() {
                        self.project_modal_open = false;
                    }
                    if ui.button("Close").clicked() {
                        self.project_modal_open = false;
                    }
                });

                ui.separator();
                ui.label("Recent projects");
                let recents = self.app.project.session.recent_projects.clone();
                egui::ScrollArea::vertical()
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for recent in recents {
                            let label = recent.display().to_string();
                            if ui.button(label).clicked() {
                                self.queue_or_run_project_action(
                                    PendingProjectAction::SwitchRecent {
                                        project_path: recent.clone(),
                                    },
                                );
                            }
                        }
                    });
            });

        self.project_modal_open = open;
    }

    fn draw_dirty_guard_modal(&mut self, ctx: &egui::Context) {
        if !self.dirty_guard_modal_open {
            return;
        }

        let mut open = self.dirty_guard_modal_open;
        egui::Window::new("Unsaved Changes")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label("Current scene has unsaved changes.");
                ui.label("Save before switching project?");
                ui.horizontal(|ui| {
                    if ui.button("Save and Continue").clicked() {
                        match self.app.save_active_scene() {
                            Ok(()) => {
                                if let Some(action) = self.pending_project_action.take() {
                                    self.run_project_action(action);
                                }
                                self.dirty_guard_modal_open = false;
                            }
                            Err(err) => {
                                self.app.status_line = format!("save failed: {err}");
                            }
                        }
                    }
                    if ui.button("Discard and Continue").clicked() {
                        if let Some(action) = self.pending_project_action.take() {
                            self.run_project_action(action);
                        }
                        self.dirty_guard_modal_open = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_project_action = None;
                        self.dirty_guard_modal_open = false;
                    }
                });
            });
        self.dirty_guard_modal_open = open;
    }

    fn draw_autosave_restore_modal(&mut self, ctx: &egui::Context) {
        if !self.autosave_restore_modal_open {
            return;
        }

        let autosave_path = self.app.project.autosave_path();
        if !autosave_path.exists() {
            self.autosave_restore_modal_open = false;
            return;
        }

        let mut open = self.autosave_restore_modal_open;
        egui::Window::new("Autosave Recovery")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!(
                    "Recovered autosave found: {}",
                    autosave_path.display()
                ));
                ui.horizontal(|ui| {
                    if ui.button("Restore Autosave").clicked() {
                        match load_scene_document(&autosave_path) {
                            Ok(scene) => {
                                self.app.project.document =
                                    engine_editor::EditorDocument::from_scene(scene.clone());
                                self.app.project.next_node_id = self
                                    .app
                                    .project
                                    .document
                                    .scene
                                    .graph
                                    .nodes
                                    .iter()
                                    .map(|node| node.id)
                                    .max()
                                    .unwrap_or(0)
                                    + 1;
                                self.app.project.next_layer_id = self
                                    .app
                                    .project
                                    .document
                                    .scene
                                    .layers
                                    .iter()
                                    .map(|layer| layer.layer_id)
                                    .max()
                                    .unwrap_or(0)
                                    + 1;
                                self.app.project.next_object_id = self
                                    .app
                                    .project
                                    .document
                                    .scene
                                    .objects
                                    .iter()
                                    .map(|object| object.object_id)
                                    .max()
                                    .unwrap_or(0)
                                    + 1;
                                self.app.project.dirty = true;
                                self.app
                                    .canvas
                                    .rebuild_from_document(&self.app.project.document);
                                let _ = self.app.runtime.set_active_scene(scene);
                                self.app.status_line = "autosave restored".to_string();
                                self.autosave_restore_modal_open = false;
                            }
                            Err(err) => {
                                self.app.status_line = format!("autosave restore failed: {err}");
                            }
                        }
                    }
                    if ui.button("Discard Autosave").clicked() {
                        let _ = std::fs::remove_file(&autosave_path);
                        self.autosave_restore_modal_open = false;
                    }
                });
            });

        self.autosave_restore_modal_open = open;
    }

    fn draw_top_menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("editor_top_menu").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Save").clicked() {
                        if let Err(err) = self.app.save_active_scene() {
                            self.app.status_line = format!("save failed: {err}");
                        }
                        ui.close();
                    }

                    if ui.button("Save As autosave_scene.scene.ron").clicked() {
                        let path = self
                            .app
                            .project
                            .project_root
                            .join("assets")
                            .join("autosave_scene.scene.ron");
                        if let Err(err) = self.app.save_scene(path) {
                            self.app.status_line = format!("save-as failed: {err}");
                        }
                        ui.close();
                    }

                    ui.separator();
                    if ui.button("New Project...").clicked() {
                        self.project_modal_open = true;
                        self.set_project_inputs_from_active();
                        ui.close();
                    }
                    if ui.button("Open Project...").clicked() {
                        self.project_modal_open = true;
                        self.set_project_inputs_from_active();
                        ui.close();
                    }

                    ui.separator();
                    let recents = self.app.project.session.recent_projects.clone();
                    for recent in recents {
                        let label = format!("Switch: {}", recent.display());
                        if ui.button(label).clicked() {
                            self.queue_or_run_project_action(PendingProjectAction::SwitchRecent {
                                project_path: recent.clone(),
                            });
                            ui.close();
                        }
                    }
                });

                ui.menu_button("Tools", |ui| {
                    if ui.button("Project Manager").clicked() {
                        self.project_modal_open = true;
                        self.set_project_inputs_from_active();
                        ui.close();
                    }

                    if ui.button("Refresh Assets").clicked() {
                        if let Err(err) = self.app.project.refresh_asset_index() {
                            self.app.status_line = format!("asset refresh failed: {err}");
                        }
                        ui.close();
                    }

                    if ui.button("Hot Recompile").clicked() {
                        match self
                            .app
                            .runtime
                            .set_active_scene(self.app.project.document.scene.clone())
                        {
                            Ok(true) => self.app.status_line = "runtime updated".to_string(),
                            Ok(false) => {
                                self.app.status_line =
                                    "compile failed; previous runtime artifact kept".to_string()
                            }
                            Err(err) => self.app.status_line = format!("compile failed: {err}"),
                        }
                        ui.close();
                    }
                });

                ui.separator();
                let current_workspace = self.app.project.session.workspace_mode;
                ui.horizontal(|ui| {
                    ui.label("Workspace");
                    if ui
                        .selectable_label(
                            matches!(current_workspace, EditorWorkspaceMode::Gameplay),
                            workspace_label(EditorWorkspaceMode::Gameplay),
                        )
                        .clicked()
                    {
                        self.app.project.session.workspace_mode = EditorWorkspaceMode::Gameplay;
                        self.selected_node = None;
                        self.inspector_name_buffer.clear();
                        self.inspector_setting_key_buffer.clear();
                        self.inspector_setting_value_buffer.clear();
                        self.app.status_line =
                            "workspace switched to Gameplay / Script".to_string();
                    }
                    if ui
                        .selectable_label(
                            matches!(current_workspace, EditorWorkspaceMode::Render),
                            workspace_label(EditorWorkspaceMode::Render),
                        )
                        .clicked()
                    {
                        self.app.project.session.workspace_mode = EditorWorkspaceMode::Render;
                        self.selected_node = None;
                        self.inspector_name_buffer.clear();
                        self.inspector_setting_key_buffer.clear();
                        self.inspector_setting_value_buffer.clear();
                        self.app.status_line = "workspace switched to Render Pipeline".to_string();
                    }
                });

                if ui.button("Refresh Assets").clicked() {
                    if let Err(err) = self.app.project.refresh_asset_index() {
                        self.app.status_line = format!("asset refresh failed: {err}");
                    }
                }
                ui.separator();
                if ui.button("Undo").clicked() {
                    match self.app.project.undo() {
                        Ok(true) => self.canvas_refresh_and_compile(),
                        Ok(false) => self.app.status_line = "nothing to undo".to_string(),
                        Err(err) => self.app.status_line = format!("undo failed: {err}"),
                    }
                }
                if ui.button("Redo").clicked() {
                    match self.app.project.redo() {
                        Ok(true) => self.canvas_refresh_and_compile(),
                        Ok(false) => self.app.status_line = "nothing to redo".to_string(),
                        Err(err) => self.app.status_line = format!("redo failed: {err}"),
                    }
                }
                ui.separator();
                if ui.button("Play").clicked() {
                    self.app.runtime.start_play_mode();
                }
                if ui.button("Stop").clicked() {
                    self.app.runtime.stop_play_mode();
                }
                if ui.button("Restart").clicked() {
                    self.app.runtime.stop_play_mode();
                    self.app.runtime.start_play_mode();
                }
                if ui.button("Step").clicked() {
                    if let Err(err) = self.app.runtime.step_play_frame() {
                        self.app.status_line = format!("step failed: {err}");
                    }
                }
                if ui.button("Hot Recompile").clicked() {
                    match self
                        .app
                        .runtime
                        .set_active_scene(self.app.project.document.scene.clone())
                    {
                        Ok(true) => self.app.status_line = "runtime updated".to_string(),
                        Ok(false) => {
                            self.app.status_line =
                                "compile failed; previous runtime artifact kept".to_string()
                        }
                        Err(err) => self.app.status_line = format!("compile failed: {err}"),
                    }
                }
                ui.separator();
                ui.label(format!("Status: {}", self.app.status_line));
            });
        });
    }

    fn draw_assets_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("assets_panel")
            .resizable(true)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.heading("Scene");
                ui.label(format!(
                    "Project: {}",
                    self.app.project.project_root.display()
                ));
                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Add Layer").clicked() {
                        let order = self
                            .app
                            .project
                            .document
                            .scene
                            .layers
                            .iter()
                            .map(|layer| layer.order)
                            .max()
                            .unwrap_or(0)
                            + 1;
                        let layer = SceneLayer {
                            layer_id: self.app.project.allocate_layer_id(),
                            name: format!("Layer {order}"),
                            order,
                            visible: true,
                            locked: false,
                        };
                        self.apply_project_commands(
                            "add_layer",
                            vec![EditorCommand::AddLayer { layer }],
                            false,
                        );
                    }

                    if ui.button("Add Object").clicked() {
                        let object = SceneObject {
                            object_id: self.app.project.allocate_object_id(),
                            parent: None,
                            layer_id: self.app.project.default_layer_id(),
                            name: "Object".to_string(),
                            tags: Vec::new(),
                            components: SceneComponents {
                                transform: Transform2D::default(),
                                ..SceneComponents::default()
                            },
                        };
                        self.apply_project_commands(
                            "add_object",
                            vec![EditorCommand::AddObject {
                                object: object.clone(),
                            }],
                            false,
                        );
                        self.selected_object = Some(object.object_id);
                        self.object_name_buffer = object.name;
                    }
                });

                egui::CollapsingHeader::new("Layers")
                    .default_open(true)
                    .show(ui, |ui| {
                        let layers = self.app.project.document.scene.layers.clone();
                        for layer in layers {
                            let selected = self.selected_layer == Some(layer.layer_id);
                            ui.horizontal(|ui| {
                                if ui.selectable_label(selected, &layer.name).clicked() {
                                    self.selected_layer = Some(layer.layer_id);
                                    self.layer_name_buffer = layer.name.clone();
                                }
                                if ui
                                    .small_button(if layer.visible { "Hide" } else { "Show" })
                                    .clicked()
                                {
                                    self.apply_project_commands(
                                        "layer_visibility",
                                        vec![EditorCommand::SetLayerProps {
                                            layer_id: layer.layer_id,
                                            old_name: layer.name.clone(),
                                            new_name: layer.name.clone(),
                                            old_order: layer.order,
                                            new_order: layer.order,
                                            old_visible: layer.visible,
                                            new_visible: !layer.visible,
                                            old_locked: layer.locked,
                                            new_locked: layer.locked,
                                        }],
                                        false,
                                    );
                                }
                                if ui
                                    .small_button(if layer.locked { "Unlock" } else { "Lock" })
                                    .clicked()
                                {
                                    self.apply_project_commands(
                                        "layer_lock",
                                        vec![EditorCommand::SetLayerProps {
                                            layer_id: layer.layer_id,
                                            old_name: layer.name.clone(),
                                            new_name: layer.name.clone(),
                                            old_order: layer.order,
                                            new_order: layer.order,
                                            old_visible: layer.visible,
                                            new_visible: layer.visible,
                                            old_locked: layer.locked,
                                            new_locked: !layer.locked,
                                        }],
                                        false,
                                    );
                                }
                            });
                        }

                        if let Some(layer_id) = self.selected_layer {
                            if let Some(layer) = self
                                .app
                                .project
                                .document
                                .scene
                                .layers
                                .iter()
                                .find(|layer| layer.layer_id == layer_id)
                                .cloned()
                            {
                                ui.separator();
                                ui.label("Rename Layer");
                                ui.text_edit_singleline(&mut self.layer_name_buffer);
                                if ui.button("Apply Layer Name").clicked() {
                                    self.apply_project_commands(
                                        "rename_layer",
                                        vec![EditorCommand::SetLayerProps {
                                            layer_id,
                                            old_name: layer.name,
                                            new_name: self.layer_name_buffer.clone(),
                                            old_order: layer.order,
                                            new_order: layer.order,
                                            old_visible: layer.visible,
                                            new_visible: layer.visible,
                                            old_locked: layer.locked,
                                            new_locked: layer.locked,
                                        }],
                                        false,
                                    );
                                }
                            }
                        }
                    });

                egui::CollapsingHeader::new("Hierarchy")
                    .default_open(true)
                    .show(ui, |ui| {
                        let objects = self.app.project.document.scene.objects.clone();
                        for object in &objects {
                            let selected = self.selected_object == Some(object.object_id);
                            let layer_name = self
                                .app
                                .project
                                .document
                                .scene
                                .layers
                                .iter()
                                .find(|layer| layer.layer_id == object.layer_id)
                                .map(|layer| layer.name.clone())
                                .unwrap_or_else(|| "MissingLayer".to_string());
                            let label = if let Some(parent) = object.parent {
                                format!("{}  [L:{} P:{}]", object.name, layer_name, parent)
                            } else {
                                format!("{}  [L:{}]", object.name, layer_name)
                            };
                            if ui.selectable_label(selected, label).clicked() {
                                self.selected_object = Some(object.object_id);
                                self.object_name_buffer = object.name.clone();
                            }
                        }

                        if let Some(selected_object) = self.selected_object {
                            ui.horizontal(|ui| {
                                if ui.button("Duplicate").clicked() {
                                    if let Some(source) = self
                                        .app
                                        .project
                                        .document
                                        .scene
                                        .objects
                                        .iter()
                                        .find(|object| object.object_id == selected_object)
                                        .cloned()
                                    {
                                        let mut duplicated = source.clone();
                                        duplicated.object_id =
                                            self.app.project.allocate_object_id();
                                        duplicated.name = format!("{} Copy", source.name);
                                        self.apply_project_commands(
                                            "duplicate_object",
                                            vec![EditorCommand::AddObject {
                                                object: duplicated.clone(),
                                            }],
                                            false,
                                        );
                                        self.selected_object = Some(duplicated.object_id);
                                    }
                                }

                                if ui.button("Delete").clicked() {
                                    if let Some(object) = self
                                        .app
                                        .project
                                        .document
                                        .scene
                                        .objects
                                        .iter()
                                        .find(|object| object.object_id == selected_object)
                                        .cloned()
                                    {
                                        self.apply_project_commands(
                                            "remove_object",
                                            vec![EditorCommand::RemoveObject { object }],
                                            false,
                                        );
                                        self.selected_object = None;
                                    }
                                }
                            });
                        }
                    });

                ui.separator();
                ui.heading("Assets");
                ui.label("Browse assets by type, inspect, or drag into graph.");
                let selected_asset_path = self.app.project.session.selected_asset.clone();
                let asset_index = self.app.project.asset_index.clone();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.draw_asset_kind_section(
                        ui,
                        &asset_index,
                        &selected_asset_path,
                        engine_editor::EditorAssetKind::Shape,
                        "Shapes",
                    );
                    self.draw_asset_kind_section(
                        ui,
                        &asset_index,
                        &selected_asset_path,
                        engine_editor::EditorAssetKind::Graph,
                        "Scenes / Graphs",
                    );
                    self.draw_asset_kind_section(
                        ui,
                        &asset_index,
                        &selected_asset_path,
                        engine_editor::EditorAssetKind::Texture,
                        "Textures",
                    );
                    self.draw_asset_kind_section(
                        ui,
                        &asset_index,
                        &selected_asset_path,
                        engine_editor::EditorAssetKind::Audio,
                        "Audio",
                    );
                    self.draw_asset_kind_section(
                        ui,
                        &asset_index,
                        &selected_asset_path,
                        engine_editor::EditorAssetKind::Shader,
                        "Shaders",
                    );
                    self.draw_asset_kind_section(
                        ui,
                        &asset_index,
                        &selected_asset_path,
                        engine_editor::EditorAssetKind::Unknown,
                        "Other",
                    );
                });

                ui.separator();
                let selected_asset = self.app.project.session.selected_asset.clone();

                if let Some(selected_asset) = selected_asset {
                    if let Some(entry) = asset_index
                        .iter()
                        .find(|entry| entry.path == selected_asset)
                        .cloned()
                    {
                        ui.heading("Asset Preview");
                        ui.label(format!("Kind: {:?}", entry.kind));
                        ui.label(format!("Path: {}", entry.path.display()));

                        self.draw_asset_preview(ui, &entry);

                        if matches!(entry.kind, engine_editor::EditorAssetKind::Graph)
                            && ui.button("Load Scene Asset").clicked()
                        {
                            let scene_path = entry.path.clone();
                            match self.app.open_scene(&scene_path) {
                                Ok(()) => {
                                    self.app
                                        .canvas
                                        .rebuild_from_document(&self.app.project.document);
                                    self.selected_node = None;
                                    self.inspector_name_buffer.clear();
                                    self.inspector_setting_key_buffer.clear();
                                    self.inspector_setting_value_buffer.clear();
                                    self.app.status_line =
                                        format!("loaded scene {}", scene_path.display());
                                }
                                Err(err) => {
                                    self.app.status_line = format!("load scene failed: {err}");
                                }
                            }
                        }
                    }
                }
            });
    }

    fn draw_asset_kind_section(
        &mut self,
        ui: &mut egui::Ui,
        asset_index: &[engine_editor::ProjectAssetEntry],
        selected_asset_path: &Option<PathBuf>,
        kind: engine_editor::EditorAssetKind,
        title: &str,
    ) {
        let items: Vec<_> = asset_index
            .iter()
            .filter(|entry| entry.kind == kind)
            .cloned()
            .collect();

        if items.is_empty() {
            return;
        }

        let mut header = egui::CollapsingHeader::new(title);
        if matches!(kind, engine_editor::EditorAssetKind::Graph) {
            header = header.default_open(true);
        }

        header.show(ui, |ui| {
            for entry in &items {
                let is_selected = selected_asset_path
                    .as_ref()
                    .map(|selected| selected == &entry.path)
                    .unwrap_or(false);

                let file_name = entry
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("asset");
                let is_active_scene = entry.path == self.app.project.scene_path;
                let label = if matches!(kind, engine_editor::EditorAssetKind::Graph) {
                    let prefix = if file_name.to_ascii_lowercase().contains("scene") {
                        "Scene"
                    } else {
                        "Graph"
                    };

                    if is_active_scene {
                        format!("[active] {prefix}: {file_name}")
                    } else {
                        format!("{prefix}: {file_name}")
                    }
                } else {
                    file_name.to_string()
                };

                let response = ui.selectable_label(is_selected, label);
                if response.clicked() {
                    self.app.project.session.selected_asset = Some(entry.path.clone());
                }

                if response.drag_started() {
                    self.dragged_asset = Some(entry.clone());
                    self.app.project.session.selected_asset = Some(entry.path.clone());
                }

                ui.label(entry.path.display().to_string());
            }
        });
    }

    fn draw_inspector_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("inspector_panel")
            .resizable(true)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.heading("Inspector");
                ui.separator();
                self.draw_object_inspector(ui);
                ui.separator();
                ui.heading("Node Inspector");

                let Some(selected) = self.selected_node else {
                    ui.label("No node selected.");
                    return;
                };

                let maybe_node = self
                    .app
                    .project
                    .document
                    .scene
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == selected)
                    .cloned();

                let Some(node) = maybe_node else {
                    ui.label("Selected node no longer exists.");
                    return;
                };

                let active_workspace = self.app.project.session.workspace_mode;
                let node_workspace = node_workspace(node.kind);
                if node_workspace != active_workspace {
                    ui.label(format!(
                        "Node belongs to {} workspace.",
                        workspace_label(node_workspace)
                    ));
                    ui.label(format!(
                        "Switch to {} to edit this node.",
                        workspace_label(node_workspace)
                    ));
                    return;
                }

                ui.label(format!("Node #{}", node.id));
                if self.inspector_name_buffer.is_empty() {
                    self.inspector_name_buffer = node.name.clone();
                }

                ui.label("Name");
                ui.text_edit_singleline(&mut self.inspector_name_buffer);

                let mut kind = node.kind;
                egui::ComboBox::from_label("Kind")
                    .selected_text(format!("{:?}", kind))
                    .show_ui(ui, |ui| {
                        for option in workspace_kinds(self.app.project.session.workspace_mode) {
                            ui.selectable_value(&mut kind, *option, format!("{:?}", option));
                        }
                    });

                let mut target = node.target;
                egui::ComboBox::from_label("Target")
                    .selected_text(format!("{:?}", target))
                    .show_ui(ui, |ui| {
                        for option in [
                            NodeExecutionTarget::Cpu,
                            NodeExecutionTarget::Gpu,
                            NodeExecutionTarget::Hybrid,
                        ] {
                            ui.selectable_value(&mut target, option, format!("{:?}", option));
                        }
                    });

                let mut fallback = node.fallback_policy;
                egui::ComboBox::from_label("Fallback")
                    .selected_text(format!("{:?}", fallback))
                    .show_ui(ui, |ui| {
                        for option in [
                            NodeFallbackPolicy::Error,
                            NodeFallbackPolicy::Cpu,
                            NodeFallbackPolicy::Disable,
                        ] {
                            ui.selectable_value(&mut fallback, option, format!("{:?}", option));
                        }
                    });

                let mut shader_entry = node.shader_entry.clone().unwrap_or_default();
                let mut shader_profile = node.shader_profile.clone().unwrap_or_default();
                ui.label("Shader Entry");
                ui.text_edit_singleline(&mut shader_entry);
                ui.label("Shader Profile");
                ui.text_edit_singleline(&mut shader_profile);

                let script_asset = node
                    .settings
                    .get("script_asset")
                    .cloned()
                    .unwrap_or_else(|| "assets/scripts/player_controller.rhai".to_string());
                let script_entry = node
                    .settings
                    .get("script_entry")
                    .cloned()
                    .unwrap_or_else(|| "update".to_string());
                let script_phase = node
                    .settings
                    .get("script_phase")
                    .cloned()
                    .unwrap_or_else(|| "gameplay".to_string());
                let mut script_asset_buffer = script_asset;
                let mut script_entry_buffer = script_entry;
                let mut script_phase_buffer = script_phase;
                if node.kind == NodeKind::ScriptBehavior {
                    ui.separator();
                    ui.heading("Script Node");
                    ui.label("Script Asset");
                    ui.text_edit_singleline(&mut script_asset_buffer);
                    ui.label("Script Entry");
                    ui.text_edit_singleline(&mut script_entry_buffer);
                    ui.label("Frame Phase");
                    ui.text_edit_singleline(&mut script_phase_buffer);
                }

                if ui.button("Apply Inspector Changes").clicked() {
                    let script_changes = if node.kind == NodeKind::ScriptBehavior {
                        vec![
                            ("script_asset".to_string(), Some(script_asset_buffer)),
                            ("script_entry".to_string(), Some(script_entry_buffer)),
                            ("script_phase".to_string(), Some(script_phase_buffer)),
                        ]
                    } else {
                        Vec::new()
                    };
                    if let Err(err) = apply_inspector_node_change(
                        &mut self.app.project,
                        selected,
                        Some(kind),
                        Some(target),
                        Some(fallback),
                        Some(if shader_entry.trim().is_empty() {
                            None
                        } else {
                            Some(shader_entry.clone())
                        }),
                        Some(if shader_profile.trim().is_empty() {
                            None
                        } else {
                            Some(shader_profile.clone())
                        }),
                        None,
                    ) {
                        self.app.status_line = format!("inspector apply failed: {err}");
                    } else if let Some(node_mut) = self
                        .app
                        .project
                        .document
                        .scene
                        .graph
                        .nodes
                        .iter_mut()
                        .find(|node| node.id == selected)
                    {
                        node_mut.name = self.inspector_name_buffer.clone();
                        self.app.project.dirty = true;
                    }

                    for (key, value) in script_changes {
                        if let Err(err) = apply_inspector_node_change(
                            &mut self.app.project,
                            selected,
                            None,
                            None,
                            None,
                            None,
                            None,
                            Some((key, value)),
                        ) {
                            self.app.status_line = format!("script metadata update failed: {err}");
                        }
                    }

                    self.canvas_refresh_and_compile();
                }

                ui.separator();
                ui.heading("Node Settings");
                ui.label("Author gameplay/render values as key-value settings.");

                let mut pending_delete: Option<String> = None;
                for (key, value) in &node.settings {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} = {}", key, value));
                        if ui.small_button("Edit").clicked() {
                            self.inspector_setting_key_buffer = key.clone();
                            self.inspector_setting_value_buffer = value.clone();
                        }
                        if ui.small_button("Delete").clicked() {
                            pending_delete = Some(key.clone());
                        }
                    });
                }

                if node.settings.is_empty() {
                    ui.label("No settings yet.");
                }

                if let Some(key) = pending_delete {
                    if let Err(err) = apply_inspector_node_change(
                        &mut self.app.project,
                        selected,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some((key.clone(), None)),
                    ) {
                        self.app.status_line = format!("delete setting failed: {err}");
                    } else {
                        self.app.status_line = format!("removed setting '{key}'");
                        if self.inspector_setting_key_buffer == key {
                            self.inspector_setting_key_buffer.clear();
                            self.inspector_setting_value_buffer.clear();
                        }
                        self.canvas_refresh_and_compile();
                    }
                }

                ui.separator();
                ui.label("Setting Key");
                ui.text_edit_singleline(&mut self.inspector_setting_key_buffer);
                ui.label("Setting Value");
                ui.text_edit_singleline(&mut self.inspector_setting_value_buffer);

                if ui.button("Add / Update Setting").clicked() {
                    let key = self.inspector_setting_key_buffer.trim().to_string();
                    let value = self.inspector_setting_value_buffer.trim().to_string();
                    if key.is_empty() {
                        self.app.status_line = "setting key is required".to_string();
                    } else if let Err(err) = apply_inspector_node_change(
                        &mut self.app.project,
                        selected,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some((key.clone(), Some(value.clone()))),
                    ) {
                        self.app.status_line = format!("update setting failed: {err}");
                    } else {
                        self.app.status_line = format!("updated setting '{key}'");
                        self.canvas_refresh_and_compile();
                    }
                }

                ui.separator();
                let out_pin = node_output_pin_type(node.kind);
                let in_pin = node_input_pin_type(node.kind);
                ui.label(format!("Output pin type: {:?}", out_pin));
                ui.label(format!("Input pin type: {:?}", in_pin));
                ui.label(format!(
                    "Self-connect allowed: {}",
                    pin_types_compatible(out_pin, in_pin)
                ));
            });
    }

    fn draw_object_inspector(&mut self, ui: &mut egui::Ui) {
        ui.heading("Object Inspector");
        let Some(object_id) = self.selected_object else {
            ui.label("No object selected.");
            return;
        };

        let Some(object) = self
            .app
            .project
            .document
            .scene
            .objects
            .iter()
            .find(|object| object.object_id == object_id)
            .cloned()
        else {
            ui.label("Selected object no longer exists.");
            return;
        };

        if self.object_name_buffer.is_empty() {
            self.object_name_buffer = object.name.clone();
        }

        ui.label(format!("Object #{}", object.object_id));
        ui.label("Name");
        ui.text_edit_singleline(&mut self.object_name_buffer);

        let mut selected_layer = object.layer_id;
        egui::ComboBox::from_label("Layer")
            .selected_text(
                self.app
                    .project
                    .document
                    .scene
                    .layers
                    .iter()
                    .find(|layer| layer.layer_id == selected_layer)
                    .map(|layer| layer.name.clone())
                    .unwrap_or_else(|| "Missing".to_string()),
            )
            .show_ui(ui, |ui| {
                for layer in &self.app.project.document.scene.layers {
                    ui.selectable_value(&mut selected_layer, layer.layer_id, &layer.name);
                }
            });

        let mut selected_parent = object.parent;
        egui::ComboBox::from_label("Parent")
            .selected_text(
                selected_parent
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "None".to_string()),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected_parent, None, "None");
                for other in &self.app.project.document.scene.objects {
                    if other.object_id != object.object_id {
                        ui.selectable_value(
                            &mut selected_parent,
                            Some(other.object_id),
                            format!("{} ({})", other.name, other.object_id),
                        );
                    }
                }
            });

        let mut transform = object.components.transform.clone();
        ui.collapsing("Transform", |ui| {
            ui.add(egui::DragValue::new(&mut transform.x).prefix("x: "));
            ui.add(egui::DragValue::new(&mut transform.y).prefix("y: "));
            ui.add(egui::DragValue::new(&mut transform.rotation_radians).prefix("rot: "));
            ui.add(egui::DragValue::new(&mut transform.scale_x).prefix("sx: "));
            ui.add(egui::DragValue::new(&mut transform.scale_y).prefix("sy: "));
        });

        let mut sprite = object.components.sprite.clone();
        let mut sprite_enabled = sprite.is_some();
        ui.checkbox(&mut sprite_enabled, "Sprite Component");
        if sprite_enabled && sprite.is_none() {
            sprite = Some(Sprite2D {
                texture_asset: "assets/texture.png".to_string(),
                width: 64,
                height: 64,
                tint_rgba: [255, 255, 255, 255],
                layer_order: 0,
            });
        }
        if !sprite_enabled {
            sprite = None;
        }
        if let Some(sprite_mut) = sprite.as_mut() {
            ui.collapsing("Sprite", |ui| {
                ui.text_edit_singleline(&mut sprite_mut.texture_asset);
                ui.add(egui::DragValue::new(&mut sprite_mut.width).prefix("w: "));
                ui.add(egui::DragValue::new(&mut sprite_mut.height).prefix("h: "));
                ui.add(egui::DragValue::new(&mut sprite_mut.layer_order).prefix("order: "));
            });
        }

        let mut collider = object.components.collider.clone();
        let mut collider_enabled = collider.is_some();
        ui.checkbox(&mut collider_enabled, "Collider Component");
        if collider_enabled && collider.is_none() {
            collider = Some(Collider2D {
                shape: "circle".to_string(),
                radius: 20.0,
                width: 40.0,
                height: 40.0,
                is_sensor: false,
            });
        }
        if !collider_enabled {
            collider = None;
        }
        if let Some(collider_mut) = collider.as_mut() {
            ui.collapsing("Collider", |ui| {
                ui.text_edit_singleline(&mut collider_mut.shape);
                ui.add(egui::DragValue::new(&mut collider_mut.radius).prefix("r: "));
                ui.add(egui::DragValue::new(&mut collider_mut.width).prefix("w: "));
                ui.add(egui::DragValue::new(&mut collider_mut.height).prefix("h: "));
                ui.checkbox(&mut collider_mut.is_sensor, "Sensor");
            });
        }

        let mut camera = object.components.camera.clone();
        let mut camera_enabled = camera.is_some();
        ui.checkbox(&mut camera_enabled, "Camera Component");
        if camera_enabled && camera.is_none() {
            camera = Some(Camera2DComponent {
                zoom: 1.0,
                near: -1000.0,
                far: 1000.0,
                clear_color_rgba: [16, 20, 28, 255],
            });
        }
        if !camera_enabled {
            camera = None;
        }
        if let Some(camera_mut) = camera.as_mut() {
            ui.collapsing("Camera", |ui| {
                ui.add(egui::DragValue::new(&mut camera_mut.zoom).prefix("zoom: "));
                ui.add(egui::DragValue::new(&mut camera_mut.near).prefix("near: "));
                ui.add(egui::DragValue::new(&mut camera_mut.far).prefix("far: "));
            });
        }

        ui.collapsing("Custom Properties", |ui| {
            for (key, value) in &object.components.custom_properties {
                ui.label(format!("{key} = {value}"));
            }
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.object_custom_key_buffer);
                ui.text_edit_singleline(&mut self.object_custom_value_buffer);
            });
        });

        if ui.button("Apply Object Changes").clicked() {
            let mut commands = Vec::new();
            if self.object_name_buffer != object.name {
                commands.push(EditorCommand::SetObjectName {
                    object_id,
                    old_name: object.name.clone(),
                    new_name: self.object_name_buffer.clone(),
                });
            }
            if selected_layer != object.layer_id {
                commands.push(EditorCommand::MoveObjectToLayer {
                    object_id,
                    old_layer: object.layer_id,
                    new_layer: selected_layer,
                });
            }
            if selected_parent != object.parent {
                commands.push(EditorCommand::ReparentObject {
                    object_id,
                    old_parent: object.parent,
                    new_parent: selected_parent,
                });
            }
            if transform != object.components.transform {
                commands.push(EditorCommand::SetObjectTransform {
                    object_id,
                    old: object.components.transform.clone(),
                    new: transform,
                });
            }
            if sprite != object.components.sprite {
                commands.push(EditorCommand::SetObjectSprite {
                    object_id,
                    old: object.components.sprite.clone(),
                    new: sprite,
                });
            }
            if collider != object.components.collider {
                commands.push(EditorCommand::SetObjectCollider {
                    object_id,
                    old: object.components.collider.clone(),
                    new: collider,
                });
            }
            if camera != object.components.camera {
                commands.push(EditorCommand::SetObjectCamera {
                    object_id,
                    old: object.components.camera.clone(),
                    new: camera,
                });
            }

            let custom_key = self.object_custom_key_buffer.trim().to_string();
            let custom_value = self.object_custom_value_buffer.trim().to_string();
            if !custom_key.is_empty() {
                let old = object
                    .components
                    .custom_properties
                    .get(&custom_key)
                    .cloned();
                let new = if custom_value.is_empty() {
                    None
                } else {
                    Some(custom_value)
                };
                if old != new {
                    commands.push(EditorCommand::SetObjectCustom {
                        object_id,
                        key: custom_key.clone(),
                        old,
                        new,
                    });
                }
                self.object_custom_key_buffer.clear();
                self.object_custom_value_buffer.clear();
            }

            self.apply_project_commands("object_inspector", commands, false);
        }
    }

    fn draw_graph_and_viewport(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |columns| {
                columns[0].heading("Graph Canvas");
                columns[0].label("Add nodes: right-click graph canvas");
                let output = self
                    .app
                    .canvas
                    .show(
                        &mut columns[0],
                        &mut self.app.project.next_node_id,
                        self.selected_node,
                        self.app.project.session.workspace_mode,
                        self.dragged_asset.clone(),
                    );

                if !output.commands.is_empty() {
                    let tx = self.app.project.begin_transaction();
                    if let Err(err) = self
                        .app
                        .project
                        .apply_command_batch("graph_edit", output.commands)
                    {
                        self.app.status_line = format!("graph command failed: {err}");
                    }
                    self.app.project.end_transaction();
                    let _ = tx;

                    sync_document_dependencies_from_snarl(
                        &mut self.app.project.document,
                        &self.app.canvas.snarl,
                    );
                    self.canvas_refresh_and_compile();
                }

                if let Some(selected) = output.selected_node {
                    self.selected_node = Some(selected);
                    self.app.project.session.selected_node = Some(selected);
                    self.inspector_name_buffer.clear();
                    self.inspector_setting_key_buffer.clear();
                    self.inspector_setting_value_buffer.clear();
                }

                if columns[0].ctx().input(|input| input.pointer.any_released()) {
                    self.dragged_asset = None;
                }

                let gameplay_nodes = self
                    .app
                    .project
                    .document
                    .scene
                    .graph
                    .nodes
                    .iter()
                    .filter(|node| node_workspace(node.kind) == EditorWorkspaceMode::Gameplay)
                    .count();
                let render_nodes = self
                    .app
                    .project
                    .document
                    .scene
                    .graph
                    .nodes
                    .iter()
                    .filter(|node| node_workspace(node.kind) == EditorWorkspaceMode::Render)
                    .count();
                columns[0].separator();
                columns[0].label(format!(
                    "Workspace: {} (gameplay nodes: {}, render nodes: {})",
                    workspace_label(self.app.project.session.workspace_mode),
                    gameplay_nodes,
                    render_nodes
                ));
                columns[0].label("Click a node to edit it in Inspector.");
                columns[0].label("Use Hierarchy/Layers on the left for scene editing.");
                columns[0].label(format!(
                    "Active scene: {}",
                    self.app.project.scene_path.display()
                ));

                columns[1].heading("Viewport");
                columns[1].label("Preview controls are in this Viewport panel");
                columns[1].horizontal(|ui| {
                    if ui.button("Pan Left").clicked() {
                        self.app.project.session.viewport.pan[0] -= 10.0;
                    }
                    if ui.button("Pan Right").clicked() {
                        self.app.project.session.viewport.pan[0] += 10.0;
                    }
                    if ui.button("Zoom +").clicked() {
                        self.app.project.session.viewport.zoom *= 1.1;
                    }
                    if ui.button("Zoom -").clicked() {
                        self.app.project.session.viewport.zoom /= 1.1;
                    }
                });

                self.draw_viewport_preview(&mut columns[1]);

                columns[1].label("Quick Object Focus");
                let objects = self.app.project.document.scene.objects.clone();
                egui::ScrollArea::vertical()
                    .max_height(96.0)
                    .show(&mut columns[1], |ui| {
                        for object in objects {
                            if ui
                                .button(format!("Focus {} ({})", object.name, object.object_id))
                                .clicked()
                            {
                                self.selected_object = Some(object.object_id);
                                self.app.project.session.viewport.selected_object =
                                    Some(object.object_id);
                                self.app.project.session.viewport.pan = [
                                    -object.components.transform.x,
                                    -object.components.transform.y,
                                ];
                            }
                        }
                    });

                let viewport = self.app.runtime.viewport_frame();
                let playing = self.app.runtime.is_play_mode();
                columns[1].horizontal(|ui| {
                    let tick_state = if playing { "running" } else { "paused" };
                    ui.label(format!(
                        "Runtime Tick: {} ({tick_state})",
                        viewport.frame_index
                    ));
                    ui.label("(?)")
                        .on_hover_text(
                            "Runtime Tick counts simulation/update steps. It advances only while running Play or when using Step.",
                        );
                });
                columns[1].label(format!("Viewport: {}x{}", viewport.width, viewport.height));
                columns[1].label(format!(
                    "Pan: ({:.1}, {:.1})  Zoom: {:.2}",
                    self.app.project.session.viewport.pan[0],
                    self.app.project.session.viewport.pan[1],
                    self.app.project.session.viewport.zoom
                ));
                columns[1].label(format!(
                    "Selected object: {:?}",
                    self.app.project.session.viewport.selected_object
                ));
            });
        });
    }

    fn draw_viewport_preview(&mut self, ui: &mut egui::Ui) {
        let viewport = self.app.runtime.viewport_frame();
        let panel_size = egui::vec2(ui.available_width().max(300.0), 300.0);

        if let Some(rgba) = viewport.rgba8.clone() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [viewport.width as usize, viewport.height as usize],
                &rgba,
            );

            let texture = self.viewport_texture.get_or_insert_with(|| {
                ui.ctx().load_texture(
                    "viewport_readback",
                    image.clone(),
                    egui::TextureOptions::LINEAR,
                )
            });
            texture.set(image, egui::TextureOptions::LINEAR);

            let response = ui.image((texture.id(), panel_size));
            if response.clicked() {
                if let Some(selected_object) = self.selected_object {
                    if let Some(object) = self
                        .app
                        .project
                        .document
                        .scene
                        .objects
                        .iter()
                        .find(|object| object.object_id == selected_object)
                    {
                        self.app.project.session.viewport.pan = [
                            -object.components.transform.x,
                            -object.components.transform.y,
                        ];
                    }
                }
            }
            ui.label(format!(
                "Viewport source: {} ({}x{})",
                viewport.source, viewport.width, viewport.height
            ));
        } else {
            let (rect, _) = ui.allocate_exact_size(panel_size, egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 8.0, egui::Color32::from_rgb(18, 20, 28));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No viewport frame readback yet",
                egui::FontId::proportional(16.0),
                egui::Color32::from_rgb(180, 190, 210),
            );
        }
    }

    fn draw_asset_preview(&self, ui: &mut egui::Ui, entry: &engine_editor::ProjectAssetEntry) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 130.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 6.0, egui::Color32::from_rgb(20, 24, 34));
        painter.rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(58, 66, 86)),
            egui::StrokeKind::Outside,
        );

        let center = rect.center();
        let size = egui::vec2(54.0, 54.0);
        let swatch = egui::Rect::from_center_size(center, size);

        match entry.kind {
            engine_editor::EditorAssetKind::Texture => {
                painter.rect_filled(swatch, 8.0, egui::Color32::from_rgb(90, 170, 255));
                painter.circle_filled(
                    swatch.center(),
                    16.0,
                    egui::Color32::from_rgb(210, 235, 255),
                );
                painter.text(
                    rect.left_top() + egui::vec2(12.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    "Texture preview",
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );
            }
            engine_editor::EditorAssetKind::Audio => {
                painter.circle_filled(swatch.center(), 22.0, egui::Color32::from_rgb(240, 170, 90));
                painter.text(
                    rect.left_top() + egui::vec2(12.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    "Audio asset",
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );
            }
            engine_editor::EditorAssetKind::Shape => {
                painter.rect_filled(swatch, 6.0, egui::Color32::from_rgb(88, 160, 120));
                let stem = entry
                    .path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();

                if stem.contains("circle") {
                    painter.circle_filled(
                        swatch.center(),
                        18.0,
                        egui::Color32::from_rgb(220, 245, 230),
                    );
                    painter.circle_stroke(
                        swatch.center(),
                        18.0,
                        egui::Stroke::new(1.5, egui::Color32::WHITE),
                    );
                } else if stem.contains("square") {
                    let rect =
                        egui::Rect::from_center_size(swatch.center(), egui::vec2(30.0, 30.0));
                    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(220, 245, 230));
                    painter.rect_stroke(
                        rect,
                        2.0,
                        egui::Stroke::new(1.5, egui::Color32::WHITE),
                        egui::StrokeKind::Outside,
                    );
                } else {
                    let triangle = vec![
                        egui::pos2(swatch.center().x, swatch.top() + 8.0),
                        egui::pos2(swatch.right() - 8.0, swatch.bottom() - 8.0),
                        egui::pos2(swatch.left() + 8.0, swatch.bottom() - 8.0),
                    ];
                    painter.add(egui::Shape::convex_polygon(
                        triangle,
                        egui::Color32::from_rgb(220, 245, 230),
                        egui::Stroke::new(1.5, egui::Color32::WHITE),
                    ));
                }
                painter.text(
                    rect.left_top() + egui::vec2(12.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    format!("Shape: {}", stem),
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );
            }
            engine_editor::EditorAssetKind::Graph => {
                painter.rect_filled(swatch, 6.0, egui::Color32::from_rgb(78, 110, 220));
                painter.line_segment(
                    [
                        egui::pos2(swatch.left(), swatch.top()),
                        egui::pos2(swatch.center().x, swatch.bottom()),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                );
                painter.line_segment(
                    [
                        egui::pos2(swatch.right(), swatch.top()),
                        egui::pos2(swatch.center().x, swatch.bottom()),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                );
                painter.text(
                    rect.left_top() + egui::vec2(12.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    "Graph asset",
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );
            }
            engine_editor::EditorAssetKind::Shader => {
                painter.rect_filled(swatch, 6.0, egui::Color32::from_rgb(150, 105, 235));
                painter.text(
                    swatch.center(),
                    egui::Align2::CENTER_CENTER,
                    "</>",
                    egui::FontId::proportional(24.0),
                    egui::Color32::WHITE,
                );
                painter.text(
                    rect.left_top() + egui::vec2(12.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    "Shader asset",
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );
            }
            engine_editor::EditorAssetKind::Unknown => {
                painter.rect_filled(swatch, 6.0, egui::Color32::from_rgb(96, 96, 96));
                painter.text(
                    swatch.center(),
                    egui::Align2::CENTER_CENTER,
                    "?",
                    egui::FontId::proportional(24.0),
                    egui::Color32::WHITE,
                );
            }
        }

        painter.text(
            rect.left_bottom() - egui::vec2(-12.0, 12.0),
            egui::Align2::LEFT_BOTTOM,
            entry
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("asset"),
            egui::FontId::monospace(12.0),
            egui::Color32::from_rgb(176, 186, 202),
        );
    }

    fn draw_diagnostics_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("diagnostics_panel")
            .resizable(true)
            .default_height(160.0)
            .show(ctx, |ui| {
                ui.heading("Diagnostics");
                let snapshot: RuntimeDiagnosticsSnapshot = self.app.runtime.diagnostics_snapshot();
                ui.label(format!("Backend: {:?}", snapshot.active_backend));
                ui.label(format!(
                    "CPU {:.2} ms, GPU {:.2} ms, Compile {:.2} ms",
                    snapshot.frame_timings.cpu_frame_ms,
                    snapshot.frame_timings.gpu_frame_ms,
                    snapshot.frame_timings.node_compile_ms
                ));
                ui.label(format!(
                    "Fallback events: {}, Shader rebuild errors: {}",
                    snapshot.telemetry.compile_fallback_events,
                    snapshot.telemetry.shader_rebuild_errors
                ));

                egui::ScrollArea::vertical().show(ui, |ui| {
                    if snapshot.compile_diagnostics.is_empty()
                        && snapshot.backend_diagnostics.events.is_empty()
                    {
                        ui.label("No diagnostics.");
                    }

                    for diag in snapshot.compile_diagnostics {
                        ui.label(format!(
                            "compile {:?} node={:?}: {}",
                            diag.severity, diag.node_id, diag.message
                        ));
                    }

                    for anchor in snapshot.diagnostic_anchors {
                        ui.horizontal(|ui| {
                            let label = format!(
                                "anchor {:?} node={:?} object={:?}: {}",
                                anchor.severity, anchor.node_id, anchor.object_id, anchor.message
                            );
                            if ui.button("Focus").clicked() {
                                if let Some(node_id) = anchor.node_id {
                                    self.selected_node = Some(node_id);
                                }
                                if let Some(object_id) = anchor.object_id {
                                    self.selected_object = Some(object_id);
                                }
                            }
                            ui.label(label);
                        });
                    }

                    for event in snapshot.backend_diagnostics.events.iter().rev().take(24) {
                        ui.label(format!("backend {:?}: {}", event.level, event.message));
                    }
                });
            });
    }

    fn draw_history_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("history_panel")
            .resizable(true)
            .default_height(120.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("History");
                    if ui.button("Undo").clicked() {
                        match self.app.project.undo() {
                            Ok(true) => self.canvas_refresh_and_compile(),
                            Ok(false) => self.app.status_line = "nothing to undo".to_string(),
                            Err(err) => self.app.status_line = format!("undo failed: {err}"),
                        }
                    }
                    if ui.button("Redo").clicked() {
                        match self.app.project.redo() {
                            Ok(true) => self.canvas_refresh_and_compile(),
                            Ok(false) => self.app.status_line = "nothing to redo".to_string(),
                            Err(err) => self.app.status_line = format!("redo failed: {err}"),
                        }
                    }
                    ui.label(format!(
                        "Current Node: {}",
                        self.app.project.session.history.current
                    ));
                });

                let history_nodes: Vec<_> = self
                    .app
                    .project
                    .session
                    .history
                    .nodes
                    .values()
                    .cloned()
                    .collect();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for node in history_nodes {
                        let selected = node.id == self.app.project.session.history.current;
                        let label = format!(
                            "#{} tx={} parent={:?} children={} {}",
                            node.id,
                            node.transaction.0,
                            node.parent,
                            node.children.len(),
                            node.label
                        );
                        ui.horizontal(|ui| {
                            if ui.selectable_label(selected, label).clicked() {
                                match self.app.project.checkout_history(node.id) {
                                    Ok(true) => self.canvas_refresh_and_compile(),
                                    Ok(false) => {}
                                    Err(err) => {
                                        self.app.status_line =
                                            format!("history checkout failed: {err}")
                                    }
                                }
                            }
                            if !self
                                .app
                                .project
                                .session
                                .history
                                .replay_matches_snapshot(node.id)
                            {
                                ui.colored_label(egui::Color32::YELLOW, "replay mismatch");
                            }
                        });
                    }
                });
            });
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (
            command,
            shift,
            key_s,
            key_z,
            key_y,
            key_c,
            key_v,
            key_d,
            key_delete,
            key_space,
            key_f,
        ) = ctx.input(|input| {
            (
                input.modifiers.command,
                input.modifiers.shift,
                input.key_pressed(egui::Key::S),
                input.key_pressed(egui::Key::Z),
                input.key_pressed(egui::Key::Y),
                input.key_pressed(egui::Key::C),
                input.key_pressed(egui::Key::V),
                input.key_pressed(egui::Key::D),
                input.key_pressed(egui::Key::Delete),
                input.key_pressed(egui::Key::Space),
                input.key_pressed(egui::Key::F),
            )
        });

        if command && key_s {
            if let Err(err) = self.app.save_active_scene() {
                self.app.status_line = format!("save failed: {err}");
            }
        }

        if command && key_z && !shift {
            match self.app.project.undo() {
                Ok(true) => self.canvas_refresh_and_compile(),
                Ok(false) => self.app.status_line = "nothing to undo".to_string(),
                Err(err) => self.app.status_line = format!("undo failed: {err}"),
            }
        }

        if (command && key_y) || (command && shift && key_z) {
            match self.app.project.redo() {
                Ok(true) => self.canvas_refresh_and_compile(),
                Ok(false) => self.app.status_line = "nothing to redo".to_string(),
                Err(err) => self.app.status_line = format!("redo failed: {err}"),
            }
        }

        if command && key_c {
            if let Some(node_id) = self.selected_node {
                self.node_clipboard = self
                    .app
                    .project
                    .document
                    .scene
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == node_id)
                    .cloned();
            } else if let Some(object_id) = self.selected_object {
                self.object_clipboard = self
                    .app
                    .project
                    .document
                    .scene
                    .objects
                    .iter()
                    .find(|object| object.object_id == object_id)
                    .cloned();
            }
        }

        if command && key_v {
            if let Some(node) = self.node_clipboard.clone() {
                let mut duplicated = node;
                duplicated.id = self.app.project.next_node_id;
                self.app.project.next_node_id += 1;
                duplicated.name = format!("{}_copy", duplicated.name);
                let position = self
                    .app
                    .project
                    .document
                    .node_positions
                    .get(&self.selected_node.unwrap_or(duplicated.id))
                    .copied()
                    .unwrap_or([120.0, 120.0]);
                self.apply_project_commands(
                    "paste_node",
                    vec![EditorCommand::AddNode {
                        node: duplicated,
                        position: [position[0] + 32.0, position[1] + 24.0],
                    }],
                    true,
                );
            } else if let Some(object) = self.object_clipboard.clone() {
                let mut duplicated = object;
                duplicated.object_id = self.app.project.allocate_object_id();
                duplicated.name = format!("{} Copy", duplicated.name);
                duplicated.parent = None;
                self.apply_project_commands(
                    "paste_object",
                    vec![EditorCommand::AddObject {
                        object: duplicated.clone(),
                    }],
                    false,
                );
                self.selected_object = Some(duplicated.object_id);
            }
        }

        if command && key_d {
            if let Some(object_id) = self.selected_object {
                if let Some(object) = self
                    .app
                    .project
                    .document
                    .scene
                    .objects
                    .iter()
                    .find(|object| object.object_id == object_id)
                    .cloned()
                {
                    let mut duplicated = object;
                    duplicated.object_id = self.app.project.allocate_object_id();
                    duplicated.name = format!("{} Copy", duplicated.name);
                    duplicated.parent = None;
                    self.apply_project_commands(
                        "duplicate_object_shortcut",
                        vec![EditorCommand::AddObject {
                            object: duplicated.clone(),
                        }],
                        false,
                    );
                    self.selected_object = Some(duplicated.object_id);
                }
            } else if let Some(node_id) = self.selected_node {
                if let Some(node) = self
                    .app
                    .project
                    .document
                    .scene
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == node_id)
                    .cloned()
                {
                    let mut duplicated = node;
                    duplicated.id = self.app.project.next_node_id;
                    self.app.project.next_node_id += 1;
                    duplicated.name = format!("{}_copy", duplicated.name);
                    let position = self
                        .app
                        .project
                        .document
                        .node_positions
                        .get(&node_id)
                        .copied()
                        .unwrap_or([120.0, 120.0]);
                    self.apply_project_commands(
                        "duplicate_node_shortcut",
                        vec![EditorCommand::AddNode {
                            node: duplicated,
                            position: [position[0] + 36.0, position[1] + 24.0],
                        }],
                        true,
                    );
                }
            }
        }

        if key_delete {
            if let Some(object_id) = self.selected_object {
                if let Some(object) = self
                    .app
                    .project
                    .document
                    .scene
                    .objects
                    .iter()
                    .find(|object| object.object_id == object_id)
                    .cloned()
                {
                    self.apply_project_commands(
                        "delete_object_shortcut",
                        vec![EditorCommand::RemoveObject { object }],
                        false,
                    );
                    self.selected_object = None;
                }
            } else if let Some(node_id) = self.selected_node {
                if let Some(command) = self.app.project.remove_node_command(node_id) {
                    self.apply_project_commands("delete_node_shortcut", vec![command], true);
                    self.selected_node = None;
                }
            }
        }

        if key_space {
            if self.app.runtime.is_play_mode() {
                self.app.runtime.stop_play_mode();
            } else {
                self.app.runtime.start_play_mode();
            }
        }

        if key_f {
            if let Some(object_id) = self.selected_object {
                if let Some(object) = self
                    .app
                    .project
                    .document
                    .scene
                    .objects
                    .iter()
                    .find(|object| object.object_id == object_id)
                {
                    self.app.project.session.viewport.pan = [
                        -object.components.transform.x,
                        -object.components.transform.y,
                    ];
                }
            }
        }
    }

    fn canvas_refresh_and_compile(&mut self) {
        self.app
            .canvas
            .rebuild_from_document(&self.app.project.document);
        match self
            .app
            .runtime
            .set_active_scene(self.app.project.document.scene.clone())
        {
            Ok(true) => {
                self.app.status_line = "graph compiled".to_string();
            }
            Ok(false) => {
                self.app.status_line =
                    "compile failed; last valid runtime artifact remains active".to_string();
            }
            Err(err) => {
                self.app.status_line = format!("runtime compile error: {err}");
            }
        }
    }
}

impl eframe::App for EditorUi {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);

        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested && self.app.project.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dirty_guard_modal_open = true;
            self.pending_project_action = None;
            self.app.status_line = "save or discard changes before closing the editor".to_string();
        }

        if self.project_modal_open {
            self.draw_project_modal(ctx);
            self.draw_dirty_guard_modal(ctx);
            return;
        }

        self.draw_autosave_restore_modal(ctx);
        self.draw_dirty_guard_modal(ctx);

        if let Err(err) = self.app.runtime.run_for_frames(1) {
            self.app.status_line = format!("runtime frame failed: {err}");
        }

        self.draw_top_menu(ctx);
        self.draw_assets_panel(ctx);
        self.draw_inspector_panel(ctx);
        self.draw_history_panel(ctx);
        self.draw_graph_and_viewport(ctx);
        self.draw_diagnostics_panel(ctx);

        if let Err(err) = self.app.project.autosave_if_dirty() {
            self.app.status_line = format!("autosave failed: {err}");
        }
    }
}
