use std::collections::{BTreeMap, BTreeSet, HashMap};

pub mod config;
pub mod custom_registry;

pub use config::{
    load_node_config, parse_node_config, save_node_config, NodeConfigDocument, NodeConfigError,
    NodeInputConfig, NodeOutputConfig, NodeTypeDescriptor, PredefinedNodeType,
};
pub use custom_registry::{
    load_custom_node_registry, parse_custom_node_registry, save_custom_node_registry,
    CustomNodeRegistration, CustomNodeRegistry, CustomNodeRegistryError,
};

use engine_render_api::{
    BackendCapabilities, BackendKind, BlendMode, Camera2d, GraphResourceDescriptor,
    GraphResourceKind, GraphResourceLifetime, RenderGraph, RenderGraphPass, RenderPassNode,
    RenderTargetDescriptor, RenderTargetHandle, SpriteBatchCommand, SpriteInstance, TextureHandle,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CURRENT_GRAPH_VERSION: u32 = 2;

pub type NodeId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameplayEventPayload {
    pub event_name: String,
}

impl Default for GameplayEventPayload {
    fn default() -> Self {
        Self {
            event_name: "on_update".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameplayFlowPayload {
    pub condition_key: String,
    pub expected_value: String,
}

impl Default for GameplayFlowPayload {
    fn default() -> Self {
        Self {
            condition_key: "state".to_string(),
            expected_value: "ready".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MathStatePayload {
    pub operation: String,
    pub lhs: f32,
    pub rhs: f32,
    pub output_key: String,
}

impl Default for MathStatePayload {
    fn default() -> Self {
        Self {
            operation: "add".to_string(),
            lhs: 0.0,
            rhs: 0.0,
            output_key: "result".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectInitializerPayload {
    pub object_name: String,
    pub layer_id: u64,
    pub x: f32,
    pub y: f32,
}

impl Default for ObjectInitializerPayload {
    fn default() -> Self {
        Self {
            object_name: "Entity".to_string(),
            layer_id: 1,
            x: 0.0,
            y: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptBehaviorPayload {
    pub script_asset: String,
    pub entry: String,
    pub frame_phase: String,
}

impl Default for ScriptBehaviorPayload {
    fn default() -> Self {
        Self {
            script_asset: "assets/scripts/behavior.rhai".to_string(),
            entry: "update".to_string(),
            frame_phase: "gameplay".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderPassPayload {
    pub target_resource: String,
    pub target_width: u32,
    pub target_height: u32,
    pub sprite_count: u32,
    pub blend: String,
}

impl Default for RenderPassPayload {
    fn default() -> Self {
        Self {
            target_resource: "frame_color".to_string(),
            target_width: 1920,
            target_height: 1080,
            sprite_count: 0,
            blend: "alpha".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputePassPayload {
    pub shader: String,
    pub dispatch: [u32; 3],
    pub reads: Vec<String>,
    pub writes: Vec<String>,
}

impl Default for ComputePassPayload {
    fn default() -> Self {
        Self {
            shader: "compute.hlsl".to_string(),
            dispatch: [1, 1, 1],
            reads: Vec::new(),
            writes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetReferencePayload {
    pub asset_path: String,
    pub asset_kind: String,
}

impl Default for AssetReferencePayload {
    fn default() -> Self {
        Self {
            asset_path: String::new(),
            asset_kind: "Unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildExportPayload {
    pub target: String,
}

impl Default for BuildExportPayload {
    fn default() -> Self {
        Self {
            target: "build/default".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NodeLibraryScope {
    #[default]
    ProjectLocal,
    GlobalShared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomNodePayload {
    pub type_name: String,
    pub config_path: String,
    #[serde(default)]
    pub impl_path: Option<String>,
    #[serde(default)]
    pub library_scope: NodeLibraryScope,
}

impl Default for CustomNodePayload {
    fn default() -> Self {
        Self {
            type_name: "CustomNode".to_string(),
            config_path: "assets/nodes/custom.node.yml".to_string(),
            impl_path: Some("assets/nodes/custom.rhai".to_string()),
            library_scope: NodeLibraryScope::ProjectLocal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodePayload {
    GameplayEvent(GameplayEventPayload),
    GameplayFlow(GameplayFlowPayload),
    MathState(MathStatePayload),
    ScriptBehavior(ScriptBehaviorPayload),
    ObjectInitializer(ObjectInitializerPayload),
    RenderPass(RenderPassPayload),
    ComputePass(ComputePassPayload),
    AssetReference(AssetReferencePayload),
    BuildExport(BuildExportPayload),
    Custom(CustomNodePayload),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeExecutionTarget {
    Cpu,
    Gpu,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    GameplayFlow,
    GameplayEvent,
    MathState,
    ScriptBehavior,
    ObjectInitializer,
    RenderPass,
    ComputePass,
    AssetReference,
    BuildExport,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NodeFallbackPolicy {
    Error,
    #[default]
    Cpu,
    Disable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuResourceAccess {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeGpuResourceState {
    pub resource: String,
    pub access: GpuResourceAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeGpuBinding {
    pub set: u32,
    pub binding: u32,
    pub resource: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeDispatchConfig {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl Default for ComputeDispatchConfig {
    fn default() -> Self {
        Self { x: 1, y: 1, z: 1 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub target: NodeExecutionTarget,
    pub dependencies: Vec<NodeId>,
    pub settings: BTreeMap<String, String>,
    #[serde(default)]
    pub gpu_bindings: Vec<NodeGpuBinding>,
    #[serde(default)]
    pub compute: Option<ComputeDispatchConfig>,
    #[serde(default)]
    pub fallback_policy: NodeFallbackPolicy,
    #[serde(default)]
    pub gpu_resource_states: Vec<NodeGpuResourceState>,
    #[serde(default)]
    pub shader_entry: Option<String>,
    #[serde(default)]
    pub shader_profile: Option<String>,
    #[serde(default)]
    pub payload: Option<NodePayload>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeGraph {
    pub version: u32,
    pub nodes: Vec<Node>,
}

impl NodeGraph {
    pub fn empty() -> Self {
        Self {
            version: CURRENT_GRAPH_VERSION,
            nodes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCompileOptions {
    pub optimize: bool,
    pub force_cpu_fallback: bool,
    pub allow_gpu_for_hybrid: bool,
    pub strict_gpu: bool,
}

impl Default for NodeCompileOptions {
    fn default() -> Self {
        Self {
            optimize: true,
            force_cpu_fallback: false,
            allow_gpu_for_hybrid: true,
            strict_gpu: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileDiagnostic {
    pub severity: DiagnosticSeverity,
    pub node_id: Option<NodeId>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolvedExecutionTarget {
    Cpu,
    Gpu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EcsJobDescriptor {
    pub node_id: NodeId,
    pub node_name: String,
    pub phase: String,
    pub execution: ResolvedExecutionTarget,
    pub payload_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuPassDescriptor {
    pub node_id: NodeId,
    pub backend: BackendKind,
    pub label: String,
    pub is_compute: bool,
    pub shader_entry: String,
    pub shader_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptJobDescriptor {
    pub node_id: NodeId,
    pub node_name: String,
    pub script_asset: String,
    pub entry: String,
    pub frame_phase: String,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticAnchor {
    pub node_id: Option<NodeId>,
    pub object_id: Option<u64>,
    pub message: String,
    pub severity: DiagnosticSeverity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuResourceLifetime {
    pub resource: String,
    pub first_pass: usize,
    pub last_pass: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutablePassResource {
    pub resource: String,
    pub access: GpuResourceAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutablePassDescriptor {
    pub index: usize,
    pub node_id: NodeId,
    pub label: String,
    pub is_compute: bool,
    pub dependencies: Vec<usize>,
    pub resources: Vec<ExecutablePassResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExecutableRenderPlan {
    pub passes: Vec<ExecutablePassDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompiledGraphArtifact {
    pub node_order: Vec<NodeId>,
    pub ecs_jobs: Vec<EcsJobDescriptor>,
    pub gpu_passes: Vec<GpuPassDescriptor>,
    pub script_jobs: Vec<ScriptJobDescriptor>,
    pub execution_plan: ExecutableRenderPlan,
    pub render_graph: RenderGraph,
    pub resource_lifetimes: Vec<GpuResourceLifetime>,
    pub diagnostics: Vec<CompileDiagnostic>,
    pub diagnostic_anchors: Vec<DiagnosticAnchor>,
}

#[derive(Debug, Error)]
pub enum NodeCompileError {
    #[error("graph version {0} is newer than supported version {CURRENT_GRAPH_VERSION}")]
    UnsupportedFutureVersion(u32),

    #[error("duplicate node id found: {0}")]
    DuplicateNodeId(NodeId),

    #[error("node {node_id} references missing dependency {dependency_id}")]
    MissingDependency {
        node_id: NodeId,
        dependency_id: NodeId,
    },

    #[error("node graph contains a cycle")]
    CycleDetected,

    #[error("strict gpu mode failed for node {node_id}: {reason}")]
    StrictGpuUnsupported { node_id: NodeId, reason: String },

    #[error("RON serialization error: {0}")]
    RonSerialization(String),

    #[error("RON deserialization error: {0}")]
    RonDeserialization(String),
}

pub fn serialize_graph_ron(graph: &NodeGraph) -> Result<String, NodeCompileError> {
    ron::to_string(graph).map_err(|err| NodeCompileError::RonSerialization(err.to_string()))
}

pub fn deserialize_graph_ron(input: &str) -> Result<NodeGraph, NodeCompileError> {
    let graph = ron::from_str::<NodeGraph>(input)
        .map_err(|err| NodeCompileError::RonDeserialization(err.to_string()))?;
    migrate_graph(graph)
}

pub fn migrate_graph(mut graph: NodeGraph) -> Result<NodeGraph, NodeCompileError> {
    if graph.version > CURRENT_GRAPH_VERSION {
        return Err(NodeCompileError::UnsupportedFutureVersion(graph.version));
    }

    if graph.version < CURRENT_GRAPH_VERSION {
        graph.version = CURRENT_GRAPH_VERSION;
    }

    Ok(graph)
}

pub fn validate_graph(graph: &NodeGraph) -> Result<(), NodeCompileError> {
    if graph.version > CURRENT_GRAPH_VERSION {
        return Err(NodeCompileError::UnsupportedFutureVersion(graph.version));
    }

    let mut seen = BTreeSet::new();
    let node_index: HashMap<NodeId, &Node> =
        graph.nodes.iter().map(|node| (node.id, node)).collect();

    for node in &graph.nodes {
        if !seen.insert(node.id) {
            return Err(NodeCompileError::DuplicateNodeId(node.id));
        }

        for dependency in &node.dependencies {
            if !node_index.contains_key(dependency) {
                return Err(NodeCompileError::MissingDependency {
                    node_id: node.id,
                    dependency_id: *dependency,
                });
            }
        }
    }

    topological_sort(graph).map(|_| ())
}

pub fn topological_sort(graph: &NodeGraph) -> Result<Vec<NodeId>, NodeCompileError> {
    let mut in_degree: HashMap<NodeId, usize> = HashMap::new();
    let mut outgoing: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

    for node in &graph.nodes {
        in_degree.entry(node.id).or_insert(0);
        outgoing.entry(node.id).or_default();
    }

    for node in &graph.nodes {
        for dep in &node.dependencies {
            *in_degree.entry(node.id).or_insert(0) += 1;
            outgoing.entry(*dep).or_default().push(node.id);
        }
    }

    let mut ready: BTreeSet<NodeId> = in_degree
        .iter()
        .filter_map(|(node_id, degree)| if *degree == 0 { Some(*node_id) } else { None })
        .collect();

    let mut ordered = Vec::with_capacity(graph.nodes.len());

    while let Some(&next) = ready.iter().next() {
        ready.remove(&next);
        ordered.push(next);

        if let Some(children) = outgoing.get(&next) {
            let mut sorted_children = children.clone();
            sorted_children.sort_unstable();

            for child in sorted_children {
                if let Some(entry) = in_degree.get_mut(&child) {
                    *entry -= 1;
                    if *entry == 0 {
                        ready.insert(child);
                    }
                }
            }
        }
    }

    if ordered.len() != graph.nodes.len() {
        return Err(NodeCompileError::CycleDetected);
    }

    Ok(ordered)
}

pub fn compile_graph(
    graph: &NodeGraph,
    options: &NodeCompileOptions,
    backend: BackendKind,
    capabilities: BackendCapabilities,
) -> Result<CompiledGraphArtifact, NodeCompileError> {
    let graph = migrate_graph(graph.clone())?;
    validate_graph(&graph)?;

    let node_order = topological_sort(&graph)?;
    let node_index: HashMap<NodeId, &Node> =
        graph.nodes.iter().map(|node| (node.id, node)).collect();

    let mut diagnostics = Vec::new();
    let mut diagnostic_anchors = Vec::new();
    let mut ecs_jobs = Vec::new();
    let mut gpu_passes = Vec::new();
    let mut script_jobs = Vec::new();
    let mut execution_plan = ExecutableRenderPlan::default();
    let mut render_graph = RenderGraph::empty();
    let mut resource_usage: HashMap<String, (usize, usize)> = HashMap::new();
    let mut node_to_gpu_pass: HashMap<NodeId, usize> = HashMap::new();

    for node_id in &node_order {
        let node = node_index
            .get(node_id)
            .expect("node id from topo sort must exist");

        let execution = resolve_execution_target(node, options, capabilities, &mut diagnostics)?;
        let phase = phase_for_node_kind(node.kind).to_string();

        ecs_jobs.push(EcsJobDescriptor {
            node_id: node.id,
            node_name: node.name.clone(),
            phase,
            execution,
            payload_hint: payload_hint(node),
        });

        if node.kind == NodeKind::ScriptBehavior {
            let payload = script_payload(node);
            script_jobs.push(ScriptJobDescriptor {
                node_id: node.id,
                node_name: node.name.clone(),
                script_asset: payload.script_asset,
                entry: payload.entry,
                frame_phase: payload.frame_phase,
                settings: node.settings.clone(),
            });
        }

        if node.kind == NodeKind::Custom {
            let payload = custom_script_payload(node);
            script_jobs.push(ScriptJobDescriptor {
                node_id: node.id,
                node_name: node.name.clone(),
                script_asset: payload.script_asset,
                entry: payload.entry,
                frame_phase: payload.frame_phase,
                settings: node.settings.clone(),
            });
        }

        if execution != ResolvedExecutionTarget::Gpu {
            continue;
        }

        let current_pass_index = render_graph.passes.len();
        node_to_gpu_pass.insert(node.id, current_pass_index);

        let mut resources = effective_resource_states(node);
        let dependencies = node
            .dependencies
            .iter()
            .filter_map(|dep| node_to_gpu_pass.get(dep).copied())
            .collect::<Vec<_>>();

        match node.kind {
            NodeKind::RenderPass | NodeKind::BuildExport => {
                let render_payload = render_pass_payload(node);
                let target_name = render_payload.target_resource.clone();

                let target_handle = RenderTargetHandle(node.id);
                ensure_resource(
                    &mut render_graph.resources,
                    GraphResourceDescriptor {
                        name: target_name.clone(),
                        kind: GraphResourceKind::RenderTarget(RenderTargetDescriptor {
                            width: render_payload.target_width,
                            height: render_payload.target_height,
                        }),
                        lifetime: GraphResourceLifetime::Transient,
                    },
                );
                track_usage(&mut resource_usage, &target_name, current_pass_index);
                resources.push(ExecutablePassResource {
                    resource: target_name,
                    access: GpuResourceAccess::Write,
                });

                for binding in &node.gpu_bindings {
                    ensure_resource(
                        &mut render_graph.resources,
                        GraphResourceDescriptor {
                            name: binding.resource.clone(),
                            kind: GraphResourceKind::StorageBuffer { size_bytes: 4096 },
                            lifetime: GraphResourceLifetime::Persistent,
                        },
                    );
                    track_usage(&mut resource_usage, &binding.resource, current_pass_index);
                }

                let mut sprites = make_placeholder_sprites(render_payload.sprite_count);
                if options.optimize {
                    sprites.sort_by_key(|sprite| sprite.texture.0);
                }

                let batch = SpriteBatchCommand {
                    label: format!("{}::sprites", node.name),
                    blend: parse_blend_mode(Some(&render_payload.blend)),
                    target: Some(target_handle),
                    sprites,
                };

                render_graph
                    .passes
                    .push(RenderGraphPass::Render(RenderPassNode {
                        label: node.name.clone(),
                        camera: Camera2d::default(),
                        target: Some(target_handle),
                        batches: vec![batch],
                    }));

                gpu_passes.push(GpuPassDescriptor {
                    node_id: node.id,
                    backend,
                    label: node.name.clone(),
                    is_compute: false,
                    shader_entry: node
                        .shader_entry
                        .clone()
                        .unwrap_or_else(|| "vs_main".to_string()),
                    shader_profile: node
                        .shader_profile
                        .clone()
                        .unwrap_or_else(|| default_shader_profile(backend, false).to_string()),
                });
            }
            NodeKind::ComputePass => {
                let compute_payload = compute_pass_payload(node);
                let dispatch = if let Some(config) = node.compute {
                    [config.x, config.y, config.z]
                } else {
                    compute_payload.dispatch
                };
                let shader = compute_payload.shader;
                let reads = compute_payload.reads;
                let writes = compute_payload.writes;

                for resource in reads.iter().chain(writes.iter()) {
                    ensure_resource(
                        &mut render_graph.resources,
                        GraphResourceDescriptor {
                            name: resource.clone(),
                            kind: GraphResourceKind::StorageBuffer { size_bytes: 4096 },
                            lifetime: GraphResourceLifetime::Persistent,
                        },
                    );
                    track_usage(&mut resource_usage, resource, current_pass_index);
                }

                for resource in &reads {
                    resources.push(ExecutablePassResource {
                        resource: resource.clone(),
                        access: GpuResourceAccess::Read,
                    });
                }
                for resource in &writes {
                    resources.push(ExecutablePassResource {
                        resource: resource.clone(),
                        access: GpuResourceAccess::Write,
                    });
                }

                render_graph.passes.push(RenderGraphPass::Compute(
                    engine_render_api::ComputeDispatchNode {
                        label: node.name.clone(),
                        shader,
                        dispatch,
                        reads,
                        writes,
                    },
                ));

                gpu_passes.push(GpuPassDescriptor {
                    node_id: node.id,
                    backend,
                    label: node.name.clone(),
                    is_compute: true,
                    shader_entry: node
                        .shader_entry
                        .clone()
                        .unwrap_or_else(|| "cs_main".to_string()),
                    shader_profile: node
                        .shader_profile
                        .clone()
                        .unwrap_or_else(|| default_shader_profile(backend, true).to_string()),
                });
            }
            _ => {}
        }

        execution_plan.passes.push(ExecutablePassDescriptor {
            index: current_pass_index,
            node_id: node.id,
            label: node.name.clone(),
            is_compute: matches!(node.kind, NodeKind::ComputePass),
            dependencies,
            resources,
        });
    }

    let mut resource_lifetimes = resource_usage
        .into_iter()
        .map(|(resource, (first_pass, last_pass))| GpuResourceLifetime {
            resource,
            first_pass,
            last_pass,
        })
        .collect::<Vec<_>>();

    resource_lifetimes.sort_by(|a, b| a.resource.cmp(&b.resource));

    for diagnostic in &diagnostics {
        diagnostic_anchors.push(DiagnosticAnchor {
            node_id: diagnostic.node_id,
            object_id: None,
            message: diagnostic.message.clone(),
            severity: diagnostic.severity,
        });
    }

    Ok(CompiledGraphArtifact {
        node_order,
        ecs_jobs,
        gpu_passes,
        script_jobs,
        execution_plan,
        render_graph,
        resource_lifetimes,
        diagnostics,
        diagnostic_anchors,
    })
}

fn resolve_execution_target(
    node: &Node,
    options: &NodeCompileOptions,
    capabilities: BackendCapabilities,
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> Result<ResolvedExecutionTarget, NodeCompileError> {
    if options.force_cpu_fallback {
        return Ok(ResolvedExecutionTarget::Cpu);
    }

    if matches!(node.kind, NodeKind::ScriptBehavior | NodeKind::Custom) {
        match node.target {
            NodeExecutionTarget::Cpu => return Ok(ResolvedExecutionTarget::Cpu),
            NodeExecutionTarget::Hybrid => {
                diagnostics.push(CompileDiagnostic {
                    severity: DiagnosticSeverity::Info,
                    node_id: Some(node.id),
                    message: format!(
                        "{:?} executes on CPU even when Hybrid is selected",
                        node.kind
                    ),
                });
                return Ok(ResolvedExecutionTarget::Cpu);
            }
            NodeExecutionTarget::Gpu => {
                return fallback_for_unsupported_gpu(node, options, diagnostics);
            }
        }
    }

    let gpu_supported = capabilities.gpu_nodes;
    let compute_supported = capabilities.compute_nodes;

    let wants_compute = matches!(node.kind, NodeKind::ComputePass);

    match node.target {
        NodeExecutionTarget::Cpu => Ok(ResolvedExecutionTarget::Cpu),
        NodeExecutionTarget::Gpu => {
            if gpu_supported && (!wants_compute || compute_supported) {
                Ok(ResolvedExecutionTarget::Gpu)
            } else {
                fallback_for_unsupported_gpu(node, options, diagnostics)
            }
        }
        NodeExecutionTarget::Hybrid => {
            if options.allow_gpu_for_hybrid
                && capabilities.gpu_nodes
                && capabilities.hybrid_nodes
                && (!wants_compute || capabilities.compute_nodes)
            {
                Ok(ResolvedExecutionTarget::Gpu)
            } else {
                diagnostics.push(CompileDiagnostic {
                    severity: DiagnosticSeverity::Info,
                    node_id: Some(node.id),
                    message: "Hybrid execution lowered to CPU based on backend or compile options"
                        .to_string(),
                });
                Ok(ResolvedExecutionTarget::Cpu)
            }
        }
    }
}

fn fallback_for_unsupported_gpu(
    node: &Node,
    options: &NodeCompileOptions,
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> Result<ResolvedExecutionTarget, NodeCompileError> {
    let reason = format!(
        "GPU execution unsupported for node '{}' ({:?})",
        node.name, node.kind
    );

    match node.fallback_policy {
        NodeFallbackPolicy::Error => {
            diagnostics.push(CompileDiagnostic {
                severity: DiagnosticSeverity::Error,
                node_id: Some(node.id),
                message: reason.clone(),
            });
            if options.strict_gpu {
                return Err(NodeCompileError::StrictGpuUnsupported {
                    node_id: node.id,
                    reason,
                });
            }
            Ok(ResolvedExecutionTarget::Cpu)
        }
        NodeFallbackPolicy::Cpu => {
            diagnostics.push(CompileDiagnostic {
                severity: DiagnosticSeverity::Warning,
                node_id: Some(node.id),
                message: format!("{reason}; falling back to CPU"),
            });
            Ok(ResolvedExecutionTarget::Cpu)
        }
        NodeFallbackPolicy::Disable => {
            diagnostics.push(CompileDiagnostic {
                severity: DiagnosticSeverity::Warning,
                node_id: Some(node.id),
                message: format!("{reason}; node marked as disabled and lowered to CPU no-op"),
            });
            Ok(ResolvedExecutionTarget::Cpu)
        }
    }
}

fn phase_for_node_kind(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::GameplayFlow
        | NodeKind::GameplayEvent
        | NodeKind::ScriptBehavior
        | NodeKind::ObjectInitializer
        | NodeKind::Custom => "gameplay",
        NodeKind::MathState => "math",
        NodeKind::RenderPass | NodeKind::ComputePass => "render",
        NodeKind::AssetReference => "asset",
        NodeKind::BuildExport => "build",
    }
}

fn payload_hint(node: &Node) -> String {
    match &node.payload {
        Some(NodePayload::GameplayEvent(payload)) => {
            format!("event:{}", payload.event_name)
        }
        Some(NodePayload::GameplayFlow(payload)) => {
            format!("flow:{}={}", payload.condition_key, payload.expected_value)
        }
        Some(NodePayload::MathState(payload)) => {
            format!(
                "math:{}({}, {})",
                payload.operation, payload.lhs, payload.rhs
            )
        }
        Some(NodePayload::ScriptBehavior(payload)) => {
            format!("script:{}::{}", payload.script_asset, payload.entry)
        }
        Some(NodePayload::ObjectInitializer(payload)) => {
            format!("object:{}@{}", payload.object_name, payload.layer_id)
        }
        Some(NodePayload::RenderPass(payload)) => {
            format!(
                "render:{}:{}x{}:{}",
                payload.target_resource,
                payload.target_width,
                payload.target_height,
                payload.sprite_count
            )
        }
        Some(NodePayload::ComputePass(payload)) => {
            format!(
                "compute:{}:{}x{}x{}",
                payload.shader, payload.dispatch[0], payload.dispatch[1], payload.dispatch[2]
            )
        }
        Some(NodePayload::AssetReference(payload)) => {
            format!("asset:{}:{}", payload.asset_kind, payload.asset_path)
        }
        Some(NodePayload::BuildExport(payload)) => {
            format!("export:{}", payload.target)
        }
        Some(NodePayload::Custom(payload)) => {
            format!("custom:{}:{}", payload.type_name, payload.config_path)
        }
        None => "legacy_settings".to_string(),
    }
}

fn script_payload(node: &Node) -> ScriptBehaviorPayload {
    let mut payload = node
        .payload
        .as_ref()
        .and_then(|payload| match payload {
            NodePayload::ScriptBehavior(payload) => Some(payload.clone()),
            _ => None,
        })
        .unwrap_or_default();

    if let Some(script_asset) = node.settings.get("script_asset") {
        payload.script_asset = script_asset.clone();
    }
    if let Some(entry) = node.settings.get("script_entry") {
        payload.entry = entry.clone();
    }
    if let Some(frame_phase) = node.settings.get("script_phase") {
        payload.frame_phase = frame_phase.clone();
    }

    payload
}

fn custom_script_payload(node: &Node) -> ScriptBehaviorPayload {
    let mut payload = ScriptBehaviorPayload::default();

    if let Some(NodePayload::Custom(custom_payload)) = node.payload.as_ref() {
        if let Some(script_asset) = custom_payload.impl_path.as_ref() {
            payload.script_asset = script_asset.clone();
        }
    }

    if let Some(script_asset) = node.settings.get("impl_path") {
        payload.script_asset = script_asset.clone();
    } else if let Some(script_asset) = node.settings.get("script_asset") {
        payload.script_asset = script_asset.clone();
    }

    if let Some(entry) = node.settings.get("script_entry") {
        payload.entry = entry.clone();
    } else {
        payload.entry = "execute".to_string();
    }

    if let Some(frame_phase) = node.settings.get("script_phase") {
        payload.frame_phase = frame_phase.clone();
    }

    payload
}

fn render_pass_payload(node: &Node) -> RenderPassPayload {
    let mut payload = node
        .payload
        .as_ref()
        .and_then(|payload| match payload {
            NodePayload::RenderPass(payload) => Some(payload.clone()),
            _ => None,
        })
        .unwrap_or_default();

    if let Some(target_resource) = node.settings.get("target_resource") {
        payload.target_resource = target_resource.clone();
    }
    payload.target_width = parse_u32_setting(node, "target_width", payload.target_width);
    payload.target_height = parse_u32_setting(node, "target_height", payload.target_height);
    payload.sprite_count = parse_u32_setting(node, "sprite_count", payload.sprite_count);
    if let Some(blend) = node.settings.get("blend") {
        payload.blend = blend.clone();
    }

    payload
}

fn compute_pass_payload(node: &Node) -> ComputePassPayload {
    let mut payload = node
        .payload
        .as_ref()
        .and_then(|payload| match payload {
            NodePayload::ComputePass(payload) => Some(payload.clone()),
            _ => None,
        })
        .unwrap_or_default();

    if let Some(shader) = node.settings.get("shader") {
        payload.shader = shader.clone();
    }
    payload.dispatch = node
        .compute
        .map(|dispatch| [dispatch.x, dispatch.y, dispatch.z])
        .unwrap_or(payload.dispatch);
    let reads = parse_csv(node.settings.get("read_resources"));
    if !reads.is_empty() {
        payload.reads = reads;
    }
    let writes = parse_csv(node.settings.get("write_resources"));
    if !writes.is_empty() {
        payload.writes = writes;
    }

    payload
}

fn parse_u32_setting(node: &Node, key: &str, default_value: u32) -> u32 {
    node.settings
        .get(key)
        .and_then(|raw| raw.parse::<u32>().ok())
        .unwrap_or(default_value)
}

fn parse_blend_mode(raw: Option<&String>) -> BlendMode {
    match raw.map(|value| value.to_ascii_lowercase()) {
        Some(value) if value == "additive" => BlendMode::Additive,
        Some(value) if value == "multiply" => BlendMode::Multiply,
        _ => BlendMode::Alpha,
    }
}

fn parse_csv(raw: Option<&String>) -> Vec<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(ToString::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn ensure_resource(
    resources: &mut Vec<GraphResourceDescriptor>,
    descriptor: GraphResourceDescriptor,
) {
    if resources
        .iter()
        .all(|resource| resource.name != descriptor.name)
    {
        resources.push(descriptor);
    }
}

fn track_usage(
    resource_usage: &mut HashMap<String, (usize, usize)>,
    resource: &str,
    pass_index: usize,
) {
    resource_usage
        .entry(resource.to_string())
        .and_modify(|bounds| {
            bounds.0 = bounds.0.min(pass_index);
            bounds.1 = bounds.1.max(pass_index);
        })
        .or_insert((pass_index, pass_index));
}

fn make_placeholder_sprites(count: u32) -> Vec<SpriteInstance> {
    let columns = (count.max(1) as f32).sqrt().ceil().max(1.0) as u32;
    let rows = count.div_ceil(columns);
    let spacing = 72.0;

    (0..count)
        .map(|index| SpriteInstance {
            texture: TextureHandle((index % 4) as u64),
            x: ((index % columns) as f32 - (columns.saturating_sub(1) as f32 * 0.5)) * spacing,
            y: ((index / columns) as f32 - (rows.saturating_sub(1) as f32 * 0.5)) * spacing,
            width: 32.0,
            height: 32.0,
            rotation_radians: 0.0,
            tint: match index % 4 {
                0 => [0.95, 0.42, 0.40, 1.0],
                1 => [0.42, 0.75, 0.96, 1.0],
                2 => [0.48, 0.88, 0.58, 1.0],
                _ => [0.96, 0.82, 0.38, 1.0],
            },
        })
        .collect()
}

fn effective_resource_states(node: &Node) -> Vec<ExecutablePassResource> {
    if !node.gpu_resource_states.is_empty() {
        return node
            .gpu_resource_states
            .iter()
            .map(|state| ExecutablePassResource {
                resource: state.resource.clone(),
                access: state.access,
            })
            .collect();
    }

    node.gpu_bindings
        .iter()
        .map(|binding| ExecutablePassResource {
            resource: binding.resource.clone(),
            access: GpuResourceAccess::ReadWrite,
        })
        .collect()
}

fn default_shader_profile(backend: BackendKind, compute: bool) -> &'static str {
    match (backend, compute) {
        (BackendKind::Vulkan, true) => "spirv1.5",
        (BackendKind::Vulkan, false) => "spirv1.5",
        (BackendKind::Dx12, true) => "cs_6_6",
        (BackendKind::Dx12, false) => "ps_6_0",
        (BackendKind::Dx11, true) => "cs_5_0",
        (BackendKind::Dx11, false) => "ps_5_0",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_graph() -> NodeGraph {
        NodeGraph {
            version: CURRENT_GRAPH_VERSION,
            nodes: vec![
                Node {
                    id: 1,
                    name: "start".to_string(),
                    kind: NodeKind::GameplayEvent,
                    target: NodeExecutionTarget::Cpu,
                    dependencies: vec![],
                    settings: Default::default(),
                    gpu_bindings: vec![],
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![],
                    shader_entry: None,
                    shader_profile: None,
                    payload: Some(NodePayload::GameplayEvent(GameplayEventPayload::default())),
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
                    gpu_bindings: vec![NodeGpuBinding {
                        set: 0,
                        binding: 0,
                        resource: "particles_buffer".to_string(),
                    }],
                    compute: Some(ComputeDispatchConfig { x: 8, y: 8, z: 1 }),
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![NodeGpuResourceState {
                        resource: "particles_buffer".to_string(),
                        access: GpuResourceAccess::Write,
                    }],
                    shader_entry: Some("cs_main".to_string()),
                    shader_profile: Some("cs_6_6".to_string()),
                    payload: Some(NodePayload::ComputePass(ComputePassPayload {
                        shader: "particles.hlsl".to_string(),
                        dispatch: [8, 8, 1],
                        reads: Vec::new(),
                        writes: vec!["particles_buffer".to_string()],
                    })),
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
                    payload: Some(NodePayload::RenderPass(RenderPassPayload {
                        target_resource: "frame_color".to_string(),
                        target_width: 1920,
                        target_height: 1080,
                        sprite_count: 4,
                        blend: "alpha".to_string(),
                    })),
                },
            ],
        }
    }

    fn caps(gpu: bool, hybrid: bool, compute: bool) -> BackendCapabilities {
        BackendCapabilities {
            textured_sprites: true,
            batching: true,
            camera_transforms: true,
            blend_modes: true,
            offscreen_targets: true,
            texture_atlas: true,
            gpu_nodes: gpu,
            hybrid_nodes: hybrid,
            compute_nodes: compute,
            viewport_readback: true,
        }
    }

    #[test]
    fn validate_graph_rejects_cycles() {
        let graph = NodeGraph {
            version: CURRENT_GRAPH_VERSION,
            nodes: vec![
                Node {
                    id: 1,
                    name: "a".into(),
                    kind: NodeKind::GameplayFlow,
                    target: NodeExecutionTarget::Cpu,
                    dependencies: vec![2],
                    settings: Default::default(),
                    gpu_bindings: vec![],
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![],
                    shader_entry: None,
                    shader_profile: None,
                    payload: Some(NodePayload::GameplayFlow(GameplayFlowPayload::default())),
                },
                Node {
                    id: 2,
                    name: "b".into(),
                    kind: NodeKind::GameplayFlow,
                    target: NodeExecutionTarget::Cpu,
                    dependencies: vec![1],
                    settings: Default::default(),
                    gpu_bindings: vec![],
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![],
                    shader_entry: None,
                    shader_profile: None,
                    payload: Some(NodePayload::GameplayFlow(GameplayFlowPayload::default())),
                },
            ],
        };

        let error = validate_graph(&graph).expect_err("cycle should fail validation");
        assert!(matches!(error, NodeCompileError::CycleDetected));
    }

    #[test]
    fn compilation_is_deterministic() {
        let mut graph_a = sample_graph();
        let mut graph_b = sample_graph();
        graph_b.nodes.swap(0, 2);
        graph_a.nodes.swap(0, 1);

        let artifact_a = compile_graph(
            &graph_a,
            &NodeCompileOptions::default(),
            BackendKind::Vulkan,
            caps(true, true, true),
        )
        .expect("graph should compile");
        let artifact_b = compile_graph(
            &graph_b,
            &NodeCompileOptions::default(),
            BackendKind::Vulkan,
            caps(true, true, true),
        )
        .expect("graph should compile");

        assert_eq!(artifact_a.node_order, artifact_b.node_order);
        assert_eq!(artifact_a.ecs_jobs, artifact_b.ecs_jobs);
        assert_eq!(artifact_a.gpu_passes, artifact_b.gpu_passes);
        assert_eq!(artifact_a.execution_plan, artifact_b.execution_plan);
    }

    #[test]
    fn lowering_respects_cpu_gpu_hybrid_and_compute_support() {
        let graph = sample_graph();

        let gpu_artifact = compile_graph(
            &graph,
            &NodeCompileOptions::default(),
            BackendKind::Dx12,
            caps(true, true, true),
        )
        .expect("graph should compile");

        assert_eq!(
            gpu_artifact.ecs_jobs[0].execution,
            ResolvedExecutionTarget::Cpu
        );
        assert_eq!(
            gpu_artifact.ecs_jobs[1].execution,
            ResolvedExecutionTarget::Gpu
        );
        assert_eq!(
            gpu_artifact.ecs_jobs[2].execution,
            ResolvedExecutionTarget::Gpu
        );
        assert_eq!(gpu_artifact.gpu_passes.len(), 2);

        let cpu_artifact = compile_graph(
            &graph,
            &NodeCompileOptions::default(),
            BackendKind::Dx11,
            caps(false, false, false),
        )
        .expect("graph should compile");

        assert!(cpu_artifact
            .ecs_jobs
            .iter()
            .all(|job| job.execution == ResolvedExecutionTarget::Cpu));
        assert!(!cpu_artifact.diagnostics.is_empty());
        assert!(cpu_artifact.gpu_passes.is_empty());
    }

    #[test]
    fn graph_ron_roundtrip_is_stable() {
        let graph = sample_graph();
        let encoded = serialize_graph_ron(&graph).expect("serialization should work");
        let decoded = deserialize_graph_ron(&encoded).expect("deserialization should work");

        assert_eq!(graph, decoded);
    }

    #[test]
    fn compute_resources_generate_lifetimes_and_plan_deps() {
        let graph = sample_graph();
        let artifact = compile_graph(
            &graph,
            &NodeCompileOptions::default(),
            BackendKind::Vulkan,
            caps(true, true, true),
        )
        .expect("graph should compile");

        assert!(artifact
            .resource_lifetimes
            .iter()
            .any(|lifetime| lifetime.resource == "particles_buffer"));
        assert!(!artifact.render_graph.passes.is_empty());
        assert_eq!(
            artifact.execution_plan.passes.len(),
            artifact.gpu_passes.len()
        );
        assert!(artifact.execution_plan.passes[1].dependencies.contains(&0));
    }
}
