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
    /// Switch to a configured profile.
    Switch { profile: String },
    /// Reload config.ini without restarting the daemon.
    Reload,
    /// Print daemon state.
    Status,
    /// Print the configuration file path, creating the default config if needed.
    Config,
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
        Commands::Config => println!("{}", config::ensure_config()?.display()),
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
        Commands::Reload => send("RELOAD")?,
        Commands::Status => send("STATUS")?,
        Commands::Switch { profile } => send(&format!("SWITCH {profile}"))?,
        Commands::Daemon => daemon::Daemon::new()?.run()?,
    }
    Ok(())
}

fn send(command: &str) -> Result<()> {
    println!("{}", ipc::send_command(command)?);
    Ok(())
}
