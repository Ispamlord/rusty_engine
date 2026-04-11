use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui;
use engine_app::{EngineApp, EngineAppError, RuntimeDiagnosticsSnapshot};
use engine_core::EngineConfig;
use engine_editor::{
    apply_inspector_node_change, pin_types_compatible, node_input_pin_type, node_output_pin_type,
    sync_document_dependencies_from_snarl, EditorError, EditorProjectState, GraphCanvasState,
};
use engine_nodes::{
    Node, NodeExecutionTarget, NodeFallbackPolicy, NodeGraph, NodeId, NodeKind,
    CURRENT_GRAPH_VERSION,
};
use engine_render_api::RenderGraphPass;
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
    pub fn new(project_path: impl AsRef<Path>, config: EditorAppConfig) -> Result<Self, EditorAppError> {
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

        let _ = runtime.set_active_scene_graph(project.document.graph.clone())?;

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
        let graph = engine_assets::load_node_graph(&scene_path)?;
        self.project.scene_path = scene_path;
        self.project.document = engine_editor::EditorDocument::from_graph(graph.clone());
        self.project.next_node_id = self
            .project
            .document
            .graph
            .nodes
            .iter()
            .map(|node| node.id)
            .max()
            .unwrap_or(0)
            + 1;
        self.project.dirty = false;
        self.canvas.rebuild_from_document(&self.project.document);

        let compiled = self.runtime.set_active_scene_graph(graph)?;
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
        let graph = project.document.graph.clone();

        self.project = project;
        self.canvas.rebuild_from_document(&self.project.document);

        let compiled = self.runtime.set_active_scene_graph(graph)?;
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
            .unwrap_or_else(|| project_path.join("assets").join("sample_scene.ron"));

        if !scene_path.exists() {
            if let Some(parent) = scene_path.parent() {
                fs::create_dir_all(parent)?;
            }
            engine_assets::save_node_graph(&scene_path, &Self::default_test_game_graph())?;
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
                },
            ],
        }
    }
}

struct EditorUi {
    app: EditorApp,
    selected_node: Option<NodeId>,
    inspector_name_buffer: String,
    project_modal_open: bool,
    project_path_input: String,
    scene_path_input: String,
}

impl EditorUi {
    fn new(app: EditorApp) -> Self {
        let project_path_input = app.project.project_root.display().to_string();
        let scene_path_input = app.project.scene_path.display().to_string();
        Self {
            app,
            selected_node: None,
            inspector_name_buffer: String::new(),
            project_modal_open: true,
            project_path_input,
            scene_path_input,
        }
    }

    fn set_project_inputs_from_active(&mut self) {
        self.project_path_input = self.app.project.project_root.display().to_string();
        self.scene_path_input = self.app.project.scene_path.display().to_string();
    }

    fn after_project_switch(&mut self) {
        self.selected_node = None;
        self.app.project.session.selected_node = None;
        self.inspector_name_buffer.clear();
        self.set_project_inputs_from_active();
    }

    fn parse_optional_scene_path(&self) -> Option<PathBuf> {
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
        match self.app.open_project(project_path, scene_override) {
            Ok(()) => {
                self.project_modal_open = false;
                self.after_project_switch();
            }
            Err(err) => {
                self.app.status_line = format!("open project failed: {err}");
            }
        }
    }

    fn create_project_from_inputs(&mut self) {
        let project_raw = self.project_path_input.trim();
        if project_raw.is_empty() {
            self.app.status_line = "project path is required".to_string();
            return;
        }

        let project_path = PathBuf::from(project_raw);
        if project_path.is_file() {
            self.app.status_line =
                "project path points to a file; select a directory".to_string();
            return;
        }
        let scene_override = self.parse_optional_scene_path();
        match self.app.create_project_minimal(project_path, scene_override) {
            Ok(()) => {
                self.project_modal_open = false;
                self.after_project_switch();
            }
            Err(err) => {
                self.app.status_line = format!("create project failed: {err}");
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
                    ui.text_edit_singleline(&mut self.project_path_input);
                    if ui.button("Browse Folder...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.project_path_input = path.display().to_string();
                        }
                    }
                });
                ui.label("Scene path (optional)");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.scene_path_input);
                    if ui.button("Browse Scene...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Scene", &["ron"])
                            .pick_file()
                        {
                            self.scene_path_input = path.display().to_string();
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
                                match self.app.open_project(&recent, None) {
                                    Ok(()) => {
                                        self.project_modal_open = false;
                                        self.after_project_switch();
                                    }
                                    Err(err) => {
                                        self.app.status_line =
                                            format!("open recent failed: {err}");
                                    }
                                }
                            }
                        }
                    });
            });

                self.project_modal_open = open;
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

                    if ui.button("Save As autosave_scene.ron").clicked() {
                        let path = self
                            .app
                            .project
                            .project_root
                            .join("assets")
                            .join("autosave_scene.ron");
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
                            match self.app.open_project(&recent, None) {
                                Ok(()) => self.after_project_switch(),
                                Err(err) => {
                                    self.app.status_line =
                                        format!("project switch failed: {err}");
                                }
                            }
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
                            .set_active_scene_graph(self.app.project.document.graph.clone())
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

                if ui.button("Refresh Assets").clicked() {
                    if let Err(err) = self.app.project.refresh_asset_index() {
                        self.app.status_line = format!("asset refresh failed: {err}");
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
                        .set_active_scene_graph(self.app.project.document.graph.clone())
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
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("Assets");
                ui.label(format!("Project: {}", self.app.project.project_root.display()));
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for entry in &self.app.project.asset_index {
                        ui.label(format!("{:?}: {}", entry.kind, entry.path.display()));
                    }
                });
            });
    }

    fn draw_inspector_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("inspector_panel")
            .resizable(true)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.heading("Inspector");
                ui.separator();

                let Some(selected) = self.selected_node else {
                    ui.label("No node selected.");
                    return;
                };

                let maybe_node = self
                    .app
                    .project
                    .document
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == selected)
                    .cloned();

                let Some(node) = maybe_node else {
                    ui.label("Selected node no longer exists.");
                    return;
                };

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
                        for option in [
                            NodeKind::GameplayEvent,
                            NodeKind::GameplayFlow,
                            NodeKind::MathState,
                            NodeKind::ComputePass,
                            NodeKind::RenderPass,
                            NodeKind::AssetReference,
                            NodeKind::BuildExport,
                        ] {
                            ui.selectable_value(&mut kind, option, format!("{:?}", option));
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

                if ui.button("Apply Inspector Changes").clicked() {
                    if let Err(err) = apply_inspector_node_change(
                        &mut self.app.project,
                        selected,
                        Some(kind),
                        Some(target),
                        Some(fallback),
                        None,
                        None,
                    ) {
                        self.app.status_line = format!("inspector apply failed: {err}");
                    } else if let Some(node_mut) = self
                        .app
                        .project
                        .document
                        .graph
                        .nodes
                        .iter_mut()
                        .find(|node| node.id == selected)
                    {
                        node_mut.name = self.inspector_name_buffer.clone();
                        self.app.project.dirty = true;
                    }

                    self.canvas_refresh_and_compile();
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

    fn draw_graph_and_viewport(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |columns| {
                columns[0].heading("Graph Canvas");
                columns[0].label("Add nodes: right-click graph canvas");
                let output = self
                    .app
                    .canvas
                    .show(&mut columns[0], &mut self.app.project.next_node_id, self.selected_node);

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
                }

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

                if columns[1].button("Select Object #1").clicked() {
                    self.app.project.session.viewport.selected_object = Some(1);
                }

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

    fn draw_viewport_preview(&self, ui: &mut egui::Ui) {
        let preview_height = 280.0;
        let available_width = ui.available_width().max(280.0);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(available_width, preview_height),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 8.0, egui::Color32::from_rgb(18, 20, 28));
        painter.rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(56, 64, 84)),
            egui::StrokeKind::Outside,
        );

        painter.text(
            rect.left_top() + egui::vec2(12.0, 10.0),
            egui::Align2::LEFT_TOP,
            "Live Preview",
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );

        let Some(compiled_graph) = self.app.runtime.compiled_graph() else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No compiled scene yet",
                egui::FontId::proportional(16.0),
                egui::Color32::from_rgb(180, 190, 210),
            );
            return;
        };

        let Some(render_pass) = compiled_graph.render_graph.passes.iter().find_map(|pass| match pass {
            RenderGraphPass::Render(render_pass) => Some(render_pass),
            _ => None,
        }) else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No render pass in scene",
                egui::FontId::proportional(16.0),
                egui::Color32::from_rgb(180, 190, 210),
            );
            return;
        };

        let sprites: Vec<_> = render_pass
            .batches
            .iter()
            .flat_map(|batch| batch.sprites.iter())
            .collect();

        if sprites.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Render pass has no sprites",
                egui::FontId::proportional(16.0),
                egui::Color32::from_rgb(180, 190, 210),
            );
            return;
        }

        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;

        for sprite in &sprites {
            min_x = min_x.min(sprite.x);
            min_y = min_y.min(sprite.y);
            max_x = max_x.max(sprite.x + sprite.width.max(1.0));
            max_y = max_y.max(sprite.y + sprite.height.max(1.0));
        }

        let content_width = (max_x - min_x).max(1.0);
        let content_height = (max_y - min_y).max(1.0);
        let scale_x = (rect.width() - 40.0) / content_width;
        let scale_y = (rect.height() - 52.0) / content_height;
        let scale = scale_x.min(scale_y).clamp(0.6, 4.0);

        let base = rect.left_top() + egui::vec2(20.0, 34.0);
        let center_x = min_x + content_width * 0.5;
        let center_y = min_y + content_height * 0.5;
        let preview_center = rect.center() + egui::vec2(0.0, 8.0);

        let shape_count = render_pass
            .batches
            .iter()
            .map(|batch| batch.sprites.len())
            .sum::<usize>();
        painter.text(
            rect.left_top() + egui::vec2(12.0, 26.0),
            egui::Align2::LEFT_TOP,
            format!("{} sprites", shape_count),
            egui::FontId::monospace(12.0),
            egui::Color32::from_rgb(155, 170, 190),
        );

        painter.line_segment(
            [
                egui::pos2(base.x, rect.bottom() - 16.0),
                egui::pos2(rect.right() - 16.0, rect.bottom() - 16.0),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(44, 51, 68)),
        );

        for (index, sprite) in sprites.iter().enumerate() {
            let x = preview_center.x + (sprite.x - center_x) * scale;
            let y = preview_center.y + (sprite.y - center_y) * scale;
            let width = sprite.width.max(8.0) * scale;
            let height = sprite.height.max(8.0) * scale;
            let sprite_rect = egui::Rect::from_center_size(
                egui::pos2(x, y),
                egui::vec2(width, height),
            );

            let tint = egui::Color32::from_rgb(
                (60 + ((index * 37) % 170)) as u8,
                (110 + ((index * 53) % 120)) as u8,
                (150 + ((index * 29) % 90)) as u8,
            );

            match sprite.texture.0 % 4 {
                0 => {
                    painter.rect_filled(sprite_rect, 3.0, tint);
                    painter.rect_stroke(
                        sprite_rect,
                        3.0,
                        egui::Stroke::new(1.0, egui::Color32::BLACK),
                        egui::StrokeKind::Outside,
                    );
                }
                1 => {
                    painter.circle_filled(sprite_rect.center(), sprite_rect.width().min(sprite_rect.height()) * 0.5, tint);
                }
                2 => {
                    let top = egui::pos2(sprite_rect.center().x, sprite_rect.top());
                    let left = egui::pos2(sprite_rect.left(), sprite_rect.bottom());
                    let right = egui::pos2(sprite_rect.right(), sprite_rect.bottom());
                    painter.add(egui::Shape::convex_polygon(
                        vec![top, right, left],
                        tint,
                        egui::Stroke::new(1.0, egui::Color32::BLACK),
                    ));
                }
                _ => {
                    let top = egui::pos2(sprite_rect.center().x, sprite_rect.top());
                    let right = egui::pos2(sprite_rect.right(), sprite_rect.center().y);
                    let bottom = egui::pos2(sprite_rect.center().x, sprite_rect.bottom());
                    let left = egui::pos2(sprite_rect.left(), sprite_rect.center().y);
                    painter.add(egui::Shape::convex_polygon(
                        vec![top, right, bottom, left],
                        tint,
                        egui::Stroke::new(1.0, egui::Color32::BLACK),
                    ));
                }
            }
        }
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

                    for event in snapshot.backend_diagnostics.events.iter().rev().take(24) {
                        ui.label(format!("backend {:?}: {}", event.level, event.message));
                    }
                });
            });
    }

    fn canvas_refresh_and_compile(&mut self) {
        self.app.canvas.rebuild_from_document(&self.app.project.document);
        match self
            .app
            .runtime
            .set_active_scene_graph(self.app.project.document.graph.clone())
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
        if self.project_modal_open {
            self.draw_project_modal(ctx);
            return;
        }

        if let Err(err) = self.app.runtime.run_for_frames(1) {
            self.app.status_line = format!("runtime frame failed: {err}");
        }

        self.draw_top_menu(ctx);
        self.draw_assets_panel(ctx);
        self.draw_inspector_panel(ctx);
        self.draw_diagnostics_panel(ctx);
        self.draw_graph_and_viewport(ctx);

        if let Err(err) = self.app.project.autosave_if_dirty() {
            self.app.status_line = format!("autosave failed: {err}");
        }
    }
}
