use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use egui_snarl::ui::{PinInfo, SnarlStyle, SnarlViewer};
use egui_snarl::{InPin, NodeId as SnarlNodeId, OutPin, Snarl};
use engine_assets::{
    infer_asset_kind, load_scene_document, save_scene_document, AssetError, AssetKind,
    Camera2DComponent, Collider2D, SceneDocument, SceneLayer, SceneObject, Sprite2D, Transform2D,
};
use engine_nodes::{
    AssetReferencePayload, BuildExportPayload, CompileDiagnostic, ComputePassPayload,
    GameplayEventPayload, GameplayFlowPayload, MathStatePayload, Node, NodeExecutionTarget,
    NodeFallbackPolicy, NodeGraph, NodeId, NodeKind, NodePayload, ObjectInitializerPayload,
    RenderPassPayload, ScriptBehaviorPayload, CURRENT_GRAPH_VERSION,
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
    pub scene: SceneDocument,
    pub node_positions: BTreeMap<NodeId, [f32; 2]>,
}

impl EditorDocument {
    pub fn from_scene(scene: SceneDocument) -> Self {
        let mut node_positions = BTreeMap::new();
        for (index, node) in scene.graph.nodes.iter().enumerate() {
            let x = 80.0 + (index as f32 % 6.0) * 220.0;
            let y = 80.0 + (index as f32 / 6.0).floor() * 140.0;
            node_positions.insert(node.id, [x, y]);
        }

        Self {
            scene,
            node_positions,
        }
    }

    pub fn from_graph(graph: NodeGraph) -> Self {
        Self::from_scene(SceneDocument::from_graph(graph))
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
    AddLayer {
        layer: SceneLayer,
    },
    RemoveLayer {
        layer: SceneLayer,
        removed_objects: Vec<SceneObject>,
    },
    SetLayerProps {
        layer_id: u64,
        old_name: String,
        new_name: String,
        old_order: i32,
        new_order: i32,
        old_visible: bool,
        new_visible: bool,
        old_locked: bool,
        new_locked: bool,
    },
    AddObject {
        object: SceneObject,
    },
    RemoveObject {
        object: SceneObject,
    },
    ReparentObject {
        object_id: u64,
        old_parent: Option<u64>,
        new_parent: Option<u64>,
    },
    MoveObjectToLayer {
        object_id: u64,
        old_layer: u64,
        new_layer: u64,
    },
    SetObjectName {
        object_id: u64,
        old_name: String,
        new_name: String,
    },
    SetObjectTransform {
        object_id: u64,
        old: Transform2D,
        new: Transform2D,
    },
    SetObjectSprite {
        object_id: u64,
        old: Option<Sprite2D>,
        new: Option<Sprite2D>,
    },
    SetObjectCollider {
        object_id: u64,
        old: Option<Collider2D>,
        new: Option<Collider2D>,
    },
    SetObjectCamera {
        object_id: u64,
        old: Option<Camera2DComponent>,
        new: Option<Camera2DComponent>,
    },
    SetObjectCustom {
        object_id: u64,
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
    Asset(#[from] AssetError),

    #[error("legacy graph-only files are not editable in the scene editor; convert to .scene.ron")]
    LegacySceneFormat,

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
    pub next_layer_id: u64,
    pub next_object_id: u64,
}

impl EditorProjectState {
    pub fn open(
        project_root: impl AsRef<Path>,
        scene_override: Option<PathBuf>,
    ) -> Result<Self, EditorError> {
        let project_root = project_root.as_ref().to_path_buf();
        fs::create_dir_all(&project_root)?;

        let scene_path =
            scene_override.unwrap_or_else(|| project_root.join("assets/sample_scene.scene.ron"));
        if !scene_path.exists() {
            if let Some(parent) = scene_path.parent() {
                fs::create_dir_all(parent)?;
            }
            save_scene_document(&scene_path, &SceneDocument::new_default())?;
        }

        let scene = match load_scene_document(&scene_path) {
            Ok(scene) => scene,
            Err(AssetError::LegacyGraphFormat) => {
                return Err(EditorError::LegacySceneFormat);
            }
            Err(err) => return Err(EditorError::Asset(err)),
        };
        let document = EditorDocument::from_scene(scene);
        let next_node_id = document
            .scene
            .graph
            .nodes
            .iter()
            .map(|node| node.id)
            .max()
            .unwrap_or(0)
            + 1;
        let next_layer_id = document
            .scene
            .layers
            .iter()
            .map(|layer| layer.layer_id)
            .max()
            .unwrap_or(0)
            + 1;
        let next_object_id = document
            .scene
            .objects
            .iter()
            .map(|object| object.object_id)
            .max()
            .unwrap_or(0)
            + 1;

        let mut session = match load_session(&project_root) {
            Ok(Some(session)) => session,
            Ok(None) | Err(EditorError::SessionParse(_)) => EditorSessionState {
                selected_node: None,
                selected_asset: None,
                recent_projects: vec![project_root.clone()],
                viewport: EditorViewportState::default(),
                history: HistoryGraph::new(document.clone()),
                diagnostics: Vec::new(),
                workspace_mode: EditorWorkspaceMode::default(),
            },
            Err(err) => return Err(err),
        };

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
            next_layer_id,
            next_object_id,
        };

        project.refresh_asset_index()?;
        Ok(project)
    }

    pub fn save_scene(&mut self) -> Result<(), EditorError> {
        save_scene_document(&self.scene_path, &self.document.scene)?;
        self.dirty = false;
        self.persist_session()?;
        Ok(())
    }

    pub fn save_scene_as(&mut self, path: impl AsRef<Path>) -> Result<(), EditorError> {
        self.scene_path = path.as_ref().to_path_buf();
        save_scene_document(&self.scene_path, &self.document.scene)?;
        self.dirty = false;
        self.persist_session()?;
        Ok(())
    }

    pub fn autosave_if_dirty(&mut self) -> Result<(), EditorError> {
        if !self.dirty {
            return Ok(());
        }

        let autosave_path = self
            .project_root
            .join(".rusty_engine/editor_autosave.scene.ron");
        if let Some(parent) = autosave_path.parent() {
            fs::create_dir_all(parent)?;
        }
        save_scene_document(autosave_path, &self.document.scene)?;
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

        self.session
            .history
            .record(label, EditorCommand::Batch { commands }, updated);

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
        let node = self
            .document
            .scene
            .graph
            .nodes
            .iter()
            .find(|node| node.id == node_id)?
            .clone();
        let position = self.document.node_positions.get(&node_id).copied();
        Some(EditorCommand::RemoveNode { node, position })
    }

    pub fn allocate_layer_id(&mut self) -> u64 {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        id
    }

    pub fn allocate_object_id(&mut self) -> u64 {
        let id = self.next_object_id;
        self.next_object_id += 1;
        id
    }

    pub fn default_layer_id(&self) -> u64 {
        self.document
            .scene
            .layers
            .iter()
            .min_by_key(|layer| layer.order)
            .map(|layer| layer.layer_id)
            .unwrap_or(1)
    }

    pub fn autosave_path(&self) -> PathBuf {
        self.project_root
            .join(".rusty_engine/editor_autosave.scene.ron")
    }
}

fn apply_command_internal(
    document: &mut EditorDocument,
    command: &EditorCommand,
) -> Result<(), EditorError> {
    match command {
        EditorCommand::AddNode { node, position } => {
            if document
                .scene
                .graph
                .nodes
                .iter()
                .any(|existing| existing.id == node.id)
            {
                return Err(EditorError::Command(format!(
                    "duplicate node id {}",
                    node.id
                )));
            }
            document.scene.graph.nodes.push(node.clone());
            document.node_positions.insert(node.id, *position);
        }
        EditorCommand::RemoveNode { node, .. } => {
            document
                .scene
                .graph
                .nodes
                .retain(|existing| existing.id != node.id);
            document.node_positions.remove(&node.id);
            for existing in &mut document.scene.graph.nodes {
                existing.dependencies.retain(|dep| *dep != node.id);
            }
        }
        EditorCommand::ConnectNodes { from, to } => {
            if from == to {
                return Err(EditorError::Command(
                    "cannot connect node to itself".to_string(),
                ));
            }

            let out_node = document
                .scene
                .graph
                .nodes
                .iter()
                .find(|node| node.id == *from)
                .ok_or_else(|| EditorError::Command(format!("missing source node {from}")))?;
            let in_node = document
                .scene
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
                .scene
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
                .scene
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
                .scene
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == *node_id)
                .ok_or_else(|| EditorError::Command(format!("missing node {node_id}")))?;
            node.kind = *new;
        }
        EditorCommand::SetNodeTarget { node_id, new, .. } => {
            let node = document
                .scene
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == *node_id)
                .ok_or_else(|| EditorError::Command(format!("missing node {node_id}")))?;
            node.target = *new;
        }
        EditorCommand::SetNodeFallback { node_id, new, .. } => {
            let node = document
                .scene
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
                .scene
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == *node_id)
                .ok_or_else(|| EditorError::Command(format!("missing node {node_id}")))?;
            node.shader_entry = new_entry.clone();
            node.shader_profile = new_profile.clone();
        }
        EditorCommand::SetNodeSetting {
            node_id, key, new, ..
        } => {
            let node = document
                .scene
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
        EditorCommand::AddLayer { layer } => {
            if document
                .scene
                .layers
                .iter()
                .any(|existing| existing.layer_id == layer.layer_id)
            {
                return Err(EditorError::Command(format!(
                    "duplicate layer id {}",
                    layer.layer_id
                )));
            }
            document.scene.layers.push(layer.clone());
            document.scene.layers.sort_by_key(|entry| entry.order);
        }
        EditorCommand::RemoveLayer {
            layer,
            removed_objects,
        } => {
            document
                .scene
                .layers
                .retain(|existing| existing.layer_id != layer.layer_id);
            let removed_set = removed_objects
                .iter()
                .map(|object| object.object_id)
                .collect::<BTreeSet<_>>();
            document
                .scene
                .objects
                .retain(|existing| !removed_set.contains(&existing.object_id));
        }
        EditorCommand::SetLayerProps {
            layer_id,
            new_name,
            new_order,
            new_visible,
            new_locked,
            ..
        } => {
            let layer = document
                .scene
                .layers
                .iter_mut()
                .find(|layer| layer.layer_id == *layer_id)
                .ok_or_else(|| EditorError::Command(format!("missing layer {layer_id}")))?;
            layer.name = new_name.clone();
            layer.order = *new_order;
            layer.visible = *new_visible;
            layer.locked = *new_locked;
            document.scene.layers.sort_by_key(|entry| entry.order);
        }
        EditorCommand::AddObject { object } => {
            if document
                .scene
                .objects
                .iter()
                .any(|existing| existing.object_id == object.object_id)
            {
                return Err(EditorError::Command(format!(
                    "duplicate object id {}",
                    object.object_id
                )));
            }
            if !document
                .scene
                .layers
                .iter()
                .any(|layer| layer.layer_id == object.layer_id)
            {
                return Err(EditorError::Command(format!(
                    "object references missing layer {}",
                    object.layer_id
                )));
            }
            if let Some(parent) = object.parent {
                if !document
                    .scene
                    .objects
                    .iter()
                    .any(|existing| existing.object_id == parent)
                {
                    return Err(EditorError::Command(format!(
                        "object references missing parent {}",
                        parent
                    )));
                }
            }
            document.scene.objects.push(object.clone());
        }
        EditorCommand::RemoveObject { object } => {
            document
                .scene
                .objects
                .retain(|existing| existing.object_id != object.object_id);
            for other in &mut document.scene.objects {
                if other.parent == Some(object.object_id) {
                    other.parent = None;
                }
            }
        }
        EditorCommand::ReparentObject {
            object_id,
            new_parent,
            ..
        } => {
            if let Some(parent) = new_parent {
                if *parent == *object_id {
                    return Err(EditorError::Command(
                        "cannot parent object to itself".to_string(),
                    ));
                }
                if !document
                    .scene
                    .objects
                    .iter()
                    .any(|existing| existing.object_id == *parent)
                {
                    return Err(EditorError::Command(format!(
                        "missing parent object {parent}"
                    )));
                }
                if would_create_cycle(&document.scene.objects, *object_id, *parent) {
                    return Err(EditorError::Command(
                        "reparent would create hierarchy cycle".to_string(),
                    ));
                }
            }
            let object = document
                .scene
                .objects
                .iter_mut()
                .find(|object| object.object_id == *object_id)
                .ok_or_else(|| EditorError::Command(format!("missing object {object_id}")))?;
            object.parent = *new_parent;
        }
        EditorCommand::MoveObjectToLayer {
            object_id,
            new_layer,
            ..
        } => {
            if !document
                .scene
                .layers
                .iter()
                .any(|layer| layer.layer_id == *new_layer)
            {
                return Err(EditorError::Command(format!("missing layer {new_layer}")));
            }
            let object = document
                .scene
                .objects
                .iter_mut()
                .find(|object| object.object_id == *object_id)
                .ok_or_else(|| EditorError::Command(format!("missing object {object_id}")))?;
            object.layer_id = *new_layer;
        }
        EditorCommand::SetObjectName {
            object_id,
            new_name,
            ..
        } => {
            let object = document
                .scene
                .objects
                .iter_mut()
                .find(|object| object.object_id == *object_id)
                .ok_or_else(|| EditorError::Command(format!("missing object {object_id}")))?;
            object.name = new_name.clone();
        }
        EditorCommand::SetObjectTransform { object_id, new, .. } => {
            let object = document
                .scene
                .objects
                .iter_mut()
                .find(|object| object.object_id == *object_id)
                .ok_or_else(|| EditorError::Command(format!("missing object {object_id}")))?;
            object.components.transform = new.clone();
        }
        EditorCommand::SetObjectSprite { object_id, new, .. } => {
            let object = document
                .scene
                .objects
                .iter_mut()
                .find(|object| object.object_id == *object_id)
                .ok_or_else(|| EditorError::Command(format!("missing object {object_id}")))?;
            object.components.sprite = new.clone();
        }
        EditorCommand::SetObjectCollider { object_id, new, .. } => {
            let object = document
                .scene
                .objects
                .iter_mut()
                .find(|object| object.object_id == *object_id)
                .ok_or_else(|| EditorError::Command(format!("missing object {object_id}")))?;
            object.components.collider = new.clone();
        }
        EditorCommand::SetObjectCamera { object_id, new, .. } => {
            let object = document
                .scene
                .objects
                .iter_mut()
                .find(|object| object.object_id == *object_id)
                .ok_or_else(|| EditorError::Command(format!("missing object {object_id}")))?;
            object.components.camera = new.clone();
        }
        EditorCommand::SetObjectCustom {
            object_id,
            key,
            new,
            ..
        } => {
            let object = document
                .scene
                .objects
                .iter_mut()
                .find(|object| object.object_id == *object_id)
                .ok_or_else(|| EditorError::Command(format!("missing object {object_id}")))?;
            match new {
                Some(value) => {
                    object
                        .components
                        .custom_properties
                        .insert(key.clone(), value.clone());
                }
                None => {
                    object.components.custom_properties.remove(key);
                }
            }
        }
        EditorCommand::Batch { commands } => {
            for nested in commands {
                apply_command_internal(document, nested)?;
            }
        }
    }

    validate_document_integrity(document)?;
    document.scene.graph.version = CURRENT_GRAPH_VERSION;
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

fn would_create_cycle(objects: &[SceneObject], object_id: u64, candidate_parent: u64) -> bool {
    let mut cursor = Some(candidate_parent);
    while let Some(current) = cursor {
        if current == object_id {
            return true;
        }
        cursor = objects
            .iter()
            .find(|object| object.object_id == current)
            .and_then(|object| object.parent);
    }
    false
}

fn validate_document_integrity(document: &EditorDocument) -> Result<(), EditorError> {
    let layer_ids = document
        .scene
        .layers
        .iter()
        .map(|layer| layer.layer_id)
        .collect::<BTreeSet<_>>();
    for object in &document.scene.objects {
        if !layer_ids.contains(&object.layer_id) {
            return Err(EditorError::Command(format!(
                "object {} references missing layer {}",
                object.object_id, object.layer_id
            )));
        }
        if let Some(parent) = object.parent {
            if !document
                .scene
                .objects
                .iter()
                .any(|candidate| candidate.object_id == parent)
            {
                return Err(EditorError::Command(format!(
                    "object {} references missing parent {}",
                    object.object_id, parent
                )));
            }
            if would_create_cycle(&document.scene.objects, object.object_id, parent) {
                return Err(EditorError::Command(format!(
                    "hierarchy cycle detected for object {}",
                    object.object_id
                )));
            }
        }
    }

    let node_ids = document
        .scene
        .graph
        .nodes
        .iter()
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    for node in &document.scene.graph.nodes {
        for dep in &node.dependencies {
            if !node_ids.contains(dep) {
                return Err(EditorError::Command(format!(
                    "node {} has dangling dependency {}",
                    node.id, dep
                )));
            }
        }
    }

    Ok(())
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

        for node in &document.scene.graph.nodes {
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

        for node in &document.scene.graph.nodes {
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
        self.snarl
            .show(&mut viewer, &style, "editor_graph_snarl", ui);

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

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<SnarlVisualNode>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        let node = &snarl[pin.id.node];
        ui.label(format!("in: {:?}", node_input_pin_type(node.kind)));
        PinInfo::circle().with_fill(egui::Color32::from_rgb(100, 180, 255))
    }

    fn outputs(&mut self, _node: &SnarlVisualNode) -> usize {
        1
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<SnarlVisualNode>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        let node = &snarl[pin.id.node];
        ui.label(format!("out: {:?}", node_output_pin_type(node.kind)));
        PinInfo::square().with_fill(egui::Color32::from_rgb(255, 180, 100))
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<SnarlVisualNode>) -> bool {
        true
    }

    fn show_graph_menu(
        &mut self,
        pos: egui::Pos2,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<SnarlVisualNode>,
    ) {
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
                    payload: None,
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

fn build_node_for_kind(id: NodeId, kind: NodeKind, asset: Option<&ProjectAssetEntry>) -> Node {
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
        payload: Some(default_payload_for_kind(kind)),
    };

    if let Some(asset) = asset {
        node.settings
            .insert("asset_path".to_string(), asset.path.display().to_string());
        node.settings
            .insert("asset_kind".to_string(), format!("{:?}", asset.kind));
        node.payload = Some(NodePayload::AssetReference(AssetReferencePayload {
            asset_path: asset.path.display().to_string(),
            asset_kind: format!("{:?}", asset.kind),
        }));
        if let Some(stem) = asset.path.file_stem().and_then(|stem| stem.to_str()) {
            node.name = stem.to_string();
        }
    }

    node
}

fn default_payload_for_kind(kind: NodeKind) -> NodePayload {
    match kind {
        NodeKind::GameplayEvent => NodePayload::GameplayEvent(GameplayEventPayload::default()),
        NodeKind::GameplayFlow => NodePayload::GameplayFlow(GameplayFlowPayload::default()),
        NodeKind::MathState => NodePayload::MathState(MathStatePayload::default()),
        NodeKind::ScriptBehavior => NodePayload::ScriptBehavior(ScriptBehaviorPayload::default()),
        NodeKind::ObjectInitializer => {
            NodePayload::ObjectInitializer(ObjectInitializerPayload::default())
        }
        NodeKind::RenderPass => NodePayload::RenderPass(RenderPassPayload::default()),
        NodeKind::ComputePass => NodePayload::ComputePass(ComputePassPayload::default()),
        NodeKind::AssetReference => NodePayload::AssetReference(AssetReferencePayload::default()),
        NodeKind::BuildExport => NodePayload::BuildExport(BuildExportPayload::default()),
    }
}

pub fn sync_document_dependencies_from_snarl(
    document: &mut EditorDocument,
    snarl: &Snarl<SnarlVisualNode>,
) {
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

    for node in &mut document.scene.graph.nodes {
        node.dependencies = deps
            .get(&node.id)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();
    }
}

#[allow(clippy::too_many_arguments)]
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
    let Some(node) = project
        .document
        .scene
        .graph
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .cloned()
    else {
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
    use engine_assets::SceneComponents;
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
            payload: Some(default_payload_for_kind(kind)),
        }
    }

    #[test]
    fn pin_contract_rules_are_stable() {
        assert!(pin_types_compatible(
            PinTypeCategory::Texture,
            PinTypeCategory::Texture
        ));
        assert!(pin_types_compatible(
            PinTypeCategory::Texture,
            PinTypeCategory::Data
        ));
        assert!(!pin_types_compatible(
            PinTypeCategory::Audio,
            PinTypeCategory::Texture
        ));
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

        let parent = history.nodes[&branch]
            .parent
            .expect("branch should have parent");
        assert!(history.nodes[&parent].children.len() >= 2);
    }

    #[test]
    fn project_session_roundtrip_is_stable() {
        let temp_dir = std::env::temp_dir().join("rusty_engine_editor_project");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(temp_dir.join("assets")).expect("create temp assets dir");

        let scene = temp_dir.join("assets/sample_scene.scene.ron");
        save_scene_document(&scene, &SceneDocument::from_graph(NodeGraph::empty()))
            .expect("save scene");

        let mut project =
            EditorProjectState::open(&temp_dir, Some(scene.clone())).expect("open project");
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

    #[test]
    fn scene_commands_roundtrip_and_cycle_validation() {
        let mut document = EditorDocument::from_scene(SceneDocument::new_default());

        let layer = SceneLayer {
            layer_id: 9,
            name: "Gameplay".to_string(),
            order: 9,
            visible: true,
            locked: false,
        };
        apply_command_internal(
            &mut document,
            &EditorCommand::AddLayer {
                layer: layer.clone(),
            },
        )
        .expect("layer add should apply");

        let parent_object = SceneObject {
            object_id: 10,
            parent: None,
            layer_id: 9,
            name: "Parent".to_string(),
            tags: vec![],
            components: SceneComponents::default(),
        };
        let child_object = SceneObject {
            object_id: 11,
            parent: Some(10),
            layer_id: 9,
            name: "Child".to_string(),
            tags: vec![],
            components: SceneComponents::default(),
        };
        apply_command_internal(
            &mut document,
            &EditorCommand::AddObject {
                object: parent_object.clone(),
            },
        )
        .expect("parent object add should apply");
        apply_command_internal(
            &mut document,
            &EditorCommand::AddObject {
                object: child_object.clone(),
            },
        )
        .expect("child object add should apply");

        let cycle_err = apply_command_internal(
            &mut document,
            &EditorCommand::ReparentObject {
                object_id: 10,
                old_parent: None,
                new_parent: Some(11),
            },
        )
        .expect_err("cycle should fail");
        assert!(matches!(cycle_err, EditorError::Command(_)));
    }
}
