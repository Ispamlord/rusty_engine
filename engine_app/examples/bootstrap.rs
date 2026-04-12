use std::path::Path;

use engine_app::EngineApp;
use engine_core::EngineConfig;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config/default.ron".to_string());
    let scene_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "assets/sample_scene.scene.ron".to_string());

    let mut app = if Path::new(&config_path).exists() {
        EngineApp::from_config_path(&config_path)?
    } else {
        EngineApp::new(EngineConfig::default())?
    };

    if Path::new(&scene_path).exists() {
        app.load_scene(&scene_path)?;
    }

    app.run_for_frames(3)?;

    println!("Active backend: {:?}", app.active_backend());
    if let Some(compiled) = app.compiled_graph() {
        println!(
            "Compiled graph jobs: {}, gpu passes: {}, diagnostics: {}",
            compiled.ecs_jobs.len(),
            compiled.gpu_passes.len(),
            compiled.diagnostics.len()
        );
    }

    Ok(())
}
