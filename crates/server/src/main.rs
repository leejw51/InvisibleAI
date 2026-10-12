use anyhow::{Context, Result, ensure};
use causewaybay_server::{AppState, app, startup};
use causewaybay_shared::config;
use clap::{Parser, Subcommand};
use serde_json::json;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "causewaybay-server",
    version,
    about = "Causewaybay backend: OCR, AI, SQLite, and WebSocket routing"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    Serve(Serve),
}
#[derive(clap::Args, Default)]
struct Serve {
    #[arg(
        long,
        env = "BIND_ADDR",
        help = "Listen address (default: 127.0.0.1:3000, this Mac only)"
    )]
    bind: Option<String>,
    #[arg(
        long,
        env = "DATA_DIR",
        help = "Temporary launch override for data (default: <backend folder>/data); never saved in settings"
    )]
    data_dir: Option<PathBuf>,
    #[arg(long, env = "DIST_DIR")]
    assets: Option<PathBuf>,
    #[arg(
        long,
        env = "CAUSEWAYBAY_CONFIG_DIR",
        help = "Launch override for the backend folder (default: ~/.causewaybaybot); never saved in settings"
    )]
    config_dir: Option<PathBuf>,
    #[arg(long, env = "APP_TOKEN", hide_env_values = true)]
    token: Option<String>,
    #[arg(long, help = "Use explicit demo fixtures; never call cloud providers")]
    demo: bool,
    #[arg(long, help = "Disable local semantic embeddings")]
    no_semantic: bool,
}
fn main() -> Result<()> {
    // Load before starting Tokio or any other threads: dotenv mutates the environment.
    load_env_file(std::path::Path::new(".env"))?;
    let mut args = match Args::parse().command {
        Some(Command::Serve(v)) => v,
        None => Serve::default(),
    };
    let dir = match args.config_dir.take().filter(|p| !p.as_os_str().is_empty()) {
        Some(dir) => dir,
        None => config::directory()?,
    };
    // Resolve once: a .env inside the backend folder cannot relocate that folder.
    load_env_file(&dir.join(".env"))?;
    args.config_dir = Some(dir);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(args))
}

fn load_env_file(path: &std::path::Path) -> Result<()> {
    match dotenvy::from_path(path) {
        Ok(()) => Ok(()),
        Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        // dotenv's parse errors contain the original line, which may contain a key.
        Err(_) => anyhow::bail!(
            "Could not load {}. Check permissions and dotenv syntax.",
            path.display()
        ),
    }
}

async fn run(args: Serve) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "causewaybay_server=info,tower_http=info".into()),
        )
        .init();
    let dir = args.config_dir.context("Backend folder was not resolved")?;
    let stored = config::read(&dir, "backend")?;
    let bind = args
        .bind
        .or_else(|| std::env::var("BIND_ADDR").ok())
        .or_else(|| stored["bind"].as_str().map(str::to_string))
        .unwrap_or_else(|| "127.0.0.1:3000".into());
    let data = data_directory(
        &dir,
        args.data_dir.or_else(|| {
            std::env::var_os("DATA_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        }),
        &stored,
    )?;
    let assets = args
        .assets
        .or_else(|| std::env::var_os("DIST_DIR").map(PathBuf::from))
        .or_else(|| {
            stored["assets"]
                .as_str()
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
        })
        .unwrap_or_else(|| causewaybay_server::local_ai::distribution_root().join("dist"));
    let token = args
        .token
        .or_else(|| std::env::var("APP_TOKEN").ok())
        .filter(|v| !v.is_empty());
    if let Some(t) = &token {
        anyhow::ensure!(
            t.len() >= 16
                && t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
            "APP_TOKEN must be at least 16 URL-safe characters"
        );
    }
    let listen_address: std::net::SocketAddr = bind
        .parse()
        .context("Use an IP address and port for --bind")?;
    // Without wallet sign-in or a token, a remote caller could claim a
    // localhost Host header, so only this Mac may listen.
    let wallet_access = std::env::var("CWBBOT_ADMIN").is_ok_and(|v| !v.trim().is_empty());
    anyhow::ensure!(
        listen_address.ip().is_loopback() || token.is_some() || wallet_access,
        "Remote access requires CWBBOT_ADMIN or APP_TOKEN. Use 127.0.0.1:3000 for this Mac only."
    );
    let demo = args.demo || std::env::var("DEMO_MODE").is_ok_and(|v| v == "1");
    let no_semantic =
        args.no_semantic || std::env::var("SEMANTIC_SEARCH").is_ok_and(|v| v == "off");
    let abs_assets = std::fs::canonicalize(&assets)
        .unwrap_or_else(|_| std::env::current_dir().unwrap().join(&assets));
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let state = AppState::open_configured(data.clone(), dir.clone(), demo, token)?;
    let abs_data = std::fs::canonicalize(&data)?;
    config::append(&dir, "backend", json!({"bind":bind,"assets":abs_assets}))?;
    state.start_indexer(no_semantic);
    causewaybay_server::uploads::start_sweeper(state.clone());
    causewaybay_server::wiki::repair_later(&state);
    println!(
        "Config: {}\nDatabase: {}",
        dir.join("backend.jsonl").display(),
        abs_data.join("causewaybay.sqlite3").display()
    );
    let addresses = startup::print_urls(listener.local_addr()?).await;
    let _ = state.addresses.set(addresses);
    let runtime = state.local_runtime.clone();
    axum::serve(listener, app(state, assets))
        .with_graceful_shutdown(async {
            // Ctrl-C in a terminal, SIGTERM from `make stop` or launchd: both
            // end the server cleanly so the on-device workers stop with it.
            let terminate = async {
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(mut signal) => {
                        signal.recv().await;
                    }
                    Err(_) => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate => {}
            }
        })
        .await?;
    runtime.mlx.shutdown();
    println!("Stopped; the on-device worker was shut down.");
    Ok(())
}

fn data_directory(
    root: &std::path::Path,
    launch_override: Option<PathBuf>,
    stored: &serde_json::Value,
) -> Result<PathBuf> {
    if let Some(path) = launch_override.filter(|p| !p.as_os_str().is_empty()) {
        return Ok(path);
    }
    let data = root.join("data");
    // Older releases remembered --data-dir. Do not silently hide an existing archive
    // when switching to the fixed default, and never move a possibly live database.
    if let Some(old) = stored["data_dir"].as_str().filter(|v| !v.is_empty()) {
        let old = std::path::Path::new(old);
        let same = old == data
            || old
                .canonicalize()
                .ok()
                .zip(data.canonicalize().ok())
                .is_some_and(|(a, b)| a == b);
        if !same {
            match std::fs::read_dir(old) {
                Ok(mut entries) => ensure!(
                    entries.next().transpose()?.is_none(),
                    "Existing backend data is in {}. Stop the backend and move the complete data folder to {} before restarting, or explicitly pass --data-dir {} to keep using it for this launch. No files were moved or deleted.",
                    old.display(),
                    data.display(),
                    old.display()
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).context("Could not check the previous backend data folder");
                }
            }
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_defaults_to_backend_folder_and_does_not_reuse_saved_overrides() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join(".causewaybaybot");
        let old = temp.path().join("old");
        let stored = json!({"data_dir":old});
        assert_eq!(data_directory(&root, None, &stored)?, root.join("data"));
        std::fs::create_dir(&old)?;
        assert_eq!(data_directory(&root, None, &stored)?, root.join("data"));
        assert!(!root.exists());
        Ok(())
    }

    #[test]
    fn old_archives_require_explicit_migration_without_changing_either_folder() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join(".causewaybaybot");
        let old = temp.path().join("old");
        std::fs::create_dir_all(&old)?;
        std::fs::write(old.join("causewaybay.sqlite3"), b"existing archive")?;
        let stored = json!({"data_dir":old});
        let error = data_directory(&root, None, &stored).unwrap_err();
        assert!(error.to_string().contains("No files were moved or deleted"));
        assert!(!root.exists());
        assert_eq!(
            std::fs::read(old.join("causewaybay.sqlite3"))?,
            b"existing archive"
        );
        assert_eq!(data_directory(&root, Some(old.clone()), &stored)?, old);
        Ok(())
    }

    #[test]
    fn existing_default_archive_keeps_working() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let data = temp.path().join("data");
        std::fs::create_dir_all(&data)?;
        std::fs::write(data.join("causewaybay.sqlite3"), b"default archive")?;
        assert_eq!(
            data_directory(temp.path(), None, &json!({"data_dir":data}))?,
            data
        );
        #[cfg(unix)]
        {
            let alias = temp.path().join("alias");
            std::os::unix::fs::symlink(&data, &alias)?;
            assert_eq!(
                data_directory(temp.path(), None, &json!({"data_dir":alias}))?,
                data
            );
        }
        Ok(())
    }
}
