use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use engine_core::ShaderToolchainConfig;
use engine_nodes::{deserialize_graph_ron, serialize_graph_ron, NodeGraph, CURRENT_GRAPH_VERSION};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Texture,
    Audio,
    Shape,
    Graph,
    NodeConfig,
    Shader,
    Unknown,
}

pub const CURRENT_SCENE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SceneMetadata {
    pub name: String,
    pub author: String,
    pub description: String,
}

impl Default for SceneMetadata {
    fn default() -> Self {
        Self {
            name: "Untitled Scene".to_string(),
            author: "Unknown".to_string(),
            description: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SceneLayer {
    pub layer_id: u64,
    pub name: String,
    pub order: i32,
    pub visible: bool,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transform2D {
    pub x: f32,
    pub y: f32,
    pub rotation_radians: f32,
    pub scale_x: f32,
    pub scale_y: f32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            rotation_radians: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sprite2D {
    pub texture_asset: String,
    pub width: u32,
    pub height: u32,
    pub tint_rgba: [u8; 4],
    pub layer_order: i32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Collider2D {
    pub shape: String,
    pub radius: f32,
    pub width: f32,
    pub height: f32,
    pub is_sensor: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioEmitter {
    pub asset: String,
    pub volume: f32,
    pub looping: bool,
    pub spatial_blend: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Camera2DComponent {
    pub zoom: f32,
    pub near: f32,
    pub far: f32,
    pub clear_color_rgba: [u8; 4],
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScriptBinding {
    pub script_asset: String,
    pub entry: String,
    pub frame_phase: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RenderEffectMeta {
    pub material: String,
    pub effect_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct SceneComponents {
    #[serde(default)]
    pub transform: Transform2D,
    #[serde(default)]
    pub sprite: Option<Sprite2D>,
    #[serde(default)]
    pub collider: Option<Collider2D>,
    #[serde(default)]
    pub audio: Option<AudioEmitter>,
    #[serde(default)]
    pub camera: Option<Camera2DComponent>,
    #[serde(default)]
    pub script: Option<ScriptBinding>,
    #[serde(default)]
    pub custom_properties: BTreeMap<String, String>,
    #[serde(default)]
    pub render_effect: Option<RenderEffectMeta>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneObject {
    pub object_id: u64,
    pub parent: Option<u64>,
    pub layer_id: u64,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub components: SceneComponents,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneEditorState {
    #[serde(default)]
    pub selected_object_ids: Vec<u64>,
    #[serde(default)]
    pub selected_layer_id: Option<u64>,
    #[serde(default)]
    pub viewport_pan: [f32; 2],
    #[serde(default = "default_zoom")]
    pub viewport_zoom: f32,
}

const fn default_zoom() -> f32 {
    1.0
}

impl Default for SceneEditorState {
    fn default() -> Self {
        Self {
            selected_object_ids: Vec::new(),
            selected_layer_id: None,
            viewport_pan: [0.0, 0.0],
            viewport_zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneDocument {
    pub version: u32,
    #[serde(default)]
    pub metadata: SceneMetadata,
    #[serde(default)]
    pub layers: Vec<SceneLayer>,
    #[serde(default)]
    pub objects: Vec<SceneObject>,
    #[serde(default = "empty_graph")]
    pub graph: NodeGraph,
    #[serde(default)]
    pub editor_state: SceneEditorState,
}

fn empty_graph() -> NodeGraph {
    NodeGraph {
        version: CURRENT_GRAPH_VERSION,
        nodes: Vec::new(),
    }
}

impl SceneDocument {
    pub fn new_default() -> Self {
        Self {
            version: CURRENT_SCENE_VERSION,
            metadata: SceneMetadata::default(),
            layers: vec![SceneLayer {
                layer_id: 1,
                name: "Main".to_string(),
                order: 0,
                visible: true,
                locked: false,
            }],
            objects: vec![SceneObject {
                object_id: 1,
                parent: None,
                layer_id: 1,
                name: "Camera".to_string(),
                tags: vec!["camera".to_string()],
                components: SceneComponents {
                    camera: Some(Camera2DComponent {
                        zoom: 1.0,
                        near: -1000.0,
                        far: 1000.0,
                        clear_color_rgba: [16, 20, 28, 255],
                    }),
                    ..SceneComponents::default()
                },
            }],
            graph: NodeGraph {
                version: CURRENT_GRAPH_VERSION,
                nodes: Vec::new(),
            },
            editor_state: SceneEditorState::default(),
        }
    }

    pub fn from_graph(graph: NodeGraph) -> Self {
        let mut scene = Self::new_default();
        scene.graph = graph;
        scene
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetChange {
    pub path: PathBuf,
    pub kind: AssetKind,
    pub modified: SystemTime,
}

#[derive(Debug, Clone)]
pub struct AssetCacheEntry {
    pub key: String,
    pub built_at: SystemTime,
}

#[derive(Debug, Default)]
pub struct AssetBuildCache {
    entries: HashMap<PathBuf, AssetCacheEntry>,
    include_dependents: HashMap<PathBuf, HashSet<PathBuf>>,
}

#[derive(Debug, Default)]
pub struct AssetHotReload {
    known_timestamps: HashMap<PathBuf, SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderSourceKind {
    Glsl,
    Hlsl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderTarget {
    VulkanSpirv,
    Dx12Dxil,
    Dx11Dxbc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderBinding {
    pub set: u32,
    pub binding: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderMetadata {
    pub entry_point: String,
    pub bindings: Vec<ShaderBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderCompileOptions {
    pub toolchain: ShaderToolchainConfig,
    pub optimization: String,
    pub include_dirs: Vec<PathBuf>,
}

impl Default for ShaderCompileOptions {
    fn default() -> Self {
        Self {
            toolchain: ShaderToolchainConfig::default(),
            optimization: "O2".to_string(),
            include_dirs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderArtifact {
    pub source_hash: String,
    pub compile_key: String,
    pub source_kind: ShaderSourceKind,
    pub target: ShaderTarget,
    pub metadata: ShaderMetadata,
    pub bytecode: Vec<u8>,
    pub compiler_signature: String,
    pub include_files: Vec<PathBuf>,
}

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("graph parse error: {0}")]
    Parse(String),

    #[error("graph encode error: {0}")]
    Encode(String),

    #[error("scene parse error: {0}")]
    SceneParse(String),

    #[error("scene encode error: {0}")]
    SceneEncode(String),

    #[error("legacy graph-only scene format is not supported for this editor workflow")]
    LegacyGraphFormat,

    #[error("shader parse error: {0}")]
    ShaderParse(String),

    #[error("shader compile error: {0}")]
    ShaderCompile(String),

    #[error("shader include error: {0}")]
    UnresolvedInclude(String),
}

impl AssetHotReload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn scan_changes(&mut self, root: impl AsRef<Path>) -> Result<Vec<AssetChange>, AssetError> {
        let mut files = Vec::new();
        collect_files_recursive(root.as_ref(), &mut files)?;

        let mut changes = Vec::new();
        for file in files {
            let metadata = fs::metadata(&file)?;
            let modified = metadata.modified()?;

            let changed = match self.known_timestamps.get(&file) {
                Some(previous) => previous < &modified,
                None => true,
            };

            if changed {
                self.known_timestamps.insert(file.clone(), modified);
                changes.push(AssetChange {
                    kind: infer_asset_kind(&file),
                    path: file,
                    modified,
                });
            }
        }

        Ok(changes)
    }
}

impl AssetBuildCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn invalidate(&mut self, path: &Path) {
        self.entries.remove(path);

        if let Some(dependents) = self.include_dependents.remove(path) {
            for dependent in dependents {
                self.entries.remove(&dependent);
            }
        }
    }

    pub fn has_valid_entry(&self, path: &Path, key: &str) -> bool {
        self.entries.get(path).is_some_and(|entry| entry.key == key)
    }

    pub fn update(&mut self, path: PathBuf, key: String, include_files: &[PathBuf]) {
        self.entries.insert(
            path.clone(),
            AssetCacheEntry {
                key,
                built_at: SystemTime::now(),
            },
        );

        for include in include_files {
            self.include_dependents
                .entry(include.clone())
                .or_default()
                .insert(path.clone());
        }
    }

    pub fn build_or_reuse_shader(
        &mut self,
        path: impl AsRef<Path>,
        source_kind: ShaderSourceKind,
        target: ShaderTarget,
        options: &ShaderCompileOptions,
    ) -> Result<(ShaderArtifact, bool), AssetError> {
        let path = path.as_ref();
        let artifact = compile_shader_file(path, source_kind, target, options)?;

        if self.has_valid_entry(path, &artifact.compile_key) {
            return Ok((artifact, true));
        }

        self.update(
            path.to_path_buf(),
            artifact.compile_key.clone(),
            &artifact.include_files,
        );
        Ok((artifact, false))
    }
}

pub fn load_node_graph(path: impl AsRef<Path>) -> Result<NodeGraph, AssetError> {
    let source = fs::read_to_string(path)?;
    deserialize_graph_ron(&source).map_err(|err| AssetError::Parse(err.to_string()))
}

pub fn save_node_graph(path: impl AsRef<Path>, graph: &NodeGraph) -> Result<(), AssetError> {
    let encoded = serialize_graph_ron(graph).map_err(|err| AssetError::Encode(err.to_string()))?;
    fs::write(path, encoded)?;
    Ok(())
}

pub fn load_scene_document(path: impl AsRef<Path>) -> Result<SceneDocument, AssetError> {
    let source = fs::read_to_string(path)?;
    if ron::from_str::<NodeGraph>(&source).is_ok()
        && !source.contains("metadata:")
        && !source.contains("layers:")
        && !source.contains("objects:")
    {
        return Err(AssetError::LegacyGraphFormat);
    }

    match ron::from_str::<SceneDocument>(&source) {
        Ok(mut scene) => {
            if scene.version == 0 {
                scene.version = CURRENT_SCENE_VERSION;
            }
            if scene.layers.is_empty() {
                scene.layers.push(SceneLayer {
                    layer_id: 1,
                    name: "Main".to_string(),
                    order: 0,
                    visible: true,
                    locked: false,
                });
            }
            scene.graph = deserialize_graph_ron(
                &serialize_graph_ron(&scene.graph)
                    .map_err(|err| AssetError::SceneParse(err.to_string()))?,
            )
            .map_err(|err| AssetError::SceneParse(err.to_string()))?;
            Ok(scene)
        }
        Err(scene_err) => Err(AssetError::SceneParse(scene_err.to_string())),
    }
}

pub fn save_scene_document(
    path: impl AsRef<Path>,
    scene: &SceneDocument,
) -> Result<(), AssetError> {
    let mut scene = scene.clone();
    scene.version = CURRENT_SCENE_VERSION;
    scene.graph.version = CURRENT_GRAPH_VERSION;
    let encoded = ron::to_string(&scene).map_err(|err| AssetError::SceneEncode(err.to_string()))?;
    fs::write(path, encoded)?;
    Ok(())
}

pub fn compile_shader_file(
    path: &Path,
    source_kind: ShaderSourceKind,
    target: ShaderTarget,
    options: &ShaderCompileOptions,
) -> Result<ShaderArtifact, AssetError> {
    let source = fs::read_to_string(path)?;
    let metadata = parse_shader_metadata(&source)?;

    let include_files = resolve_includes(path, &options.include_dirs)?;
    let source_hash = hash_source(&source);
    let include_hash = hash_files(&include_files)?;
    let compiler_signature = compiler_signature(target, &options.toolchain);
    let compile_key = compute_compile_key(
        &source_hash,
        &include_hash,
        target,
        &metadata.entry_point,
        options,
        &compiler_signature,
    );

    let bytecode = match compile_external(path, target, &metadata, options) {
        Ok(bytes) => bytes,
        Err(err) => {
            if options.toolchain.strict {
                return Err(AssetError::ShaderCompile(err));
            }

            tracing::warn!("shader compile fallback for {:?}: {err}", path);
            placeholder_bytecode(target, &compile_key, &metadata.entry_point)
        }
    };

    Ok(ShaderArtifact {
        source_hash,
        compile_key,
        source_kind,
        target,
        metadata,
        bytecode,
        compiler_signature,
        include_files,
    })
}

pub fn compile_shader_source(
    source: &str,
    source_kind: ShaderSourceKind,
    target: ShaderTarget,
    options: &ShaderCompileOptions,
) -> Result<ShaderArtifact, AssetError> {
    let metadata = parse_shader_metadata(source)?;
    let source_hash = hash_source(source);
    let compiler_signature = compiler_signature(target, &options.toolchain);
    let compile_key = compute_compile_key(
        &source_hash,
        "no_includes",
        target,
        &metadata.entry_point,
        options,
        &compiler_signature,
    );

    Ok(ShaderArtifact {
        source_hash,
        compile_key: compile_key.clone(),
        source_kind,
        target,
        metadata,
        bytecode: placeholder_bytecode(target, &compile_key, "memory_source"),
        compiler_signature,
        include_files: Vec::new(),
    })
}

pub fn parse_shader_metadata(source: &str) -> Result<ShaderMetadata, AssetError> {
    let mut entry_point = None;
    let mut bindings = Vec::new();

    for line in source.lines() {
        let line = line.trim();
        if !line.starts_with("//@") {
            continue;
        }

        if let Some(raw_entry) = line.strip_prefix("//@entry") {
            let parsed_entry = raw_entry.trim();
            if parsed_entry.is_empty() {
                return Err(AssetError::ShaderParse(
                    "entry directive requires a function name".to_string(),
                ));
            }
            entry_point = Some(parsed_entry.to_string());
            continue;
        }

        if let Some(raw_binding) = line.strip_prefix("//@binding") {
            let mut set = None;
            let mut binding = None;
            let mut name = None;

            for token in raw_binding.split_whitespace() {
                if let Some(value) = token.strip_prefix("set=") {
                    set = Some(value.parse::<u32>().map_err(|_| {
                        AssetError::ShaderParse(format!(
                            "binding directive has invalid 'set' value: {value}"
                        ))
                    })?);
                } else if let Some(value) = token.strip_prefix("binding=") {
                    binding = Some(value.parse::<u32>().map_err(|_| {
                        AssetError::ShaderParse(format!(
                            "binding directive has invalid 'binding' value: {value}"
                        ))
                    })?);
                } else if let Some(value) = token.strip_prefix("name=") {
                    name = Some(value.to_string());
                }
            }

            let set = set.ok_or_else(|| {
                AssetError::ShaderParse("binding directive missing 'set'".to_string())
            })?;
            let binding = binding.ok_or_else(|| {
                AssetError::ShaderParse("binding directive missing 'binding'".to_string())
            })?;
            let name = name.ok_or_else(|| {
                AssetError::ShaderParse("binding directive missing 'name'".to_string())
            })?;

            bindings.push(ShaderBinding { set, binding, name });
        }
    }

    Ok(ShaderMetadata {
        entry_point: entry_point.unwrap_or_else(|| "main".to_string()),
        bindings,
    })
}

pub fn infer_asset_kind(path: &Path) -> AssetKind {
    let file_name_lower = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if file_name_lower.ends_with(".node.yml") || file_name_lower.ends_with(".node.yaml") {
        return AssetKind::NodeConfig;
    }

    if path
        .components()
        .any(|component| component.as_os_str() == "basic_shapes")
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .eq_ignore_ascii_case("ron")
    {
        return AssetKind::Shape;
    }

    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "ktx2" => AssetKind::Texture,
        "wav" | "ogg" | "flac" | "mp3" => AssetKind::Audio,
        "ron" | "graph" | "scene" => AssetKind::Graph,
        "vert" | "frag" | "comp" | "glsl" | "hlsl" | "rhai" => AssetKind::Shader,
        _ => AssetKind::Unknown,
    }
}

fn collect_files_recursive(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), AssetError> {
    if root.is_file() {
        output.push(root.to_path_buf());
        return Ok(());
    }

    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, output)?;
        } else {
            output.push(path);
        }
    }

    Ok(())
}

fn hash_source(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn hash_files(files: &[PathBuf]) -> Result<String, AssetError> {
    let mut hasher = Sha256::new();

    for file in files {
        hasher.update(file.to_string_lossy().as_bytes());
        if file.exists() {
            hasher.update(fs::read(file)?);
        }
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn resolve_includes(path: &Path, include_dirs: &[PathBuf]) -> Result<Vec<PathBuf>, AssetError> {
    let mut visited = HashSet::new();
    let mut ordered = Vec::new();
    resolve_includes_inner(path, include_dirs, &mut visited, &mut ordered)?;
    ordered.sort();
    ordered.dedup();
    Ok(ordered)
}

fn resolve_includes_inner(
    path: &Path,
    include_dirs: &[PathBuf],
    visited: &mut HashSet<PathBuf>,
    ordered: &mut Vec<PathBuf>,
) -> Result<(), AssetError> {
    let canonical = path.to_path_buf();
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }

    let source = fs::read_to_string(path)?;
    for line in source.lines() {
        let line = line.trim();
        if !line.starts_with("#include") {
            continue;
        }

        let include_name = extract_quoted_include(line)
            .ok_or_else(|| {
                AssetError::UnresolvedInclude(format!(
                    "unsupported #include syntax in '{}': {line}",
                    path.display()
                ))
            })?;

        let mut candidates = Vec::new();
        if let Some(parent) = path.parent() {
            candidates.push(parent.join(include_name));
        }
        for dir in include_dirs {
            candidates.push(dir.join(include_name));
        }

        let include_path = candidates
            .into_iter()
            .find(|candidate| candidate.exists())
            .ok_or_else(|| {
                AssetError::UnresolvedInclude(format!(
                    "could not resolve #include '{include_name}' in '{}'",
                    path.display()
                ))
            })?;

        ordered.push(include_path.clone());
        resolve_includes_inner(&include_path, include_dirs, visited, ordered)?;
    }

    Ok(())
}

fn extract_quoted_include(line: &str) -> Option<&str> {
    let start = line.find('"')?;
    let end = line[start + 1..].find('"')?;
    Some(&line[start + 1..start + 1 + end])
}

fn compute_compile_key(
    source_hash: &str,
    include_hash: &str,
    target: ShaderTarget,
    entry: &str,
    options: &ShaderCompileOptions,
    compiler_signature: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_hash.as_bytes());
    hasher.update(include_hash.as_bytes());
    hasher.update(format!("{:?}", target).as_bytes());
    hasher.update(entry.as_bytes());
    hasher.update(options.optimization.as_bytes());
    hasher.update(options.toolchain.vulkan_profile.as_bytes());
    hasher.update(options.toolchain.dx12_profile.as_bytes());
    hasher.update(options.toolchain.dx11_profile.as_bytes());
    hasher.update(compiler_signature.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn compiler_signature(target: ShaderTarget, toolchain: &ShaderToolchainConfig) -> String {
    let cmd = match target {
        ShaderTarget::VulkanSpirv => &toolchain.glslc_path,
        ShaderTarget::Dx12Dxil => &toolchain.dxc_path,
        ShaderTarget::Dx11Dxbc => &toolchain.fxc_path,
    };

    let version = Command::new(cmd)
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    format!("{cmd}:{version}")
}

fn compile_external(
    path: &Path,
    target: ShaderTarget,
    metadata: &ShaderMetadata,
    options: &ShaderCompileOptions,
) -> Result<Vec<u8>, String> {
    let temp_out = std::env::temp_dir().join(format!(
        "rusty_engine_shader_{}.bin",
        hash_source(path.to_string_lossy().as_ref())
    ));

    let mut command = match target {
        ShaderTarget::VulkanSpirv => {
            let mut command = Command::new(&options.toolchain.glslc_path);
            command.arg(path).arg("-o").arg(&temp_out).arg(format!(
                "-O{}",
                options.optimization.trim_start_matches('O')
            ));
            command
        }
        ShaderTarget::Dx12Dxil => {
            let mut command = Command::new(&options.toolchain.dxc_path);
            command
                .arg(path)
                .arg("-E")
                .arg(&metadata.entry_point)
                .arg("-T")
                .arg(&options.toolchain.dx12_profile)
                .arg("-Fo")
                .arg(&temp_out)
                .arg("-Zi");
            command
        }
        ShaderTarget::Dx11Dxbc => {
            let mut command = Command::new(&options.toolchain.fxc_path);
            command
                .arg("/T")
                .arg(&options.toolchain.dx11_profile)
                .arg("/E")
                .arg(&metadata.entry_point)
                .arg("/Fo")
                .arg(&temp_out)
                .arg(path);
            command
        }
    };

    for include in &options.include_dirs {
        command.arg("-I").arg(include);
    }

    let output = command
        .output()
        .map_err(|err| format!("failed to run compiler for {:?}: {err}", target))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = fs::remove_file(&temp_out);
        return Err(format!("compiler failed for {:?}: {stderr}", target));
    }

    let bytecode = fs::read(&temp_out)
        .map_err(|err| format!("failed to read compiler output: {err}"));
    let _ = fs::remove_file(&temp_out);
    bytecode
}

fn placeholder_bytecode(target: ShaderTarget, compile_key: &str, entry: &str) -> Vec<u8> {
    let mut bytecode = Vec::new();
    bytecode.extend_from_slice(match target {
        ShaderTarget::VulkanSpirv => b"SPIRV_PLACEHOLDER\0",
        ShaderTarget::Dx12Dxil => b"DXIL_PLACEHOLDER\0",
        ShaderTarget::Dx11Dxbc => b"DXBC_PLACEHOLDER\0",
    });
    bytecode.extend_from_slice(compile_key.as_bytes());
    bytecode.extend_from_slice(entry.as_bytes());
    bytecode
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_nodes::{
        ComputeDispatchConfig, Node, NodeExecutionTarget, NodeFallbackPolicy, NodeGpuResourceState,
        NodeGraph, NodeKind, NodePayload,
    };
    use std::collections::BTreeMap;

    #[test]
    fn detect_asset_kind() {
        assert_eq!(
            infer_asset_kind(Path::new("sprite.png")),
            AssetKind::Texture
        );
        assert_eq!(
            infer_asset_kind(Path::new("assets/basic_shapes/square.ron")),
            AssetKind::Shape
        );
        assert_eq!(infer_asset_kind(Path::new("scene.ron")), AssetKind::Graph);
        assert_eq!(infer_asset_kind(Path::new("scene.scene")), AssetKind::Graph);
        assert_eq!(
            infer_asset_kind(Path::new("assets/nodes/decision.node.yml")),
            AssetKind::NodeConfig
        );
        assert_eq!(
            infer_asset_kind(Path::new("assets/nodes/decision.node.yaml")),
            AssetKind::NodeConfig
        );
        assert_eq!(infer_asset_kind(Path::new("sound.ogg")), AssetKind::Audio);
        assert_eq!(infer_asset_kind(Path::new("post.comp")), AssetKind::Shader);
        assert_eq!(infer_asset_kind(Path::new("logic.rhai")), AssetKind::Shader);
    }

    #[test]
    fn roundtrip_graph_file() {
        let temp_dir = std::env::temp_dir().join("rusty_engine_asset_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("temp dir should exist");

        let graph_path = temp_dir.join("scene.ron");
        let graph = NodeGraph {
            version: engine_nodes::CURRENT_GRAPH_VERSION,
            nodes: vec![Node {
                id: 1,
                name: "start".to_string(),
                kind: NodeKind::GameplayEvent,
                target: NodeExecutionTarget::Cpu,
                dependencies: vec![],
                settings: Default::default(),
                gpu_bindings: vec![],
                compute: Some(ComputeDispatchConfig::default()),
                fallback_policy: NodeFallbackPolicy::Cpu,
                gpu_resource_states: vec![NodeGpuResourceState {
                    resource: "data".to_string(),
                    access: engine_nodes::GpuResourceAccess::Read,
                }],
                shader_entry: None,
                shader_profile: None,
                payload: Some(NodePayload::GameplayEvent(Default::default())),
            }],
        };

        save_node_graph(&graph_path, &graph).expect("graph should save");
        let decoded = load_node_graph(&graph_path).expect("graph should load");
        assert_eq!(graph, decoded);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn parse_shader_metadata_directives() {
        let source = r#"
            //@entry cs_main
            //@binding set=0 binding=0 name=particles
            //@binding set=1 binding=2 name=constants
        "#;

        let metadata = parse_shader_metadata(source).expect("metadata should parse");
        assert_eq!(metadata.entry_point, "cs_main");
        assert_eq!(metadata.bindings.len(), 2);
        assert_eq!(metadata.bindings[0].name, "particles");
    }

    #[test]
    fn parse_shader_metadata_rejects_invalid_binding_values() {
        let source = r#"
            //@entry cs_main
            //@binding set=abc binding=0 name=particles
        "#;

        let result = parse_shader_metadata(source);
        assert!(
            matches!(result, Err(AssetError::ShaderParse(_))),
            "expected shader parse error, got {result:?}"
        );
    }

    #[test]
    fn resolve_includes_finds_nested_dependencies() {
        let temp_dir = std::env::temp_dir().join("rusty_engine_include_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("temp dir should exist");

        let header_path = temp_dir.join("common.hlsl");
        let source_path = temp_dir.join("main.hlsl");

        fs::write(&header_path, "//@entry main\n").expect("header should be written");
        fs::write(
            &source_path,
            r#"#include "common.hlsl"
//@entry cs_main
"#,
        )
        .expect("source should be written");

        let options = ShaderCompileOptions::default();
        let artifact = compile_shader_file(
            &source_path,
            ShaderSourceKind::Hlsl,
            ShaderTarget::Dx12Dxil,
            &options,
        )
        .expect("compile should succeed");

        assert!(artifact.include_files.contains(&header_path));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn resolve_includes_errors_on_missing_file() {
        let temp_dir = std::env::temp_dir().join("rusty_engine_include_missing_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("temp dir should exist");

        let source_path = temp_dir.join("main.hlsl");
        fs::write(
            &source_path,
            r#"#include "nonexistent.hlsl"
//@entry cs_main
"#,
        )
        .expect("source should be written");

        let options = ShaderCompileOptions::default();
        let result = compile_shader_file(
            &source_path,
            ShaderSourceKind::Hlsl,
            ShaderTarget::Dx12Dxil,
            &options,
        );

        assert!(
            matches!(result, Err(AssetError::UnresolvedInclude(_))),
            "expected unresolved include error, got {result:?}"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn cache_invalidates_on_source_change() {
        let mut cache = AssetBuildCache::new();

        let temp_dir = std::env::temp_dir().join("rusty_engine_shader_cache_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let shader_path = temp_dir.join("particles.comp");

        fs::write(&shader_path, "//@entry main\n").expect("shader should be written");
        let options = ShaderCompileOptions::default();
        let (_, reused) = cache
            .build_or_reuse_shader(
                &shader_path,
                ShaderSourceKind::Glsl,
                ShaderTarget::VulkanSpirv,
                &options,
            )
            .expect("build should succeed");
        assert!(!reused);

        let (_, reused) = cache
            .build_or_reuse_shader(
                &shader_path,
                ShaderSourceKind::Glsl,
                ShaderTarget::VulkanSpirv,
                &options,
            )
            .expect("build should succeed");
        assert!(reused);

        fs::write(&shader_path, "//@entry updated\n").expect("shader should be rewritten");
        let (_, reused) = cache
            .build_or_reuse_shader(
                &shader_path,
                ShaderSourceKind::Glsl,
                ShaderTarget::VulkanSpirv,
                &options,
            )
            .expect("build should succeed");
        assert!(!reused);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn compile_key_changes_with_toolchain_profile() {
        let source = "//@entry main\n";
        let options_a = ShaderCompileOptions::default();
        let mut options_b = ShaderCompileOptions::default();
        options_b.toolchain.dx12_profile = "cs_6_7".to_string();

        let artifact_a = compile_shader_source(
            source,
            ShaderSourceKind::Hlsl,
            ShaderTarget::Dx12Dxil,
            &options_a,
        )
        .expect("compile should succeed");
        let artifact_b = compile_shader_source(
            source,
            ShaderSourceKind::Hlsl,
            ShaderTarget::Dx12Dxil,
            &options_b,
        )
        .expect("compile should succeed");

        assert_ne!(artifact_a.compile_key, artifact_b.compile_key);
    }

    #[test]
    fn scene_roundtrip_and_legacy_detection() {
        let temp_dir = std::env::temp_dir().join("rusty_engine_scene_asset_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("temp dir should exist");

        let scene_path = temp_dir.join("sample.scene.ron");
        let graph_path = temp_dir.join("legacy_graph.ron");

        let mut scene = SceneDocument::new_default();
        scene.metadata.name = "Scene Roundtrip".to_string();
        scene.graph = NodeGraph {
            version: CURRENT_GRAPH_VERSION,
            nodes: vec![Node {
                id: 7,
                name: "script".to_string(),
                kind: NodeKind::ScriptBehavior,
                target: NodeExecutionTarget::Cpu,
                dependencies: vec![],
                settings: BTreeMap::new(),
                gpu_bindings: vec![],
                compute: None,
                fallback_policy: NodeFallbackPolicy::Cpu,
                gpu_resource_states: vec![],
                shader_entry: None,
                shader_profile: None,
                payload: Some(NodePayload::ScriptBehavior(Default::default())),
            }],
        };

        save_scene_document(&scene_path, &scene).expect("scene save should work");
        let loaded = load_scene_document(&scene_path).expect("scene load should work");
        assert_eq!(loaded.metadata.name, "Scene Roundtrip");
        assert_eq!(loaded.graph.nodes.len(), 1);

        save_node_graph(&graph_path, &scene.graph).expect("legacy graph save should work");
        let legacy = load_scene_document(&graph_path).expect_err("legacy graph should be rejected");
        assert!(matches!(legacy, AssetError::LegacyGraphFormat));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
