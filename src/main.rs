mod config;
mod daemon;
mod engine;
mod ipc;

use clap::{Parser, Subcommand};
use std::process::{Command, Stdio};

#[derive(Parser)]
#[command(name = "wallctl", about = "Cross-platform dynamic wallpaper manager")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the wallpaper background daemon
    Start,
    /// Stop the daemon and restore original wallpaper
    Stop,
    /// Pause switching
    Pause,
    /// Resume switching
    Resume,
    /// Skip to next wallpaper
    Next,
    /// Go to previous wallpaper
    Prev,
    /// Switch active profile
    Switch { profile: String },
    /// Print status of current profile & wallpaper
    Status,
    #[command(hide = true)]
    Daemon,
}

fn main() {
    let cli = Cli::parse();

    let _ = config::load_or_create_config();

    match cli.command {
        Commands::Start => {
            if ipc::is_daemon_running() {
                println!("Daemon is already running.");
                return;
            }

            ipc::cleanup_stale_files();

            let current_exe = std::env::current_exe().expect("Failed to get current binary path");
            let _ = Command::new(current_exe)
                .arg("daemon")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("Failed to start daemon in background");

            println!("Wallpaper daemon started in background.");
        }
        Commands::Stop => {
            if ipc::is_daemon_running() {
                match ipc::send_command("STOP") {
                    Ok(res) => println!("{}", res),
                    Err(_) => {
                        ipc::force_kill_daemon();
                        let mut engine = engine::WallpaperEngine::new();
                        engine.restore_original();
                        println!("Daemon stopped and wallpaper restored.");
                    }
                }
            } else {
                ipc::cleanup_stale_files();
                let mut engine = engine::WallpaperEngine::new();
                engine.restore_original();
                println!("Daemon was not running. Wallpaper restored.");
            }
        }
        Commands::Pause => match ipc::send_command("PAUSE") {
            Ok(res) => println!("{}", res),
            Err(e) => eprintln!("{}", e),
        },
        Commands::Resume => match ipc::send_command("RESUME") {
            Ok(res) => println!("{}", res),
            Err(e) => eprintln!("{}", e),
        },
        Commands::Next => match ipc::send_command("NEXT") {
            Ok(res) => println!("{}", res),
            Err(e) => eprintln!("{}", e),
        },
        Commands::Prev => match ipc::send_command("PREV") {
            Ok(res) => println!("{}", res),
            Err(e) => eprintln!("{}", e),
        },
        Commands::Switch { profile } => match ipc::send_command(&format!("SWITCH {}", profile)) {
            Ok(res) => println!("{}", res),
            Err(e) => eprintln!("{}", e),
        },
        Commands::Status => match ipc::send_command("STATUS") {
            Ok(res) => println!("{}", res),
            Err(e) => eprintln!("{}", e),
        },
        Commands::Daemon => {
            let mut daemon = daemon::Daemon::new();
            daemon.run();
        }
    }
}
