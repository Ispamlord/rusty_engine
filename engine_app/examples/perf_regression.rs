use std::fs;
use std::path::Path;
use std::time::Instant;

use engine_app::EngineApp;
use engine_core::EngineConfig;
use engine_render_api::BackendKind;

#[derive(Debug, Clone, Copy)]
struct PerfThresholds {
    max_avg_frame_ms: f64,
    max_p95_frame_ms: f64,
    max_avg_runtime_cpu_ms: f64,
    max_p95_runtime_cpu_ms: f64,
    max_avg_backend_cpu_ms: f64,
    max_p95_backend_cpu_ms: f64,
}

impl PerfThresholds {
    /// Default thresholds tuned per backend.
    ///
    /// DX11 is a compatibility path and is expected to be slightly slower on
    /// the CPU side because of its older API model. DX12 and Vulkan share the
    /// same defaults because they are the primary production backends.
    fn defaults_for(backend: BackendKind) -> Self {
        match backend {
            BackendKind::Dx11 => Self {
                max_avg_frame_ms: 16.67,
                max_p95_frame_ms: 20.0,
                max_avg_runtime_cpu_ms: 8.0,
                max_p95_runtime_cpu_ms: 12.0,
                max_avg_backend_cpu_ms: 4.0,
                max_p95_backend_cpu_ms: 6.0,
            },
            BackendKind::Dx12 | BackendKind::Vulkan => Self {
                max_avg_frame_ms: 16.67,
                max_p95_frame_ms: 20.0,
                max_avg_runtime_cpu_ms: 6.0,
                max_p95_runtime_cpu_ms: 10.0,
                max_avg_backend_cpu_ms: 3.0,
                max_p95_backend_cpu_ms: 5.0,
            },
        }
    }

    /// Loads thresholds from environment, falling back to per-backend defaults.
    fn from_env_or_defaults(backend: BackendKind) -> Self {
        let defaults = Self::defaults_for(backend);
        Self {
            max_avg_frame_ms: parse_env_f64("PERF_MAX_AVG_FRAME_MS", defaults.max_avg_frame_ms),
            max_p95_frame_ms: parse_env_f64(
                "PERF_MAX_P95_FRAME_MS",
                defaults.max_p95_frame_ms,
            ),
            max_avg_runtime_cpu_ms: parse_env_f64(
                "PERF_MAX_AVG_RUNTIME_CPU_MS",
                defaults.max_avg_runtime_cpu_ms,
            ),
            max_p95_runtime_cpu_ms: parse_env_f64(
                "PERF_MAX_P95_RUNTIME_CPU_MS",
                defaults.max_p95_runtime_cpu_ms,
            ),
            max_avg_backend_cpu_ms: parse_env_f64(
                "PERF_MAX_AVG_BACKEND_CPU_MS",
                defaults.max_avg_backend_cpu_ms,
            ),
            max_p95_backend_cpu_ms: parse_env_f64(
                "PERF_MAX_P95_BACKEND_CPU_MS",
                defaults.max_p95_backend_cpu_ms,
            ),
        }
    }
}

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

    let backend = app.active_backend();
    let thresholds = PerfThresholds::from_env_or_defaults(backend);

    let metrics = format!(
        "backend={:?}\nwarmup_frames={}\nsample_frames={}\navg_frame_ms={:.4}\np95_frame_ms={:.4}\n",
        backend,
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

    let mut failures = Vec::new();
    check_threshold(
        &mut failures,
        "avg_frame_ms",
        avg,
        thresholds.max_avg_frame_ms,
    );
    check_threshold(
        &mut failures,
        "p95_frame_ms",
        p95,
        thresholds.max_p95_frame_ms,
    );
    check_threshold(
        &mut failures,
        "avg_runtime_cpu_ms",
        runtime_avg,
        thresholds.max_avg_runtime_cpu_ms,
    );
    check_threshold(
        &mut failures,
        "p95_runtime_cpu_ms",
        runtime_p95,
        thresholds.max_p95_runtime_cpu_ms,
    );
    check_threshold(
        &mut failures,
        "avg_backend_cpu_ms",
        backend_avg,
        thresholds.max_avg_backend_cpu_ms,
    );
    check_threshold(
        &mut failures,
        "p95_backend_cpu_ms",
        backend_p95,
        thresholds.max_p95_backend_cpu_ms,
    );

    if let Ok(baseline_path) = std::env::var("PERF_BASELINE_PATH") {
        if Path::new(&baseline_path).exists() {
            let baseline = fs::read_to_string(&baseline_path)?;
            if let Some(base_avg) = parse_value(&baseline, "avg_frame_ms") {
                let allowed_ratio = std::env::var("PERF_BASELINE_RATIO")
                    .ok()
                    .and_then(|raw| raw.parse::<f64>().ok())
                    .unwrap_or(1.10);
                if avg > base_avg * allowed_ratio {
                    failures.push(format!(
                        "avg_frame_ms {:.4} exceeds baseline {:.4} * ratio {:.2}",
                        avg, base_avg, allowed_ratio
                    ));
                }
            }
        }
    }

    println!(
        "perf_regression: backend={:?}, app_avg_ms={:.4}, app_p95_ms={:.4}, runtime_cpu_avg_ms={:.4}, runtime_cpu_p95_ms={:.4}, backend_cpu_avg_ms={:.4}, backend_cpu_p95_ms={:.4}, out={}",
        backend,
        avg,
        p95,
        runtime_avg,
        runtime_p95,
        backend_avg,
        backend_p95,
        metrics_out
    );

    if !failures.is_empty() {
        return Err(format!("perf regression: {}", failures.join("; ")).into());
    }

    Ok(())
}

fn check_threshold(failures: &mut Vec<String>, name: &str, value: f64, threshold: f64) {
    if value > threshold {
        failures.push(format!(
            "{} {:.4} exceeds threshold {:.4}",
            name, value, threshold
        ));
    }
}

fn parse_env_f64(var: &str, default: f64) -> f64 {
    std::env::var(var)
        .ok()
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or(default)
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
