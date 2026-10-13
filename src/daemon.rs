use crate::config::{load_config, Config, ImageScale, PlaylistOrder};
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
const DAEMON_POLL_INTERVAL: Duration = Duration::from_millis(10);

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
            self.playlist = find_assets(&profile.folder);
            match profile.order {
                PlaylistOrder::Random => self.playlist.shuffle(&mut rand::thread_rng()),
                PlaylistOrder::Alphabetical => self.playlist.sort_by(|left, right| {
                    let left_name = left.to_string_lossy();
                    let right_name = right.to_string_lossy();
                    left_name
                        .to_lowercase()
                        .cmp(&right_name.to_lowercase())
                        .then_with(|| left_name.cmp(&right_name))
                }),
            }
        }
        self.current_idx = None;
    }

    fn apply_edited_config(&mut self) -> Result<()> {
        let current_path = self
            .current_idx
            .and_then(|index| self.playlist.get(index))
            .cloned();
        self.config = self.load_config_preserving_active_profile()?;
        self.reload_playlist();
        self.current_idx = current_path
            .as_ref()
            .and_then(|path| self.playlist.iter().position(|item| item == path));
        // Keep the wallpaper already on screen. Restarting a video just to
        // refresh its profile settings can fail when xwinwrap/mpv is not
        // available; changed settings take effect the next time it is chosen.
        Ok(())
    }

    fn load_config_preserving_active_profile(&self) -> Result<Config> {
        let mut config = load_config()?;
        // Keep the profile selected in the running daemon when reloading.
        // The config file's active_profile is used on daemon startup.
        if config.profiles.contains_key(&self.config.active_profile) {
            config.active_profile = self.config.active_profile.clone();
        }
        Ok(config)
    }

    fn apply_index(&mut self, index: usize) -> Result<()> {
        let path = self
            .playlist
            .get(index)
            .context("no wallpapers found in the active profile")?;
        // Reapplying the same desktop image/video can make desktop services
        // tear down and redraw the wallpaper layer, which appears as a flash.
        // This also makes next/prev harmless for one-item playlists.
        if self.current_idx == Some(index) {
            self.last_switch = Instant::now();
            return Ok(());
        }
        let mute = self
            .config
            .profiles
            .get(&self.config.active_profile)
            .map(|profile| profile.mute_video)
            .unwrap_or(true);
        let image_scale = self
            .config
            .profiles
            .get(&self.config.active_profile)
            .map(|profile| profile.image_scale)
            .unwrap_or(ImageScale::Fit);
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if extension == "gif" {
            // Desktop settings services treat GIFs as still images. Play them
            // through the same desktop-layer player used for video wallpapers
            // on Linux. macOS has no supported desktop-layer player here.
            if cfg!(target_os = "macos") {
                self.engine.set_static(path, image_scale)?;
            } else {
                self.engine.set_video(path, true)?;
            }
        } else if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
            self.engine.set_static(path, image_scale)?;
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

    fn set(&mut self, one_based_index: usize) -> Result<()> {
        anyhow::ensure!(one_based_index > 0, "wallpaper indexes start at 1");
        self.apply_index(one_based_index - 1)
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
        if !self.playlist.is_empty() {
            self.next()?;
        }
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
                    let stop = matches!(command.trim(), "STOP" | "API/1 STOP");
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
                .unwrap_or(300.0);
            if !self.paused
                && !self.playlist.is_empty()
                && self.last_switch.elapsed() >= Duration::from_secs_f64(interval)
            {
                if let Err(error) = self.next() {
                    // Keep a failed wallpaper from being retried on every
                    // 10 ms daemon poll when the rotation timer is overdue.
                    self.last_switch = Instant::now();
                    eprintln!("wallctl: automatic wallpaper switch failed: {error:#}");
                }
            }
            std::thread::sleep(DAEMON_POLL_INTERVAL);
        }
    }

    fn handle_ipc(&mut self, command: &str) -> String {
        if let Some(request) = command.strip_prefix("API/1 ") {
            return self.handle_api(request);
        }
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
            "SET" => match parts.next().and_then(|index| index.parse::<usize>().ok()) {
                Some(index) => self
                    .set(index)
                    .map(|_| format!("Set wallpaper {index}."))
                    .unwrap_or_else(|error| format!("Error: {error:#}")),
                None => "Usage: wallctl set <index> (indexes start at 1)".into(),
            },
            "LIST" => self.list_text(),
            "CONFIG_UPDATED" => self
                .apply_edited_config()
                .map(|_| "Configuration updated.".into())
                .unwrap_or_else(|error| format!("Error: {error:#}")),
            "RELOAD" => {
                self.config = match self.load_config_preserving_active_profile() {
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
                    .and_then(|index| {
                        self.playlist.get(index).map(|path| {
                            format!(
                                "{} ({})",
                                index + 1,
                                path.file_name().unwrap_or_default().to_string_lossy()
                            )
                        })
                    })
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

    fn handle_api(&mut self, request: &str) -> String {
        let mut parts = request.split_whitespace();
        match parts.next().unwrap_or("") {
            "LIST" => self.list_json(),
            "STATUS" => self.status_json(),
            "NEXT" => {
                let result = self.next();
                Self::api_action(result)
            }
            "PREV" => {
                let result = self.previous();
                Self::api_action(result)
            }
            "PAUSE" => {
                self.paused = true;
                r#"{"api_version":1,"ok":true,"paused":true}"#.into()
            }
            "RESUME" => {
                self.paused = false;
                r#"{"api_version":1,"ok":true,"paused":false}"#.into()
            }
            "RELOAD" => {
                self.config = match self.load_config_preserving_active_profile() {
                    Ok(config) => config,
                    Err(error) => return api_error(&format!("{error:#}")),
                };
                self.reload_playlist();
                let result = self.next();
                Self::api_action(result)
            }
            "SWITCH" => match parts.next() {
                Some(profile) if self.config.profiles.contains_key(profile) => {
                    self.config.active_profile = profile.to_owned();
                    self.reload_playlist();
                    let result = self.next();
                    Self::api_action(result)
                }
                Some(profile) => api_error(&format!("profile '{profile}' was not found")),
                None => api_error("SWITCH requires a profile name"),
            },
            "STOP" => r#"{"api_version":1,"ok":true,"stopping":true}"#.into(),
            "SET" => match parts.next().and_then(|index| index.parse::<usize>().ok()) {
                Some(index) => match self.set(index) {
                    Ok(()) => format!(r#"{{"api_version":1,"ok":true,"index":{index}}}"#),
                    Err(error) => api_error(&format!("{error:#}")),
                },
                None => api_error("SET requires a positive, one-based index"),
            },
            _ => api_error("unknown API v1 request"),
        }
    }

    fn api_action(result: Result<()>) -> String {
        match result {
            Ok(()) => r#"{"api_version":1,"ok":true}"#.into(),
            Err(error) => api_error(&format!("{error:#}")),
        }
    }

    fn list_text(&self) -> String {
        if self.playlist.is_empty() {
            return "No wallpapers found in the active profile.".into();
        }
        self.playlist
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let current = if self.current_idx == Some(index) {
                    " *"
                } else {
                    "  "
                };
                format!(
                    "{}{current} {}",
                    index + 1,
                    path.file_name().unwrap_or_default().to_string_lossy()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn list_json(&self) -> String {
        let items = self
            .playlist
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let path = path.to_string_lossy();
                format!(
                    "{{\"index\":{},\"name\":\"{}\",\"path\":\"{}\",\"current\":{}}}",
                    index + 1,
                    json_escape(&name),
                    json_escape(&path),
                    self.current_idx == Some(index)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("{{\"api_version\":1,\"items\":[{items}]}}")
    }

    fn status_json(&self) -> String {
        let current = self.current_idx.and_then(|index| self.playlist.get(index));
        let current_index = self
            .current_idx
            .map(|index| index + 1)
            .map_or("null".into(), |index| index.to_string());
        let current_path = current
            .map(|path| format!("\"{}\"", json_escape(&path.to_string_lossy())))
            .unwrap_or_else(|| "null".into());
        format!(
            "{{\"api_version\":1,\"active_profile\":\"{}\",\"paused\":{},\"total_items\":{},\"current_index\":{},\"current_path\":{}}}",
            json_escape(&self.config.active_profile), self.paused, self.playlist.len(), current_index, current_path
        )
    }
}

fn find_assets(folder: &std::path::Path) -> Vec<PathBuf> {
    let mut assets = Vec::new();
    let Ok(entries) = fs::read_dir(folder) else {
        return assets;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            assets.extend(find_assets(&path));
        } else if path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    let ext = ext.to_ascii_lowercase();
                    IMAGE_EXTENSIONS.contains(&ext.as_str())
                        || VIDEO_EXTENSIONS.contains(&ext.as_str())
                })
        {
            assets.push(path);
        }
    }
    assets
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn api_error(message: &str) -> String {
    format!(
        "{{\"api_version\":1,\"ok\":false,\"error\":\"{}\"}}",
        json_escape(message)
    )
}
