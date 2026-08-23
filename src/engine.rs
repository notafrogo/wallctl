use crate::config::get_config_dir;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub struct WallpaperEngine {
    pub original_wallpaper: Option<String>,
    pub video_process: Option<Child>,
}

impl WallpaperEngine {
    pub fn new() -> Self {
        if cfg!(target_os = "macos") {
            if let Some(store_plist) = Self::get_macos_store_plist() {
                let backup_plist = get_config_dir().join("original_Index.plist");
                if !backup_plist.exists() {
                    let _ = fs::copy(&store_plist, &backup_plist);
                }
            }
        }

        let original = Self::fetch_current_wallpaper();
        Self {
            original_wallpaper: original,
            video_process: None,
        }
    }

    fn get_macos_store_plist() -> Option<PathBuf> {
        dirs::home_dir().map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("com.apple.wallpaper")
                .join("Store")
                .join("Index.plist")
        }).filter(|p| p.exists())
    }

    fn fetch_current_wallpaper() -> Option<String> {
        if cfg!(target_os = "macos") {
            let script = r#"ObjC.import('AppKit'); var u = $.NSWorkspace.sharedWorkspace.desktopImageURLForScreen($.NSScreen.mainScreen); u ? u.path.js : ''"#;
            if let Ok(output) = Command::new("osascript").args(["-l", "JavaScript", "-e", script]).output() {
                let res = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !res.is_empty() && res != "undefined" && res != "null" && !res.contains("DefaultDesktop.heic") {
                    return Some(res);
                }
            }

            let script_finder = r#"tell app "Finder" to get posix path of (get desktop picture as alias)"#;
            if let Ok(output) = Command::new("osascript").args(["-e", script_finder]).output() {
                let res = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !res.is_empty() && !res.contains("missing value") && !res.contains("DefaultDesktop.heic") {
                    return Some(res);
                }
            }

            None
        } else {
            let output = Command::new("gsettings")
                .args(["get", "org.cinnamon.desktop.background", "picture-uri"])
                .output()
                .ok()?;
            let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let clean = raw.replace('\'', "").replace("file://", "");
            if clean.is_empty() { None } else { Some(clean) }
        }
    }

    pub fn stop_video(&mut self) {
        if let Some(mut child) = self.video_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn set_static(&mut self, path: &Path) {
        self.stop_video();
        let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path_str = abs_path.to_string_lossy();

        if cfg!(target_os = "macos") {
            let script = format!(
                r#"ObjC.import('AppKit'); var u = $.NSURL.fileURLWithPath('{}'); var s = $.NSScreen.screens; for(var i=0; i<s.count; i++) {{ $.NSWorkspace.sharedWorkspace.setDesktopImageURLForScreenOptionsError(u, s.objectAtIndex(i), $(), null); }}"#,
                path_str
            );
            let _ = Command::new("osascript").args(["-l", "JavaScript", "-e", &script]).status();
        } else {
            let uri = format!("file://{}", path_str);
            let _ = Command::new("gsettings")
                .args(["set", "org.cinnamon.desktop.background", "picture-uri", &uri])
                .status();
        }
    }

    pub fn set_video_or_gif(&mut self, path: &Path, mute: bool) {
        self.stop_video();
        let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path_str = abs_path.to_string_lossy();
        let mute_arg = if mute { "--mute=yes" } else { "--mute=no" };

        if cfg!(target_os = "macos") {
            let child = Command::new("mpv")
                .args([
                    "--wid=0",
                    "--loop=inf",
                    "--player-operation-mode=cplayer",
                    "--no-osc",
                    "--no-osd-bar",
                    mute_arg,
                    &path_str,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();

            self.video_process = child.ok();
        } else {
            let audio_flag = if mute { "--no-audio" } else { "--audio" };
            let child = Command::new("xwinwrap")
                .args([
                    "-ov",
                    "-fs",
                    "--",
                    "mpv",
                    "-wid",
                    "WID",
                    "--loop=inf",
                    "--no-osc",
                    "--no-osd-bar",
                    audio_flag,
                    &path_str,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();

            self.video_process = child.ok();
        }
    }

    pub fn restore_original(&mut self) {
        self.stop_video();
        let orig_wallpaper = self.original_wallpaper.clone();

        if cfg!(target_os = "macos") {
            let backup_plist = get_config_dir().join("original_Index.plist");
            if let Some(store_plist) = Self::get_macos_store_plist() {
                if backup_plist.exists() {
                    let _ = fs::copy(&backup_plist, &store_plist);
                    let _ = fs::remove_file(&backup_plist);
                    let _ = Command::new("killall").arg("WallpaperAgent").status();
                    return;
                }
            }

            if let Some(orig) = &orig_wallpaper {
                self.set_static(Path::new(orig));
            }
        } else {
            if let Some(orig) = &orig_wallpaper {
                self.set_static(Path::new(orig));
            }
        }
    }
}
