use std::path::Path;

use engine_app::EngineApp;
use engine_core::EngineConfig;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config/default.ron".to_string());
    let scene_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "assets/sample_scene.ron".to_string());

    let mut app = if Path::new(&config_path).exists() {
        EngineApp::from_config_path(&config_path)?
    } else {
        EngineApp::new(EngineConfig::default())?
    };

    if Path::new(&scene_path).exists() {
        app.load_scene(&scene_path)?;
    }

    app.set_frame_pacing_sleep_enabled(false);

    let frames = 180_u32;
    let start = std::time::Instant::now();
    app.run_for_frames(frames)?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let avg_frame_ms = elapsed_ms / f64::from(frames);

    let max_frame_ms = std::env::var("PERF_MAX_FRAME_MS")
        .ok()
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or(16.67);

    println!(
        "perf_smoke: backend={:?}, frames={}, total_ms={:.3}, avg_frame_ms={:.3}, threshold_ms={:.3}",
        app.active_backend(),
        frames,
        elapsed_ms,
        avg_frame_ms,
        max_frame_ms
    );

    if avg_frame_ms > max_frame_ms {
        return Err(format!(
            "Performance gate failed: avg frame {:.3} ms exceeds threshold {:.3} ms",
            avg_frame_ms, max_frame_ms
        )
        .into());
    }

    Ok(())
}
