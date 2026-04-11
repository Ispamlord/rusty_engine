use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use engine_core::ShaderToolchainConfig;
use engine_nodes::{deserialize_graph_ron, serialize_graph_ron, NodeGraph};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Texture,
    Audio,
    Graph,
    Shader,
    Unknown,
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

    #[error("shader parse error: {0}")]
    ShaderParse(String),

    #[error("shader compile error: {0}")]
    ShaderCompile(String),
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
                    set = value.parse::<u32>().ok();
                } else if let Some(value) = token.strip_prefix("binding=") {
                    binding = value.parse::<u32>().ok();
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
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "ktx2" => AssetKind::Texture,
        "wav" | "ogg" | "flac" | "mp3" => AssetKind::Audio,
        "ron" | "graph" => AssetKind::Graph,
        "vert" | "frag" | "comp" | "glsl" | "hlsl" => AssetKind::Shader,
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

        let Some(start) = line.find('"') else {
            continue;
        };
        let Some(end) = line[start + 1..].find('"') else {
            continue;
        };

        let include_name = &line[start + 1..start + 1 + end];
        let mut candidates = Vec::new();
        if let Some(parent) = path.parent() {
            candidates.push(parent.join(include_name));
        }
        for dir in include_dirs {
            candidates.push(dir.join(include_name));
        }

        if let Some(include_path) = candidates.into_iter().find(|candidate| candidate.exists()) {
            ordered.push(include_path.clone());
            resolve_includes_inner(&include_path, include_dirs, visited, ordered)?;
        }
    }

    Ok(())
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
        return Err(format!("compiler failed for {:?}: {stderr}", target));
    }

    fs::read(&temp_out).map_err(|err| format!("failed to read compiler output: {err}"))
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
        NodeGraph, NodeKind,
    };

    #[test]
    fn detect_asset_kind() {
        assert_eq!(
            infer_asset_kind(Path::new("sprite.png")),
            AssetKind::Texture
        );
        assert_eq!(infer_asset_kind(Path::new("scene.ron")), AssetKind::Graph);
        assert_eq!(infer_asset_kind(Path::new("sound.ogg")), AssetKind::Audio);
        assert_eq!(infer_asset_kind(Path::new("post.comp")), AssetKind::Shader);
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
}
