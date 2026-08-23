use crate::config::{load_config, Config, PlaylistOrder};
use crate::engine::WallpaperEngine;
use crate::ipc::{create_listener, get_pid_path, get_socket_path};
use anyhow::{Context, Result};
use rand::seq::SliceRandom;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "heic", "bmp", "webp", "gif"];
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "webm"];

pub struct Daemon {
    config: Config,
    engine: WallpaperEngine,
    playlist: Vec<PathBuf>,
    current_idx: Option<usize>,
    paused: bool,
    last_switch: Instant,
}

impl Daemon {
    pub fn new() -> Result<Self> {
        let config = load_config()?;
        let mut daemon = Self {
            config,
            engine: WallpaperEngine::new(),
            playlist: Vec::new(),
            current_idx: None,
            paused: false,
            last_switch: Instant::now(),
        };
        daemon.reload_playlist();
        Ok(daemon)
    }

    fn reload_playlist(&mut self) {
        self.playlist.clear();
        if let Some(profile) = self.config.profiles.get(&self.config.active_profile) {
            if let Ok(entries) = fs::read_dir(&profile.folder) {
                self.playlist = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.is_file()
                            && path
                                .extension()
                                .and_then(|ext| ext.to_str())
                                .is_some_and(|ext| {
                                    let ext = ext.to_ascii_lowercase();
                                    IMAGE_EXTENSIONS.contains(&ext.as_str())
                                        || VIDEO_EXTENSIONS.contains(&ext.as_str())
                                })
                    })
                    .collect();
            }
            match profile.order {
                PlaylistOrder::Random => self.playlist.shuffle(&mut rand::thread_rng()),
                PlaylistOrder::Alphabetical => self.playlist.sort_unstable(),
            }
        }
        self.current_idx = None;
    }

    fn apply_index(&mut self, index: usize) -> Result<()> {
        let path = self
            .playlist
            .get(index)
            .context("no wallpapers found in the active profile")?;
        let mute = self
            .config
            .profiles
            .get(&self.config.active_profile)
            .map(|profile| profile.mute_video)
            .unwrap_or(true);
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
            self.engine.set_static(path)?;
        } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
            self.engine.set_video(path, mute)?;
        }
        self.current_idx = Some(index);
        self.last_switch = Instant::now();
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        let length = self.playlist.len();
        let index = self.current_idx.map_or(0, |current| (current + 1) % length);
        self.apply_index(index)
    }

    fn previous(&mut self) -> Result<()> {
        let length = self.playlist.len();
        let index = self
            .current_idx
            .map_or(0, |current| (current + length - 1) % length);
        self.apply_index(index)
    }

    pub fn run(&mut self) -> Result<()> {
        self.engine.save_original_wallpaper()?;
        let listener = create_listener()?;
        fs::write(get_pid_path(), std::process::id().to_string())?;
        let run_result = self.run_loop(&listener);
        let shutdown_result = if self.config.restore_on_exit {
            self.engine.restore_original()
        } else {
            self.engine.stop_video();
            Ok(())
        };
        let _ = fs::remove_file(get_socket_path());
        let _ = fs::remove_file(get_pid_path());
        run_result?;
        shutdown_result
    }

    fn run_loop(&mut self, listener: &std::os::unix::net::UnixListener) -> Result<()> {
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                    let mut command = String::new();
                    stream.read_to_string(&mut command)?;
                    let stop = command.trim() == "STOP";
                    let response = self.handle_ipc(command.trim());
                    stream.write_all(response.as_bytes())?;
                    if stop {
                        return Ok(());
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
            let interval = self
                .config
                .profiles
                .get(&self.config.active_profile)
                .map(|profile| profile.interval)
                .unwrap_or(300);
            if !self.paused
                && !self.playlist.is_empty()
                && self.last_switch.elapsed() >= Duration::from_secs(interval)
            {
                let _ = self.next();
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    fn handle_ipc(&mut self, command: &str) -> String {
        let mut parts = command.split_whitespace();
        match parts.next().unwrap_or("") {
            "PING" => "PONG".into(),
            "PAUSE" => {
                self.paused = true;
                "Wallpaper switching paused.".into()
            }
            "RESUME" => {
                self.paused = false;
                "Wallpaper switching resumed.".into()
            }
            "NEXT" => self
                .next()
                .map(|_| "Switched to next wallpaper.".into())
                .unwrap_or_else(|error| format!("Error: {error:#}")),
            "PREV" => self
                .previous()
                .map(|_| "Switched to previous wallpaper.".into())
                .unwrap_or_else(|error| format!("Error: {error:#}")),
            "RELOAD" => {
                self.config = match load_config() {
                    Ok(config) => config,
                    Err(error) => return format!("Error: {error:#}"),
                };
                self.reload_playlist();
                self.next()
                    .map(|_| "Configuration reloaded.".into())
                    .unwrap_or_else(|error| format!("Configuration reloaded, but {error:#}"))
            }
            "SWITCH" => match parts.next() {
                Some(profile_name) if self.config.profiles.contains_key(profile_name) => {
                    self.config.active_profile = profile_name.to_owned();
                    self.reload_playlist();
                    self.next()
                        .map(|_| format!("Switched to profile '{profile_name}'."))
                        .unwrap_or_else(|error| format!("Error: {error:#}"))
                }
                Some(profile_name) => {
                    format!("Error: profile '{profile_name}' was not found in config.ini")
                }
                None => "Usage: wallctl switch <profile>".into(),
            },
            "STATUS" => {
                let current = self
                    .current_idx
                    .and_then(|index| self.playlist.get(index))
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_else(|| "None".into());
                format!(
                    "Profile: {}\nPaused: {}\nTotal items: {}\nCurrent: {}",
                    self.config.active_profile,
                    self.paused,
                    self.playlist.len(),
                    current
                )
            }
            "STOP" => "Stopping daemon and restoring wallpaper...".into(),
            _ => "Unknown command.".into(),
        }
    }
}
