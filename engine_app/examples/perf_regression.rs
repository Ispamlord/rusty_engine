use std::fs;
use std::path::Path;
use std::time::Instant;

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
        let _ = app.load_scene(&scene_path);
    }

    app.set_frame_pacing_sleep_enabled(false);

    let warmup_frames = std::env::var("PERF_WARMUP_FRAMES")
        .ok()
        .and_then(|raw| raw.parse::<u32>().ok())
        .unwrap_or(30);

    let sample_frames = std::env::var("PERF_SAMPLE_FRAMES")
        .ok()
        .and_then(|raw| raw.parse::<u32>().ok())
        .unwrap_or(180);

    for _ in 0..warmup_frames {
        app.run_for_frames(1)?;
    }

    let mut app_wall_samples = Vec::with_capacity(sample_frames as usize);
    let mut runtime_cpu_samples = Vec::with_capacity(sample_frames as usize);
    let mut backend_cpu_samples = Vec::with_capacity(sample_frames as usize);

    for _ in 0..sample_frames {
        let frame_start = Instant::now();
        app.run_for_frames(1)?;
        app_wall_samples.push(frame_start.elapsed().as_secs_f64() * 1000.0);
        runtime_cpu_samples.push(app.cpu_frame_ms() as f64);
        backend_cpu_samples.push(app.backend_diagnostics().last_cpu_frame_ms as f64);
    }

    let (avg, p95) = summarize_stats(&mut app_wall_samples);
    let (runtime_avg, runtime_p95) = summarize_stats(&mut runtime_cpu_samples);
    let (backend_avg, backend_p95) = summarize_stats(&mut backend_cpu_samples);

    let metrics = format!(
        "backend={:?}\nwarmup_frames={}\nsample_frames={}\navg_frame_ms={:.4}\np95_frame_ms={:.4}\n",
        app.active_backend(),
        warmup_frames,
        sample_frames,
        avg,
        p95,
    );

    let metrics = format!(
        "{metrics}avg_runtime_cpu_ms={runtime_avg:.4}\np95_runtime_cpu_ms={runtime_p95:.4}\navg_backend_cpu_ms={backend_avg:.4}\np95_backend_cpu_ms={backend_p95:.4}\n"
    );

    let metrics_out = std::env::var("PERF_METRICS_OUT")
        .unwrap_or_else(|_| "target/perf/perf_metrics.txt".to_string());
    if let Some(parent) = Path::new(&metrics_out).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&metrics_out, metrics)?;

    let max_avg_ms = std::env::var("PERF_MAX_AVG_FRAME_MS")
        .ok()
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or(16.67);
    if avg > max_avg_ms {
        return Err(format!(
            "perf regression: avg_frame_ms {:.4} exceeds threshold {:.4}",
            avg, max_avg_ms
        )
        .into());
    }

    if let Ok(baseline_path) = std::env::var("PERF_BASELINE_PATH") {
        if Path::new(&baseline_path).exists() {
            let baseline = fs::read_to_string(&baseline_path)?;
            if let Some(base_avg) = parse_value(&baseline, "avg_frame_ms") {
                let allowed_ratio = std::env::var("PERF_BASELINE_RATIO")
                    .ok()
                    .and_then(|raw| raw.parse::<f64>().ok())
                    .unwrap_or(1.10);
                if avg > base_avg * allowed_ratio {
                    return Err(format!(
                        "perf regression: avg {:.4} exceeds baseline {:.4} * ratio {:.2}",
                        avg, base_avg, allowed_ratio
                    )
                    .into());
                }
            }
        }
    }

    println!(
        "perf_regression: backend={:?}, app_avg_ms={:.4}, app_p95_ms={:.4}, runtime_cpu_avg_ms={:.4}, runtime_cpu_p95_ms={:.4}, backend_cpu_avg_ms={:.4}, backend_cpu_p95_ms={:.4}, out={}",
        app.active_backend(),
        avg,
        p95,
        runtime_avg,
        runtime_p95,
        backend_avg,
        backend_p95,
        metrics_out
    );

    Ok(())
}

fn parse_value(content: &str, key: &str) -> Option<f64> {
    content.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        if k.trim() == key {
            v.trim().parse::<f64>().ok()
        } else {
            None
        }
    })
}

fn summarize_stats(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let avg = if samples.is_empty() {
        0.0
    } else {
        samples.iter().sum::<f64>() / samples.len() as f64
    };

    let p95_index = ((samples.len() as f64) * 0.95).floor() as usize;
    let p95 = samples
        .get(p95_index.min(samples.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0.0);

    (avg, p95)
}
