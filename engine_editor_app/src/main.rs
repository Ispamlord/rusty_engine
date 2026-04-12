use std::path::PathBuf;

use engine_editor_app::{EditorApp, EditorAppConfig};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CliArgs {
    project: Option<PathBuf>,
    scene: Option<PathBuf>,
    smoke: bool,
}

fn parse_cli_args(args: impl IntoIterator<Item = String>) -> Result<CliArgs, String> {
    let mut parsed = CliArgs::default();
    let mut positional_project: Option<PathBuf> = None;

    let mut iter = args.into_iter();
    let _ = iter.next();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--project" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--project requires a path".to_string())?;
                parsed.project = Some(PathBuf::from(value));
            }
            "--scene" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--scene requires a path".to_string())?;
                parsed.scene = Some(PathBuf::from(value));
            }
            "--smoke" => {
                parsed.smoke = true;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unknown flag: {arg}"));
            }
            _ => {
                if positional_project.is_none() {
                    positional_project = Some(PathBuf::from(arg));
                } else {
                    return Err("only one positional project path is supported".to_string());
                }
            }
        }
    }

    if parsed.project.is_none() {
        parsed.project = positional_project;
    }

    Ok(parsed)
}

fn print_help() {
    println!(
        "Rusty Engine Editor\n\nUsage:\n  cargo run -p engine_editor_app -- [project_path] [--project <path>] [--scene <path>] [--smoke]\n\nOptions:\n  --project <path>  Project root directory\n  --scene <path>    Scene file to open (.scene.ron)\n  --smoke           Headless startup smoke mode\n  -h, --help        Show this message"
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_cli_args(std::env::args()).map_err(|err| {
        let message = format!("{err}\nUse --help for usage.");
        std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
    })?;

    let project_path = args
        .project
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let mut app = EditorApp::new(project_path, EditorAppConfig::default())?;

    if let Some(scene) = args.scene {
        app.open_scene(scene)?;
    }

    if args.smoke {
        app.run_smoke(3)?;
        return Ok(());
    }

    app.run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_project_scene_and_smoke() {
        let parsed = parse_cli_args([
            "editor".to_string(),
            "--project".to_string(),
            "demo".to_string(),
            "--scene".to_string(),
            "assets/scene.ron".to_string(),
            "--smoke".to_string(),
        ])
        .expect("args should parse");

        assert_eq!(parsed.project, Some(PathBuf::from("demo")));
        assert_eq!(parsed.scene, Some(PathBuf::from("assets/scene.ron")));
        assert!(parsed.smoke);
    }

    #[test]
    fn positional_project_is_supported() {
        let parsed = parse_cli_args(["editor".to_string(), "my_project".to_string()])
            .expect("args should parse");
        assert_eq!(parsed.project, Some(PathBuf::from("my_project")));
    }
}
