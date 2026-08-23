use crate::config::get_state_file;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};

pub struct WallpaperEngine {
    video_process: Option<Child>,
}

impl WallpaperEngine {
    pub fn new() -> Self {
        Self {
            video_process: None,
        }
    }

    pub fn save_original_wallpaper(&self) -> Result<()> {
        let state_file = get_state_file();
        if state_file.exists() {
            return Ok(());
        }
        if let Some(path) = self.current_wallpaper()? {
            fs::write(&state_file, path)
                .with_context(|| format!("could not save {}", state_file.display()))?;
        }
        Ok(())
    }

    fn current_wallpaper(&self) -> Result<Option<String>> {
        if cfg!(target_os = "macos") {
            let script = "ObjC.import('AppKit'); const url = $.NSWorkspace.sharedWorkspace.desktopImageURLForScreen($.NSScreen.mainScreen); if (url) console.log(ObjC.unwrap(url.path));";
            let output = Command::new("osascript")
                .args(["-l", "JavaScript", "-e", script])
                .output()
                .context("could not ask macOS for the current wallpaper")?;
            if !output.status.success() {
                bail!("macOS did not return the current wallpaper");
            }
            let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            Ok((!path.is_empty()).then_some(path))
        } else {
            let output = Command::new("gsettings")
                .args(["get", "org.cinnamon.desktop.background", "picture-uri"])
                .output()
                .context("gsettings is required to read the current wallpaper")?;
            if !output.status.success() {
                bail!("gsettings could not read the current wallpaper");
            }
            let path = String::from_utf8_lossy(&output.stdout)
                .trim()
                .trim_matches('\'')
                .strip_prefix("file://")
                .unwrap_or("")
                .to_owned();
            Ok((!path.is_empty()).then_some(path))
        }
    }

    pub fn stop_video(&mut self) {
        if let Some(mut child) = self.video_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn set_static(&mut self, path: &Path) -> Result<()> {
        self.stop_video();
        let path = path
            .canonicalize()
            .with_context(|| format!("wallpaper does not exist: {}", path.display()))?;
        if cfg!(target_os = "macos") {
            let script = "ObjC.import('AppKit'); function run(argv) { const url = $.NSURL.fileURLWithPath(argv[0]); const screens = $.NSScreen.screens; for (let i = 0; i < screens.count; i++) { $.NSWorkspace.sharedWorkspace.setDesktopImageURLForScreenOptionsError(url, screens.objectAtIndex(i), $(), null); } }";
            let status = Command::new("osascript")
                .args([
                    "-l",
                    "JavaScript",
                    "-e",
                    script,
                    path.to_string_lossy().as_ref(),
                ])
                .status()
                .context("could not run osascript")?;
            if !status.success() {
                bail!("macOS rejected the wallpaper path");
            }
        } else {
            let uri = format!("file://{}", path.display());
            let status = Command::new("gsettings")
                .args([
                    "set",
                    "org.cinnamon.desktop.background",
                    "picture-uri",
                    &uri,
                ])
                .status()
                .context("gsettings is required to set wallpapers on this platform")?;
            if !status.success() {
                bail!("gsettings could not set the wallpaper");
            }
        }
        Ok(())
    }

    pub fn set_video(&mut self, path: &Path, mute: bool) -> Result<()> {
        self.stop_video();
        if cfg!(target_os = "macos") {
            bail!("video wallpapers are not supported on macOS; use an image or GIF instead");
        }
        let path = path
            .canonicalize()
            .with_context(|| format!("wallpaper does not exist: {}", path.display()))?;
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
                path.to_string_lossy().as_ref(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("video wallpapers require xwinwrap and mpv")?;
        self.video_process = Some(child);
        Ok(())
    }

    pub fn restore_original(&mut self) -> Result<()> {
        self.stop_video();
        let state_file = get_state_file();
        if !state_file.exists() {
            return Ok(());
        }
        let path = fs::read_to_string(&state_file)
            .with_context(|| format!("could not read {}", state_file.display()))?;
        self.set_static(Path::new(path.trim()))?;
        fs::remove_file(state_file).context("could not remove wallpaper restore state")?;
        Ok(())
    }
}
