use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use egui_snarl::ui::{PinInfo, SnarlStyle, SnarlViewer};
use egui_snarl::{InPin, NodeId as SnarlNodeId, OutPin, Snarl};
use engine_assets::{infer_asset_kind, load_node_graph, save_node_graph, AssetKind};
use engine_nodes::{
    CompileDiagnostic, Node, NodeExecutionTarget, NodeFallbackPolicy, NodeGraph, NodeId, NodeKind,
    CURRENT_GRAPH_VERSION,
};
use engine_render_api::{
    BackendCapabilities, BackendDiagnosticLevel, BackendDiagnostics, BackendKind,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct FrameTimings {
    pub cpu_frame_ms: f32,
    pub gpu_frame_ms: f32,
    pub node_compile_ms: f32,
}

impl Default for FrameTimings {
    fn default() -> Self {
        Self {
            cpu_frame_ms: 0.0,
            gpu_frame_ms: 0.0,
            node_compile_ms: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EditorState {
    pub is_playing: bool,
    pub graph_dirty: bool,
    pub selected_asset: Option<String>,
    pub last_fallback_count: usize,
}

impl EditorState {
    pub fn mark_graph_dirty(&mut self) {
        self.graph_dirty = true;
    }

    pub fn mark_graph_compiled(&mut self, fallback_count: usize) {
        self.graph_dirty = false;
        self.last_fallback_count = fallback_count;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinTypeCategory {
    Flow,
    Data,
    Texture,
    Buffer,
    Audio,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EditorWorkspaceMode {
    #[default]
    Gameplay,
    Render,
}

const GAMEPLAY_WORKSPACE_KINDS: &[NodeKind] = &[
    NodeKind::GameplayEvent,
    NodeKind::GameplayFlow,
    NodeKind::MathState,
    NodeKind::ScriptBehavior,
    NodeKind::ObjectInitializer,
    NodeKind::AssetReference,
];

const RENDER_WORKSPACE_KINDS: &[NodeKind] = &[
    NodeKind::ComputePass,
    NodeKind::RenderPass,
    NodeKind::AssetReference,
    NodeKind::BuildExport,
];

pub fn workspace_label(mode: EditorWorkspaceMode) -> &'static str {
    match mode {
        EditorWorkspaceMode::Gameplay => "Gameplay / Script",
        EditorWorkspaceMode::Render => "Render Pipeline",
    }
}

pub fn workspace_kinds(mode: EditorWorkspaceMode) -> &'static [NodeKind] {
    match mode {
        EditorWorkspaceMode::Gameplay => GAMEPLAY_WORKSPACE_KINDS,
        EditorWorkspaceMode::Render => RENDER_WORKSPACE_KINDS,
    }
}

pub fn node_workspace(kind: NodeKind) -> EditorWorkspaceMode {
    match kind {
        NodeKind::ComputePass | NodeKind::RenderPass | NodeKind::BuildExport => {
            EditorWorkspaceMode::Render
        }
        NodeKind::GameplayFlow
        | NodeKind::GameplayEvent
        | NodeKind::MathState
        | NodeKind::ScriptBehavior
        | NodeKind::ObjectInitializer
        | NodeKind::AssetReference => EditorWorkspaceMode::Gameplay,
    }
}

pub fn pin_types_compatible(output: PinTypeCategory, input: PinTypeCategory) -> bool {
    matches!(
        (output, input),
        (PinTypeCategory::Flow, PinTypeCategory::Flow)
            | (PinTypeCategory::Event, PinTypeCategory::Event)
            | (PinTypeCategory::Data, PinTypeCategory::Data)
            | (PinTypeCategory::Texture, PinTypeCategory::Texture)
            | (PinTypeCategory::Buffer, PinTypeCategory::Buffer)
            | (PinTypeCategory::Audio, PinTypeCategory::Audio)
            | (PinTypeCategory::Texture, PinTypeCategory::Data)
            | (PinTypeCategory::Buffer, PinTypeCategory::Data)
            | (PinTypeCategory::Data, PinTypeCategory::Buffer)
    )
}

pub fn node_output_pin_type(kind: NodeKind) -> PinTypeCategory {
    match kind {
        NodeKind::GameplayFlow => PinTypeCategory::Flow,
        NodeKind::GameplayEvent => PinTypeCategory::Event,
        NodeKind::MathState => PinTypeCategory::Data,
        NodeKind::ScriptBehavior => PinTypeCategory::Flow,
        NodeKind::ObjectInitializer => PinTypeCategory::Data,
        NodeKind::RenderPass | NodeKind::BuildExport => PinTypeCategory::Texture,
        NodeKind::ComputePass => PinTypeCategory::Buffer,
        NodeKind::AssetReference => PinTypeCategory::Data,
    }
}

pub fn node_input_pin_type(kind: NodeKind) -> PinTypeCategory {
    match kind {
        NodeKind::GameplayFlow => PinTypeCategory::Flow,
        NodeKind::GameplayEvent => PinTypeCategory::Event,
        NodeKind::MathState => PinTypeCategory::Data,
        NodeKind::ScriptBehavior => PinTypeCategory::Flow,
        NodeKind::ObjectInitializer => PinTypeCategory::Data,
        NodeKind::RenderPass | NodeKind::BuildExport => PinTypeCategory::Texture,
        NodeKind::ComputePass => PinTypeCategory::Buffer,
        NodeKind::AssetReference => PinTypeCategory::Data,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EditorDocument {
    pub graph: NodeGraph,
    pub node_positions: BTreeMap<NodeId, [f32; 2]>,
}

impl EditorDocument {
    pub fn from_graph(graph: NodeGraph) -> Self {
        let mut node_positions = BTreeMap::new();
        for (index, node) in graph.nodes.iter().enumerate() {
            let x = 80.0 + (index as f32 % 6.0) * 220.0;
            let y = 80.0 + (index as f32 / 6.0).floor() * 140.0;
            node_positions.insert(node.id, [x, y]);
        }

        Self {
            graph,
            node_positions,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectAssetEntry {
    pub path: PathBuf,
    pub kind: EditorAssetKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EditorAssetKind {
    Texture,
    Audio,
    Shape,
    Graph,
    Shader,
    Unknown,
}

impl From<AssetKind> for EditorAssetKind {
    fn from(value: AssetKind) -> Self {
        match value {
            AssetKind::Texture => Self::Texture,
            AssetKind::Audio => Self::Audio,
            AssetKind::Shape => Self::Shape,
            AssetKind::Graph => Self::Graph,
            AssetKind::Shader => Self::Shader,
            AssetKind::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EditorViewportState {
    pub pan: [f32; 2],
    pub zoom: f32,
    pub selected_object: Option<u64>,
}

impl Default for EditorViewportState {
    fn default() -> Self {
        Self {
            pan: [0.0, 0.0],
            zoom: 1.0,
            selected_object: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionId(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EditorCommand {
    AddNode {
        node: Node,
        position: [f32; 2],
    },
    RemoveNode {
        node: Node,
        position: Option<[f32; 2]>,
    },
    ConnectNodes {
        from: NodeId,
        to: NodeId,
    },
    DisconnectNodes {
        from: NodeId,
        to: NodeId,
    },
    MoveNode {
        node_id: NodeId,
        from: [f32; 2],
        to: [f32; 2],
    },
    SetNodeKind {
        node_id: NodeId,
        old: NodeKind,
        new: NodeKind,
    },
    SetNodeTarget {
        node_id: NodeId,
        old: NodeExecutionTarget,
        new: NodeExecutionTarget,
    },
    SetNodeFallback {
        node_id: NodeId,
        old: NodeFallbackPolicy,
        new: NodeFallbackPolicy,
    },
    SetNodeShaderMeta {
        node_id: NodeId,
        old_entry: Option<String>,
        new_entry: Option<String>,
        old_profile: Option<String>,
        new_profile: Option<String>,
    },
    SetNodeSetting {
        node_id: NodeId,
        key: String,
        old: Option<String>,
        new: Option<String>,
    },
    Batch {
        commands: Vec<EditorCommand>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryNode {
    pub id: u64,
    pub parent: Option<u64>,
    pub children: Vec<u64>,
    pub transaction: TransactionId,
    pub label: String,
    pub command: Option<EditorCommand>,
    pub document: EditorDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryGraph {
    pub nodes: BTreeMap<u64, HistoryNode>,
    pub root: u64,
    pub current: u64,
    pub next_id: u64,
    pub next_transaction: u64,
    pub active_transaction: Option<TransactionId>,
}

impl HistoryGraph {
    pub fn new(initial_document: EditorDocument) -> Self {
        let root_id = 1_u64;
        let root_tx = TransactionId(1);
        let root = HistoryNode {
            id: root_id,
            parent: None,
            children: Vec::new(),
            transaction: root_tx,
            label: "root".to_string(),
            command: None,
            document: initial_document,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(root_id, root);

        Self {
            nodes,
            root: root_id,
            current: root_id,
            next_id: root_id + 1,
            next_transaction: 2,
            active_transaction: None,
        }
    }

    pub fn begin_transaction(&mut self) -> TransactionId {
        let tx = TransactionId(self.next_transaction);
        self.next_transaction += 1;
        self.active_transaction = Some(tx);
        tx
    }

    pub fn end_transaction(&mut self) {
        self.active_transaction = None;
    }

    pub fn record(
        &mut self,
        label: impl Into<String>,
        command: EditorCommand,
        document: EditorDocument,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let tx = self.active_transaction.unwrap_or_else(|| {
            let tx = TransactionId(self.next_transaction);
            self.next_transaction += 1;
            tx
        });

        let parent = self.current;
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            parent_node.children.push(id);
        }

        let history_node = HistoryNode {
            id,
            parent: Some(parent),
            children: Vec::new(),
            transaction: tx,
            label: label.into(),
            command: Some(command),
            document,
        };

        self.nodes.insert(id, history_node);
        self.current = id;
        id
    }

    pub fn current_document(&self) -> Option<&EditorDocument> {
        self.nodes.get(&self.current).map(|node| &node.document)
    }

    pub fn checkout(&mut self, id: u64) -> Option<EditorDocument> {
        let node = self.nodes.get(&id)?;
        self.current = id;
        Some(node.document.clone())
    }

    pub fn undo(&mut self) -> Option<EditorDocument> {
        let current = self.nodes.get(&self.current)?;
        let parent = current.parent?;
        self.current = parent;
        self.nodes.get(&parent).map(|node| node.document.clone())
    }

    pub fn redo_latest(&mut self) -> Option<EditorDocument> {
        let current = self.nodes.get(&self.current)?;
        let child = *current.children.last()?;
        self.current = child;
        self.nodes.get(&child).map(|node| node.document.clone())
    }

    pub fn replay_matches_snapshot(&self, target: u64) -> bool {
        let Some(target_node) = self.nodes.get(&target) else {
            return false;
        };

        let mut path = Vec::new();
        let mut cursor = Some(target_node.id);
        while let Some(id) = cursor {
            let node = match self.nodes.get(&id) {
                Some(node) => node,
                None => return false,
            };
            path.push(id);
            cursor = node.parent;
        }
        path.reverse();

        let Some(root_node) = self.nodes.get(&self.root) else {
            return false;
        };
        let mut replay = root_node.document.clone();

        for id in path.into_iter().skip(1) {
            let Some(node) = self.nodes.get(&id) else {
                return false;
            };
            if let Some(command) = &node.command {
                if apply_command_internal(&mut replay, command).is_err() {
                    return false;
                }
            }
        }

        replay == target_node.document
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EditorSessionState {
    pub selected_node: Option<NodeId>,
    pub selected_asset: Option<PathBuf>,
    pub recent_projects: Vec<PathBuf>,
    pub viewport: EditorViewportState,
    pub history: HistoryGraph,
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub workspace_mode: EditorWorkspaceMode,
}

#[derive(Debug, Error)]
pub enum EditorError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("asset error: {0}")]
    Asset(#[from] engine_assets::AssetError),

    #[error("session parse error: {0}")]
    SessionParse(String),

    #[error("session encode error: {0}")]
    SessionEncode(String),

    #[error("editor command failed: {0}")]
    Command(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EditorProjectState {
    pub project_root: PathBuf,
    pub scene_path: PathBuf,
    pub document: EditorDocument,
    pub dirty: bool,
    pub asset_index: Vec<ProjectAssetEntry>,
    pub session: EditorSessionState,
    pub next_node_id: NodeId,
}

impl EditorProjectState {
    pub fn open(project_root: impl AsRef<Path>, scene_override: Option<PathBuf>) -> Result<Self, EditorError> {
        let project_root = project_root.as_ref().to_path_buf();
        fs::create_dir_all(&project_root)?;

        let scene_path = scene_override.unwrap_or_else(|| project_root.join("assets/sample_scene.ron"));
        if !scene_path.exists() {
            if let Some(parent) = scene_path.parent() {
                fs::create_dir_all(parent)?;
            }
            save_node_graph(&scene_path, &NodeGraph::empty())?;
        }

        let graph = load_node_graph(&scene_path)?;
        let document = EditorDocument::from_graph(graph);
        let next_node_id = document
            .graph
            .nodes
            .iter()
            .map(|node| node.id)
            .max()
            .unwrap_or(0)
            + 1;

        let mut session = load_session(&project_root)?
            .unwrap_or_else(|| EditorSessionState {
                selected_node: None,
                selected_asset: None,
                recent_projects: vec![project_root.clone()],
                viewport: EditorViewportState::default(),
                history: HistoryGraph::new(document.clone()),
                diagnostics: Vec::new(),
                workspace_mode: EditorWorkspaceMode::default(),
            });

        if session.history.nodes.is_empty() {
            session.history = HistoryGraph::new(document.clone());
        }

        if !session.recent_projects.contains(&project_root) {
            session.recent_projects.insert(0, project_root.clone());
            if session.recent_projects.len() > 16 {
                session.recent_projects.truncate(16);
            }
        }

        let mut project = Self {
            project_root,
            scene_path,
            document,
            dirty: false,
            asset_index: Vec::new(),
            session,
            next_node_id,
        };

        project.refresh_asset_index()?;
        Ok(project)
    }

    pub fn save_scene(&mut self) -> Result<(), EditorError> {
        save_node_graph(&self.scene_path, &self.document.graph)?;
        self.dirty = false;
        self.persist_session()?;
        Ok(())
    }

    pub fn save_scene_as(&mut self, path: impl AsRef<Path>) -> Result<(), EditorError> {
        self.scene_path = path.as_ref().to_path_buf();
        save_node_graph(&self.scene_path, &self.document.graph)?;
        self.dirty = false;
        self.persist_session()?;
        Ok(())
    }

    pub fn autosave_if_dirty(&mut self) -> Result<(), EditorError> {
        if !self.dirty {
            return Ok(());
        }

        let autosave_path = self.project_root.join(".rusty_engine/editor_autosave.ron");
        if let Some(parent) = autosave_path.parent() {
            fs::create_dir_all(parent)?;
        }
        save_node_graph(autosave_path, &self.document.graph)?;
        Ok(())
    }

    pub fn refresh_asset_index(&mut self) -> Result<(), EditorError> {
        let mut files = Vec::new();
        let asset_root = self.project_root.join("assets");
        if asset_root.exists() {
            collect_files_recursive(&asset_root, &mut files)?;
        }

        if self.scene_path.exists() && !files.contains(&self.scene_path) {
            files.push(self.scene_path.clone());
        }
        files.sort();

        self.asset_index = files
            .into_iter()
            .map(|path| ProjectAssetEntry {
                kind: EditorAssetKind::from(infer_asset_kind(&path)),
                path,
            })
            .collect();

        Ok(())
    }

    pub fn begin_transaction(&mut self) -> TransactionId {
        self.session.history.begin_transaction()
    }

    pub fn end_transaction(&mut self) {
        self.session.history.end_transaction();
    }

    pub fn apply_command_batch(
        &mut self,
        label: impl Into<String>,
        commands: Vec<EditorCommand>,
    ) -> Result<(), EditorError> {
        if commands.is_empty() {
            return Ok(());
        }

        let mut updated = self.document.clone();
        for command in &commands {
            apply_command_internal(&mut updated, command)?;
        }

        self.document = updated.clone();
        self.dirty = true;

        self.session.history.record(
            label,
            EditorCommand::Batch { commands },
            updated,
        );

        self.persist_session()?;
        Ok(())
    }

    pub fn undo(&mut self) -> Result<bool, EditorError> {
        if let Some(document) = self.session.history.undo() {
            self.document = document;
            self.dirty = true;
            self.persist_session()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn redo(&mut self) -> Result<bool, EditorError> {
        if let Some(document) = self.session.history.redo_latest() {
            self.document = document;
            self.dirty = true;
            self.persist_session()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn checkout_history(&mut self, node_id: u64) -> Result<bool, EditorError> {
        if let Some(document) = self.session.history.checkout(node_id) {
            self.document = document;
            self.dirty = true;
            self.persist_session()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn replay_current_history(&self) -> bool {
        self.session
            .history
            .replay_matches_snapshot(self.session.history.current)
    }

    pub fn persist_session(&self) -> Result<(), EditorError> {
        let session_path = self.project_root.join(".rusty_engine/editor_session.ron");
        if let Some(parent) = session_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let encoded = ron::to_string(&self.session)
            .map_err(|err| EditorError::SessionEncode(err.to_string()))?;
        fs::write(session_path, encoded)?;
        Ok(())
    }

    pub fn remove_node_command(&self, node_id: NodeId) -> Option<EditorCommand> {
        let node = self.document.graph.nodes.iter().find(|node| node.id == node_id)?.clone();
        let position = self.document.node_positions.get(&node_id).copied();
        Some(EditorCommand::RemoveNode { node, position })
    }
}

fn apply_command_internal(document: &mut EditorDocument, command: &EditorCommand) -> Result<(), EditorError> {
    match command {
        EditorCommand::AddNode { node, position } => {
            if document.graph.nodes.iter().any(|existing| existing.id == node.id) {
                return Err(EditorError::Command(format!("duplicate node id {}", node.id)));
            }
            document.graph.nodes.push(node.clone());
            document.node_positions.insert(node.id, *position);
        }
        EditorCommand::RemoveNode { node, .. } => {
            document.graph.nodes.retain(|existing| existing.id != node.id);
            document.node_positions.remove(&node.id);
            for existing in &mut document.graph.nodes {
                existing.dependencies.retain(|dep| *dep != node.id);
            }
        }
        EditorCommand::ConnectNodes { from, to } => {
            if from == to {
                return Err(EditorError::Command("cannot connect node to itself".to_string()));
            }

            let out_node = document
                .graph
                .nodes
                .iter()
                .find(|node| node.id == *from)
                .ok_or_else(|| EditorError::Command(format!("missing source node {from}")))?;
            let in_node = document
                .graph
                .nodes
                .iter()
                .find(|node| node.id == *to)
                .ok_or_else(|| EditorError::Command(format!("missing target node {to}")))?;

            let out_kind = node_output_pin_type(out_node.kind);
            let in_kind = node_input_pin_type(in_node.kind);
            if !pin_types_compatible(out_kind, in_kind) {
                return Err(EditorError::Command(format!(
                    "incompatible pin connection {out_kind:?} -> {in_kind:?}"
                )));
            }

            let target = document
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == *to)
                .ok_or_else(|| EditorError::Command(format!("missing target node {to}")))?;
            if !target.dependencies.contains(from) {
                target.dependencies.push(*from);
                target.dependencies.sort_unstable();
            }
        }
        EditorCommand::DisconnectNodes { from, to } => {
            let target = document
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == *to)
                .ok_or_else(|| EditorError::Command(format!("missing target node {to}")))?;
            target.dependencies.retain(|dep| dep != from);
        }
        EditorCommand::MoveNode { node_id, to, .. } => {
            document.node_positions.insert(*node_id, *to);
        }
        EditorCommand::SetNodeKind { node_id, new, .. } => {
            let node = document
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == *node_id)
                .ok_or_else(|| EditorError::Command(format!("missing node {node_id}")))?;
            node.kind = *new;
        }
        EditorCommand::SetNodeTarget { node_id, new, .. } => {
            let node = document
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == *node_id)
                .ok_or_else(|| EditorError::Command(format!("missing node {node_id}")))?;
            node.target = *new;
        }
        EditorCommand::SetNodeFallback { node_id, new, .. } => {
            let node = document
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == *node_id)
                .ok_or_else(|| EditorError::Command(format!("missing node {node_id}")))?;
            node.fallback_policy = *new;
        }
        EditorCommand::SetNodeShaderMeta {
            node_id,
            new_entry,
            new_profile,
            ..
        } => {
            let node = document
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == *node_id)
                .ok_or_else(|| EditorError::Command(format!("missing node {node_id}")))?;
            node.shader_entry = new_entry.clone();
            node.shader_profile = new_profile.clone();
        }
        EditorCommand::SetNodeSetting {
            node_id,
            key,
            new,
            ..
        } => {
            let node = document
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == *node_id)
                .ok_or_else(|| EditorError::Command(format!("missing node {node_id}")))?;

            match new {
                Some(value) => {
                    node.settings.insert(key.clone(), value.clone());
                }
                None => {
                    node.settings.remove(key);
                }
            }
        }
        EditorCommand::Batch { commands } => {
            for nested in commands {
                apply_command_internal(document, nested)?;
            }
        }
    }

    document.graph.version = CURRENT_GRAPH_VERSION;
    Ok(())
}

fn load_session(project_root: &Path) -> Result<Option<EditorSessionState>, EditorError> {
    let path = project_root.join(".rusty_engine/editor_session.ron");
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)?;
    let parsed = ron::from_str::<EditorSessionState>(&raw)
        .map_err(|err| EditorError::SessionParse(err.to_string()))?;
    Ok(Some(parsed))
}

fn collect_files_recursive(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), EditorError> {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnarlVisualNode {
    pub node_id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub target: NodeExecutionTarget,
}

impl SnarlVisualNode {
    fn from_node(node: &Node) -> Self {
        Self {
            node_id: node.id,
            name: node.name.clone(),
            kind: node.kind,
            target: node.target,
        }
    }
}

#[derive(Debug, Default)]
pub struct GraphCanvasState {
    pub snarl: Snarl<SnarlVisualNode>,
    pub node_map: BTreeMap<NodeId, SnarlNodeId>,
    pub last_positions: BTreeMap<NodeId, [f32; 2]>,
}

#[derive(Debug, Default)]
pub struct GraphCanvasOutput {
    pub commands: Vec<EditorCommand>,
    pub selected_node: Option<NodeId>,
}

impl GraphCanvasState {
    pub fn rebuild_from_document(&mut self, document: &EditorDocument) {
        self.snarl = Snarl::new();
        self.node_map.clear();
        self.last_positions.clear();

        for node in &document.graph.nodes {
            let pos = document
                .node_positions
                .get(&node.id)
                .copied()
                .unwrap_or([80.0, 80.0]);
            let snarl_node = self
                .snarl
                .insert_node(egui::pos2(pos[0], pos[1]), SnarlVisualNode::from_node(node));
            self.node_map.insert(node.id, snarl_node);
            self.last_positions.insert(node.id, pos);
        }

        for node in &document.graph.nodes {
            let Some(target_id) = self.node_map.get(&node.id).copied() else {
                continue;
            };

            for dep in &node.dependencies {
                let Some(source_id) = self.node_map.get(dep).copied() else {
                    continue;
                };
                self.snarl.connect(
                    egui_snarl::OutPinId {
                        node: source_id,
                        output: 0,
                    },
                    egui_snarl::InPinId {
                        node: target_id,
                        input: 0,
                    },
                );
            }
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        next_node_id: &mut NodeId,
        selected: Option<NodeId>,
        workspace_mode: EditorWorkspaceMode,
        dragged_asset: Option<ProjectAssetEntry>,
    ) -> GraphCanvasOutput {
        let mut viewer = ProjectSnarlViewer {
            commands: Vec::new(),
            selected_node: selected,
            next_node_id,
            workspace_mode,
        };
        let style = SnarlStyle::new();
        self.snarl.show(&mut viewer, &style, "editor_graph_snarl", ui);

        let mut output = GraphCanvasOutput {
            commands: viewer.commands,
            selected_node: viewer.selected_node,
        };

        for (snarl_id, visual) in self.snarl.nodes_ids_data() {
            if let Some(node_info) = self.snarl.get_node_info(snarl_id) {
                let new_pos = [node_info.pos.x, node_info.pos.y];
                if let Some(old_pos) = self.last_positions.get(&visual.value.node_id).copied() {
                    if old_pos != new_pos {
                        output.commands.push(EditorCommand::MoveNode {
                            node_id: visual.value.node_id,
                            from: old_pos,
                            to: new_pos,
                        });
                    }
                }
                self.last_positions.insert(visual.value.node_id, new_pos);
            }
        }

        if let Some(asset) = dragged_asset {
            let released = ui.ctx().input(|input| input.pointer.any_released());
            let hover_pos = ui.ctx().input(|input| input.pointer.interact_pos());
            if released && ui.rect_contains_pointer(ui.max_rect()) {
                if let Some(pos) = hover_pos {
                    let new_id = *next_node_id;
                    *next_node_id += 1;
                    let node = build_node_for_kind(new_id, NodeKind::AssetReference, Some(&asset));
                    let snarl_node = self
                        .snarl
                        .insert_node(pos, SnarlVisualNode::from_node(&node));
                    self.node_map.insert(node.id, snarl_node);
                    self.last_positions.insert(node.id, [pos.x, pos.y]);
                    output.commands.push(EditorCommand::AddNode {
                        node,
                        position: [pos.x, pos.y],
                    });
                }
            }
        }

        output
    }
}

struct ProjectSnarlViewer<'a> {
    commands: Vec<EditorCommand>,
    selected_node: Option<NodeId>,
    next_node_id: &'a mut NodeId,
    workspace_mode: EditorWorkspaceMode,
}

impl<'a> SnarlViewer<SnarlVisualNode> for ProjectSnarlViewer<'a> {
    fn title(&mut self, node: &SnarlVisualNode) -> String {
        format!("{} [{:?}]", node.name, node.kind)
    }

    fn inputs(&mut self, _node: &SnarlVisualNode) -> usize {
        1
    }

    fn show_input(&mut self, pin: &InPin, ui: &mut egui::Ui, snarl: &mut Snarl<SnarlVisualNode>) -> impl egui_snarl::ui::SnarlPin + 'static {
        let node = &snarl[pin.id.node];
        ui.label(format!("in: {:?}", node_input_pin_type(node.kind)));
        PinInfo::circle().with_fill(egui::Color32::from_rgb(100, 180, 255))
    }

    fn outputs(&mut self, _node: &SnarlVisualNode) -> usize {
        1
    }

    fn show_output(&mut self, pin: &OutPin, ui: &mut egui::Ui, snarl: &mut Snarl<SnarlVisualNode>) -> impl egui_snarl::ui::SnarlPin + 'static {
        let node = &snarl[pin.id.node];
        ui.label(format!("out: {:?}", node_output_pin_type(node.kind)));
        PinInfo::square().with_fill(egui::Color32::from_rgb(255, 180, 100))
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<SnarlVisualNode>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut egui::Ui, snarl: &mut Snarl<SnarlVisualNode>) {
        for kind in workspace_kinds(self.workspace_mode) {
            if ui.button(format!("Add {:?}", kind)).clicked() {
                let new_id = *self.next_node_id;
                *self.next_node_id += 1;
                let node = build_node_for_kind(new_id, *kind, None);

                snarl.insert_node(pos, SnarlVisualNode::from_node(&node));
                self.commands.push(EditorCommand::AddNode {
                    node,
                    position: [pos.x, pos.y],
                });
                ui.close();
                break;
            }
        }
    }

    fn has_node_menu(&mut self, _node: &SnarlVisualNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: SnarlNodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<SnarlVisualNode>,
    ) {
        let visual = snarl[node].clone();
        if ui.button("Delete Node").clicked() {
            let removed = snarl.remove_node(node);
            let remove = EditorCommand::RemoveNode {
                node: Node {
                    id: removed.node_id,
                    name: removed.name,
                    kind: removed.kind,
                    target: removed.target,
                    dependencies: Vec::new(),
                    settings: BTreeMap::new(),
                    gpu_bindings: Vec::new(),
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: Vec::new(),
                    shader_entry: None,
                    shader_profile: None,
                },
                position: None,
            };
            self.commands.push(remove);
            if self.selected_node == Some(visual.node_id) {
                self.selected_node = None;
            }
            ui.close();
        }
    }

    fn show_header(
        &mut self,
        node: SnarlNodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<SnarlVisualNode>,
    ) {
        let selected = self.selected_node == Some(snarl[node].node_id);
        let text = self.title(&snarl[node]);
        let response = ui.selectable_label(selected, text);
        if response.clicked() {
            self.selected_node = Some(snarl[node].node_id);
        }
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<SnarlVisualNode>) {
        let from_node = snarl[from.id.node].node_id;
        let to_node = snarl[to.id.node].node_id;

        let from_type = node_output_pin_type(snarl[from.id.node].kind);
        let to_type = node_input_pin_type(snarl[to.id.node].kind);
        if !pin_types_compatible(from_type, to_type) {
            return;
        }

        snarl.connect(from.id, to.id);
        self.commands.push(EditorCommand::ConnectNodes {
            from: from_node,
            to: to_node,
        });
    }

    fn disconnect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<SnarlVisualNode>) {
        let from_node = snarl[from.id.node].node_id;
        let to_node = snarl[to.id.node].node_id;

        snarl.disconnect(from.id, to.id);
        self.commands.push(EditorCommand::DisconnectNodes {
            from: from_node,
            to: to_node,
        });
    }
}

fn build_node_for_kind(
    id: NodeId,
    kind: NodeKind,
    asset: Option<&ProjectAssetEntry>,
) -> Node {
    let mut node = Node {
        id,
        name: format!("{:?}_{id}", kind).to_ascii_lowercase(),
        kind,
        target: match kind {
            NodeKind::GameplayFlow
            | NodeKind::GameplayEvent
            | NodeKind::MathState
            | NodeKind::ScriptBehavior
            | NodeKind::ObjectInitializer => NodeExecutionTarget::Cpu,
            NodeKind::AssetReference => NodeExecutionTarget::Hybrid,
            NodeKind::ComputePass | NodeKind::RenderPass | NodeKind::BuildExport => {
                NodeExecutionTarget::Gpu
            }
        },
        dependencies: Vec::new(),
        settings: BTreeMap::new(),
        gpu_bindings: Vec::new(),
        compute: None,
        fallback_policy: NodeFallbackPolicy::Cpu,
        gpu_resource_states: Vec::new(),
        shader_entry: None,
        shader_profile: None,
    };

    if let Some(asset) = asset {
        node.settings
            .insert("asset_path".to_string(), asset.path.display().to_string());
        node.settings
            .insert("asset_kind".to_string(), format!("{:?}", asset.kind));
        if let Some(stem) = asset.path.file_stem().and_then(|stem| stem.to_str()) {
            node.name = stem.to_string();
        }
    }

    node
}

pub fn sync_document_dependencies_from_snarl(document: &mut EditorDocument, snarl: &Snarl<SnarlVisualNode>) {
    let mut deps: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();

    for (from, to) in snarl.wires() {
        let Some(from_node) = snarl.get_node(from.node).map(|node| node.node_id) else {
            continue;
        };
        let Some(to_node) = snarl.get_node(to.node).map(|node| node.node_id) else {
            continue;
        };

        deps.entry(to_node).or_default().insert(from_node);
    }

    for node in &mut document.graph.nodes {
        node.dependencies = deps
            .get(&node.id)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();
    }
}

pub fn apply_inspector_node_change(
    project: &mut EditorProjectState,
    node_id: NodeId,
    kind: Option<NodeKind>,
    target: Option<NodeExecutionTarget>,
    fallback: Option<NodeFallbackPolicy>,
    shader_entry: Option<Option<String>>,
    shader_profile: Option<Option<String>>,
    setting_change: Option<(String, Option<String>)>,
) -> Result<(), EditorError> {
    let Some(node) = project.document.graph.nodes.iter().find(|node| node.id == node_id).cloned() else {
        return Ok(());
    };

    let mut commands = Vec::new();

    if let Some(kind_new) = kind {
        if kind_new != node.kind {
            commands.push(EditorCommand::SetNodeKind {
                node_id,
                old: node.kind,
                new: kind_new,
            });
        }
    }

    if let Some(target_new) = target {
        if target_new != node.target {
            commands.push(EditorCommand::SetNodeTarget {
                node_id,
                old: node.target,
                new: target_new,
            });
        }
    }

    if let Some(fallback_new) = fallback {
        if fallback_new != node.fallback_policy {
            commands.push(EditorCommand::SetNodeFallback {
                node_id,
                old: node.fallback_policy,
                new: fallback_new,
            });
        }
    }

    if shader_entry.is_some() || shader_profile.is_some() {
        let new_entry = shader_entry.unwrap_or_else(|| node.shader_entry.clone());
        let new_profile = shader_profile.unwrap_or_else(|| node.shader_profile.clone());

        if new_entry != node.shader_entry || new_profile != node.shader_profile {
            commands.push(EditorCommand::SetNodeShaderMeta {
                node_id,
                old_entry: node.shader_entry.clone(),
                new_entry,
                old_profile: node.shader_profile.clone(),
                new_profile,
            });
        }
    }

    if let Some((key, new_value)) = setting_change {
        let old_value = node.settings.get(&key).cloned();
        if old_value != new_value {
            commands.push(EditorCommand::SetNodeSetting {
                node_id,
                key,
                old: old_value,
                new: new_value,
            });
        }
    }

    project.apply_command_batch("inspector_change", commands)
}

pub fn draw_overlay(
    ctx: &egui::Context,
    state: &mut EditorState,
    timings: &FrameTimings,
    active_backend: BackendKind,
    capabilities: BackendCapabilities,
    diagnostics: &[CompileDiagnostic],
    backend_diagnostics: &BackendDiagnostics,
) {
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui
                .button(if state.is_playing { "Stop" } else { "Play" })
                .clicked()
            {
                state.is_playing = !state.is_playing;
            }
            if ui.button("Recompile Graph").clicked() {
                state.graph_dirty = true;
            }
        });
    });

    egui::Window::new("Profiler").show(ctx, |ui| {
        ui.label(format!("Backend: {:?}", active_backend));
        ui.label(format!(
            "Capability surface: {}",
            backend_diagnostics.supports_surface
        ));
        ui.label(format!(
            "Capabilities mask: sprites={} batching={} camera={} blend={} offscreen={} atlas={} gpu_nodes={} hybrid={} compute={}",
            capabilities.textured_sprites,
            capabilities.batching,
            capabilities.camera_transforms,
            capabilities.blend_modes,
            capabilities.offscreen_targets,
            capabilities.texture_atlas,
            capabilities.gpu_nodes,
            capabilities.hybrid_nodes,
            capabilities.compute_nodes
        ));
        ui.label(format!("CPU frame: {:.2} ms", timings.cpu_frame_ms));
        ui.label(format!(
            "GPU frame (reported): {:.2} ms",
            backend_diagnostics.last_gpu_frame_ms
        ));
        ui.label(format!(
            "Fallback events: {}",
            backend_diagnostics.fallback_events
        ));
        ui.label(format!(
            "Swapchain recreates: {}",
            backend_diagnostics.swapchain_recreates
        ));
        ui.label(format!(
            "Device loss events: {}",
            backend_diagnostics.device_loss_events
        ));
        ui.label(format!("Node compile: {:.2} ms", timings.node_compile_ms));
        ui.label(format!("Fallback diagnostics: {}", diagnostics.len()));
    });

    egui::Window::new("Backend Diagnostics").show(ctx, |ui| {
        for event in &backend_diagnostics.events {
            let level = match event.level {
                BackendDiagnosticLevel::Info => "INFO",
                BackendDiagnosticLevel::Warning => "WARN",
                BackendDiagnosticLevel::Error => "ERROR",
            };
            ui.label(format!("[{level}] {}", event.message));
        }

        if backend_diagnostics.events.is_empty() {
            ui.label("No backend diagnostics.");
        }
    });

    egui::Window::new("Pass Timings").show(ctx, |ui| {
        for timing in backend_diagnostics.pass_timings.iter().rev().take(16) {
            ui.label(format!(
                "frame={} pass={} cpu={:.3}ms{}",
                timing.frame,
                timing.pass,
                timing.cpu_ms,
                timing
                    .gpu_ms
                    .map(|gpu_ms| format!(", gpu={gpu_ms:.3}ms"))
                    .unwrap_or_default()
            ));
        }

        if backend_diagnostics.pass_timings.is_empty() {
            ui.label("No pass timing samples yet.");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sample_node(id: NodeId, kind: NodeKind) -> Node {
        Node {
            id,
            name: format!("node_{id}"),
            kind,
            target: NodeExecutionTarget::Cpu,
            dependencies: vec![],
            settings: BTreeMap::new(),
            gpu_bindings: vec![],
            compute: None,
            fallback_policy: NodeFallbackPolicy::Cpu,
            gpu_resource_states: vec![],
            shader_entry: None,
            shader_profile: None,
        }
    }

    #[test]
    fn pin_contract_rules_are_stable() {
        assert!(pin_types_compatible(PinTypeCategory::Texture, PinTypeCategory::Texture));
        assert!(pin_types_compatible(PinTypeCategory::Texture, PinTypeCategory::Data));
        assert!(!pin_types_compatible(PinTypeCategory::Audio, PinTypeCategory::Texture));
    }

    #[test]
    fn command_roundtrip_history_replay_is_deterministic() {
        let document = EditorDocument::from_graph(NodeGraph {
            version: CURRENT_GRAPH_VERSION,
            nodes: vec![sample_node(1, NodeKind::GameplayEvent)],
        });
        let mut history = HistoryGraph::new(document.clone());

        let command = EditorCommand::AddNode {
            node: sample_node(2, NodeKind::RenderPass),
            position: [120.0, 220.0],
        };

        let mut changed = document.clone();
        apply_command_internal(&mut changed, &command).expect("command should apply");
        let id = history.record("add", command, changed);

        assert!(history.replay_matches_snapshot(id));
    }

    #[test]
    fn transaction_grouping_and_branching_work() {
        let document = EditorDocument::from_graph(NodeGraph {
            version: CURRENT_GRAPH_VERSION,
            nodes: vec![sample_node(1, NodeKind::GameplayEvent)],
        });
        let mut history = HistoryGraph::new(document.clone());

        let tx = history.begin_transaction();
        let mut d1 = document.clone();
        apply_command_internal(
            &mut d1,
            &EditorCommand::AddNode {
                node: sample_node(2, NodeKind::MathState),
                position: [0.0, 0.0],
            },
        )
        .expect("command should apply");
        let first = history.record(
            "tx_add",
            EditorCommand::AddNode {
                node: sample_node(2, NodeKind::MathState),
                position: [0.0, 0.0],
            },
            d1.clone(),
        );

        let mut d2 = d1.clone();
        apply_command_internal(
            &mut d2,
            &EditorCommand::SetNodeTarget {
                node_id: 2,
                old: NodeExecutionTarget::Cpu,
                new: NodeExecutionTarget::Gpu,
            },
        )
        .expect("command should apply");
        let second = history.record(
            "tx_target",
            EditorCommand::SetNodeTarget {
                node_id: 2,
                old: NodeExecutionTarget::Cpu,
                new: NodeExecutionTarget::Gpu,
            },
            d2,
        );
        history.end_transaction();

        assert_eq!(history.nodes[&first].transaction, tx);
        assert_eq!(history.nodes[&second].transaction, tx);

        history.undo().expect("undo should work");
        let mut d3 = history
            .current_document()
            .expect("current doc should exist")
            .clone();
        apply_command_internal(
            &mut d3,
            &EditorCommand::SetNodeKind {
                node_id: 2,
                old: NodeKind::MathState,
                new: NodeKind::ComputePass,
            },
        )
        .expect("command should apply");
        let branch = history.record(
            "branch_kind",
            EditorCommand::SetNodeKind {
                node_id: 2,
                old: NodeKind::MathState,
                new: NodeKind::ComputePass,
            },
            d3,
        );

        let parent = history.nodes[&branch].parent.expect("branch should have parent");
        assert!(history.nodes[&parent].children.len() >= 2);
    }

    #[test]
    fn project_session_roundtrip_is_stable() {
        let temp_dir = std::env::temp_dir().join("rusty_engine_editor_project");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(temp_dir.join("assets")).expect("create temp assets dir");

        let scene = temp_dir.join("assets/sample_scene.ron");
        save_node_graph(&scene, &NodeGraph::empty()).expect("save scene");

        let mut project = EditorProjectState::open(&temp_dir, Some(scene.clone())).expect("open project");
        project
            .apply_command_batch(
                "add_node",
                vec![EditorCommand::AddNode {
                    node: sample_node(10, NodeKind::RenderPass),
                    position: [300.0, 400.0],
                }],
            )
            .expect("apply batch");
        project.persist_session().expect("persist session");

        let reopened = EditorProjectState::open(&temp_dir, Some(scene)).expect("reopen");
        assert!(!reopened.session.history.nodes.is_empty());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
