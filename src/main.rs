mod config;
mod daemon;
mod engine;
mod ipc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "wallctl",
    version,
    about = "A lightweight wallpaper rotation daemon"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the wallpaper daemon.
    Start,
    /// Stop the daemon and restore the wallpaper from before it started.
    Stop,
    /// Pause automatic switching.
    Pause,
    /// Resume automatic switching.
    Resume,
    /// Switch to the next wallpaper.
    Next,
    /// Switch to the previous wallpaper.
    Prev,
    /// List the active profile's wallpapers and their one-based indexes.
    List,
    /// Set a wallpaper by its one-based index from `wallctl list`.
    Set { index: usize },
    /// Switch to a configured profile.
    Switch { profile: String },
    /// Reload config.ini without restarting the daemon.
    Reload,
    /// Print daemon state.
    Status,
    /// Open the configuration folder.
    Config {
        /// Print the configuration file path without opening an editor.
        #[arg(long)]
        path: bool,
    },
    #[command(hide = true)]
    Daemon,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("wallctl: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Config { path } => {
            let config_path = config::ensure_config()?;
            if path {
                println!("{}", config_path.display());
            } else {
                open_config_folder(&config_path)?;
            }
        }
        Commands::Start => {
            config::load_config()?;
            if ipc::is_daemon_running() {
                println!("Wallpaper daemon is already running.");
                return Ok(());
            }
            ipc::cleanup_stale_files();
            let current_exe =
                std::env::current_exe().context("could not find wallctl executable")?;
            let mut child = Command::new(current_exe)
                .arg("daemon")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("could not start wallpaper daemon")?;
            for _ in 0..20 {
                if ipc::is_daemon_running() {
                    println!("Wallpaper daemon started in the background.");
                    return Ok(());
                }
                if child.try_wait()?.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            anyhow::bail!("wallpaper daemon exited before it became ready; run `wallctl daemon` to inspect the error")
        }
        Commands::Stop => {
            if ipc::is_daemon_running() {
                println!("{}", ipc::send_command("STOP")?);
            } else {
                let mut engine = engine::WallpaperEngine::new();
                engine.restore_original()?;
                ipc::cleanup_stale_files();
                println!("Daemon was not running; restored wallpaper state if available.");
            }
        }
        Commands::Pause => send("PAUSE")?,
        Commands::Resume => send("RESUME")?,
        Commands::Next => send("NEXT")?,
        Commands::Prev => send("PREV")?,
        Commands::List => send("LIST")?,
        Commands::Set { index } => send(&format!("SET {index}"))?,
        Commands::Reload => send("RELOAD")?,
        Commands::Status => send("STATUS")?,
        Commands::Switch { profile } => send(&format!("SWITCH {profile}"))?,
        Commands::Daemon => daemon::Daemon::new()?.run()?,
    }
    Ok(())
}

fn open_config_folder(config_path: &std::path::Path) -> Result<()> {
    let folder = config_path
        .parent()
        .context("config file has no parent directory")?;
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    anyhow::bail!("opening the configuration folder is not supported on this platform");

    Command::new(opener)
        .arg(folder)
        .spawn()
        .with_context(|| format!("could not open {} with {opener}", folder.display()))?;
    Ok(())
}

fn send(command: &str) -> Result<()> {
    let response = ipc::send_command(command)?;
    if let Some(message) = response.strip_prefix("Error: ") {
        anyhow::bail!("{message}");
    }
    println!("{response}");
    Ok(())
}
