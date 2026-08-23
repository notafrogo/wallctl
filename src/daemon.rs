use crate::config::{load_or_create_config, Config};
use crate::engine::WallpaperEngine;
use crate::ipc::{create_listener, get_pid_path, get_socket_path};
use rand::seq::SliceRandom;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct Daemon {
    config: Config,
    engine: WallpaperEngine,
    playlist: Vec<PathBuf>,
    current_idx: usize,
    paused: bool,
    last_switch: Instant,
}

impl Daemon {
    pub fn new() -> Self {
        let config = load_or_create_config();
        let engine = WallpaperEngine::new();
        let mut daemon = Self {
            config,
            engine,
            playlist: Vec::new(),
            current_idx: 0,
            paused: false,
            last_switch: Instant::now() - Duration::from_secs(999999),
        };
        daemon.reload_playlist();
        daemon
    }

    fn reload_playlist(&mut self) {
        self.playlist.clear();
        if let Some(profile) = self.config.profiles.get(&self.config.active_profile) {
            if let Ok(entries) = fs::read_dir(&profile.folder) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                        match ext.to_lowercase().as_str() {
                            "jpg" | "jpeg" | "png" | "heic" | "bmp" | "webp" | "mp4" | "mov" | "gif" | "webm" => {
                                self.playlist.push(path);
                            }
                            _ => {}
                        }
                    }
                }
            }
            if profile.order == "random" {
                let mut rng = rand::thread_rng();
                self.playlist.shuffle(&mut rng);
            } else {
                self.playlist.sort();
            }
        }
        self.current_idx = 0;
    }

    fn apply_current(&mut self) {
        if self.playlist.is_empty() { return; }
        let path = &self.playlist[self.current_idx % self.playlist.len()];
        let mute = self.config.profiles.get(&self.config.active_profile).map(|p| p.mute_video).unwrap_or(true);
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

        match ext.as_str() {
            "jpg" | "jpeg" | "png" | "heic" | "bmp" | "webp" => self.engine.set_static(path),
            "mp4" | "mov" | "gif" | "webm" => self.engine.set_video_or_gif(path, mute),
            _ => {}
        }
        self.last_switch = Instant::now();
    }

    pub fn run(&mut self) {
        let listener = create_listener().expect("Failed to bind socket");
        let pid_path = get_pid_path();
        let _ = fs::write(&pid_path, std::process::id().to_string());

        loop {
            // Check for incoming IPC connections
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));

                let mut buf = Vec::new();
                if stream.read_to_end(&mut buf).is_ok() {
                    let cmd = String::from_utf8_lossy(&buf).trim().to_string();
                    let is_stop = cmd == "STOP";
                    let response = self.handle_ipc(&cmd);
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    if is_stop {
                        break;
                    }
                }
            }

            // Interval-based rotation
            let interval = self.config.profiles
                .get(&self.config.active_profile)
                .map(|p| p.interval)
                .unwrap_or(300);

            if !self.paused && !self.playlist.is_empty() && self.last_switch.elapsed() >= Duration::from_secs(interval) {
                self.apply_current();
                self.current_idx = (self.current_idx + 1) % self.playlist.len();
            }

            std::thread::sleep(Duration::from_millis(200));
        }

        // Cleanup and restore
        if self.config.restore_on_exit {
            self.engine.restore_original();
        } else {
            self.engine.stop_video();
        }
        let _ = fs::remove_file(get_socket_path());
        let _ = fs::remove_file(pid_path);
    }

    fn handle_ipc(&mut self, cmd: &str) -> String {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        match parts.get(0).copied().unwrap_or("") {
            "PAUSE" => {
                self.paused = true;
                "Wallpaper switching paused.".into()
            }
            "RESUME" => {
                self.paused = false;
                "Wallpaper switching resumed.".into()
            }
            "NEXT" => {
                if !self.playlist.is_empty() {
                    self.current_idx = (self.current_idx + 1) % self.playlist.len();
                    self.apply_current();
                    "Skipped to next wallpaper.".into()
                } else {
                    "No wallpapers found in profile folder.".into()
                }
            }
            "PREV" => {
                if !self.playlist.is_empty() {
                    self.current_idx = if self.current_idx == 0 { self.playlist.len() - 1 } else { self.current_idx - 1 };
                    self.apply_current();
                    "Returned to previous wallpaper.".into()
                } else {
                    "No wallpapers found in profile folder.".into()
                }
            }
            "SWITCH" => {
                if let Some(profile_name) = parts.get(1) {
                    if self.config.profiles.contains_key(*profile_name) {
                        self.config.active_profile = profile_name.to_string();
                        self.reload_playlist();
                        self.apply_current();
                        format!("Switched to profile '{}'", profile_name)
                    } else {
                        format!("Error: Profile '{}' not found in config.ini", profile_name)
                    }
                } else {
                    "Usage: wallctl switch <profile>".into()
                }
            }
            "STATUS" => {
                let current_file = self.playlist.get(self.current_idx % self.playlist.len().max(1))
                    .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
                    .unwrap_or_else(|| "None".into());
                format!(
                    "Profile: {}\nPaused: {}\nTotal items: {}\nCurrent: {}",
                    self.config.active_profile, self.paused, self.playlist.len(), current_file
                )
            }
            "STOP" => "Stopping daemon and restoring wallpaper...".into(),
            _ => "Unknown command.".into(),
        }
    }
}
