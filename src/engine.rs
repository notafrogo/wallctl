use crate::config::{get_config_dir, get_state_file, ImageScale};
use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const VIDEO_TRANSITION: Duration = Duration::from_millis(450);
const XWINWRAP_DESKTOP_FLAGS: &[&str] = &[
    "-argb", "-fdt", "-b", "-ni", "-s", "-st", "-sp", "-nf", "-fs",
];

#[derive(Clone, Copy)]
enum LinuxDesktop {
    Cinnamon,
    Mate,
    Xfce,
}

fn linux_desktop() -> Result<LinuxDesktop> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if desktop.contains("xfce") {
        return Ok(LinuxDesktop::Xfce);
    }
    if desktop.contains("mate") {
        return Ok(LinuxDesktop::Mate);
    }
    if desktop.contains("cinnamon") {
        return Ok(LinuxDesktop::Cinnamon);
    }

    // Some launchers omit XDG_CURRENT_DESKTOP. Probe the schemas before
    // defaulting to Cinnamon, which is the default Linux Mint edition.
    for (schema, desktop) in [
        ("org.cinnamon.desktop.background", LinuxDesktop::Cinnamon),
        ("org.mate.background", LinuxDesktop::Mate),
    ] {
        if Command::new("gsettings")
            .args(["list-schemas"])
            .output()
            .is_ok_and(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line == schema)
            })
        {
            return Ok(desktop);
        }
    }
    if Command::new("xfconf-query")
        .arg("-c")
        .arg("xfce4-desktop")
        .arg("-l")
        .output()
        .is_ok()
    {
        return Ok(LinuxDesktop::Xfce);
    }
    Ok(LinuxDesktop::Cinnamon)
}

fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

fn decode_file_uri(value: &str) -> String {
    let Some(encoded) = value.strip_prefix("file://") else {
        return value.to_owned();
    };
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                decoded.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn xfce_wallpaper_paths() -> Result<Vec<String>> {
    let output = Command::new("xfconf-query")
        .args(["-c", "xfce4-desktop", "-l"])
        .output()
        .context("xfconf-query is required to manage Xfce wallpapers")?;
    if !output.status.success() {
        bail!("could not list Xfce wallpaper settings");
    }
    let paths = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|path| path.ends_with("/last-image"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        bail!("Xfce has no wallpaper settings yet; set a wallpaper in Desktop Settings first");
    }
    Ok(paths)
}

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
            let output = match linux_desktop()? {
                LinuxDesktop::Cinnamon => Command::new("gsettings")
                    .args(["get", "org.cinnamon.desktop.background", "picture-uri"])
                    .output()
                    .context("gsettings is required to read the Cinnamon wallpaper")?,
                LinuxDesktop::Mate => Command::new("gsettings")
                    .args(["get", "org.mate.background", "picture-filename"])
                    .output()
                    .context("gsettings is required to read the MATE wallpaper")?,
                LinuxDesktop::Xfce => {
                    let path = xfce_wallpaper_paths()?.remove(0);
                    let output = Command::new("xfconf-query")
                        .args(["-c", "xfce4-desktop", "-p", &path])
                        .output()
                        .context("xfconf-query is required to read the Xfce wallpaper")?;
                    if !output.status.success() {
                        bail!("could not read the Xfce wallpaper");
                    }
                    return Ok(Some(
                        String::from_utf8_lossy(&output.stdout).trim().to_owned(),
                    ));
                }
            };
            if !output.status.success() {
                bail!("gsettings could not read the current wallpaper");
            }
            let value = String::from_utf8_lossy(&output.stdout);
            let value = value.trim().trim_matches('\'');
            let path = decode_file_uri(value);
            Ok((!path.is_empty()).then_some(path))
        }
    }

    pub fn stop_video(&mut self) {
        if let Some(mut child) = self.video_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(mpv_socket_path());
    }

    pub fn set_static(&mut self, path: &Path, image_scale: ImageScale) -> Result<()> {
        let path = path
            .canonicalize()
            .with_context(|| format!("wallpaper does not exist: {}", path.display()))?;
        self.set_static_background(&path, image_scale)?;
        self.fade_out_video();
        self.stop_video();
        Ok(())
    }

    fn set_static_background(&self, path: &Path, image_scale: ImageScale) -> Result<()> {
        if cfg!(target_os = "macos") {
            let scaling = match image_scale {
                ImageScale::Fit => 3,
                ImageScale::FitVertical | ImageScale::FitHorizontal => 2,
                ImageScale::Original => 1,
            };
            let script = format!("ObjC.import('AppKit'); function run(argv) {{ const url = $.NSURL.fileURLWithPath(argv[0]); const screens = $.NSScreen.screens; const options = $.NSMutableDictionary.alloc.init; options.setObjectForKey(ObjC.unwrap({scaling}), $.NSWorkspaceDesktopImageScalingKey); for (let i = 0; i < screens.count; i++) {{ $.NSWorkspace.sharedWorkspace.setDesktopImageURLForScreenOptionsError(url, screens.objectAtIndex(i), options, null); }} }}");
            let status = Command::new("osascript")
                .args([
                    "-l",
                    "JavaScript",
                    "-e",
                    &script,
                    path.to_string_lossy().as_ref(),
                ])
                .status()
                .context("could not run osascript")?;
            if !status.success() {
                bail!("macOS rejected the wallpaper path");
            }
        } else {
            let path_string = path.to_string_lossy();
            match linux_desktop()? {
                LinuxDesktop::Cinnamon => {
                    let uri = file_uri(&path);
                    let option = linux_picture_option(image_scale);
                    let options_status = Command::new("gsettings")
                        .args([
                            "set",
                            "org.cinnamon.desktop.background",
                            "picture-options",
                            option,
                        ])
                        .status()
                        .context("gsettings is required to set Cinnamon wallpaper scaling")?;
                    if !options_status.success() {
                        bail!("Cinnamon rejected the wallpaper scaling option");
                    }
                    let status = Command::new("gsettings")
                        .args([
                            "set",
                            "org.cinnamon.desktop.background",
                            "picture-uri",
                            &uri,
                        ])
                        .status()
                        .context("gsettings is required to set Cinnamon wallpapers")?;
                    if !status.success() {
                        bail!("Cinnamon rejected the wallpaper path");
                    }
                }
                LinuxDesktop::Mate => {
                    let option = linux_picture_option(image_scale);
                    let options_status = Command::new("gsettings")
                        .args(["set", "org.mate.background", "picture-options", option])
                        .status()
                        .context("gsettings is required to set MATE wallpaper scaling")?;
                    if !options_status.success() {
                        bail!("MATE rejected the wallpaper scaling option");
                    }
                    let status = Command::new("gsettings")
                        .args([
                            "set",
                            "org.mate.background",
                            "picture-filename",
                            &path_string,
                        ])
                        .status()
                        .context("gsettings is required to set MATE wallpapers")?;
                    if !status.success() {
                        bail!("MATE rejected the wallpaper path");
                    }
                }
                LinuxDesktop::Xfce => {
                    for setting in xfce_wallpaper_paths()? {
                        let style = xfce_image_style(image_scale).to_string();
                        if let Some(prefix) = setting.strip_suffix("last-image") {
                            let status = Command::new("xfconf-query")
                                .args([
                                    "-c",
                                    "xfce4-desktop",
                                    "-p",
                                    &format!("{prefix}image-style"),
                                    "-s",
                                    &style,
                                ])
                                .status()
                                .context(
                                    "xfconf-query is required to set Xfce wallpaper scaling",
                                )?;
                            if !status.success() {
                                bail!("Xfce rejected the wallpaper scaling option");
                            }
                        }
                        let status = Command::new("xfconf-query")
                            .args(["-c", "xfce4-desktop", "-p", &setting, "-s", &path_string])
                            .status()
                            .context("xfconf-query is required to set Xfce wallpapers")?;
                        if !status.success() {
                            bail!("Xfce rejected the wallpaper path for {setting}");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn set_video(&mut self, path: &Path, mute: bool) -> Result<()> {
        if cfg!(target_os = "macos") {
            bail!("animated wallpapers are not supported on macOS; GIFs are shown as still images");
        }
        let path = path
            .canonicalize()
            .with_context(|| format!("wallpaper does not exist: {}", path.display()))?;
        if self.video_process.is_some() {
            let preview = render_transition_preview(&path)?;
            self.set_static_background(&preview, ImageScale::Fit)?;
            self.fade_out_video();
            self.stop_video();
        }
        // mpv enables audio by default; --audio=yes is not accepted by all
        // mpv versions (the option takes an audio device name, not a boolean).
        let audio_flag = if mute { Some("--no-audio") } else { None };
        let socket_path = mpv_socket_path();
        let _ = fs::remove_file(&socket_path);
        let child_result = Command::new("xwinwrap")
            .args(XWINWRAP_DESKTOP_FLAGS)
            .args([
                "--",
                "mpv",
                "-wid",
                "WID",
                "--alpha=yes",
                &format!("--input-ipc-server={}", socket_path.display()),
                "--loop=inf",
                "--no-osc",
                "--no-osd-bar",
                "--vd-lavc-threads=2",
                "--cache=no",
            ])
            .args(audio_flag)
            .arg(path.to_string_lossy().as_ref())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        let child = match child_result {
            Ok(child) => child,
            Err(error) => return Err(error).context("video wallpapers require xwinwrap and mpv"),
        };
        self.video_process = Some(child);
        for _ in 0..40 {
            if UnixStream::connect(&socket_path).is_ok() {
                return Ok(());
            }
            if let Some(child) = self.video_process.as_mut() {
                if let Some(status) = child.try_wait()? {
                    let mut stderr = String::new();
                    if let Some(mut output) = child.stderr.take() {
                        let _ = output.read_to_string(&mut stderr);
                    }
                    self.video_process = None;
                    let stderr = stderr.trim();
                    if stderr.is_empty() {
                        bail!("xwinwrap or mpv exited while starting the video wallpaper (exit status {status})");
                    }
                    bail!("xwinwrap or mpv exited while starting the video wallpaper (exit status {status}): {stderr}");
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.stop_video();
        bail!("mpv did not create its control socket; check the video wallpaper dependencies")
    }

    fn fade_out_video(&self) {
        if self.video_process.is_none() {
            return;
        }
        let Some(time_pos) = mpv_command(&["get_property".into(), "time-pos".into()])
            .ok()
            .and_then(|response| json_number(&response, "data"))
        else {
            return;
        };
        let filter = format!(
            "lavfi=[fade=t=out:st={time_pos}:d={:.2}:alpha=1]",
            VIDEO_TRANSITION.as_secs_f64()
        );
        let _ = mpv_command(&["vf".into(), "add".into(), filter]);
        std::thread::sleep(VIDEO_TRANSITION);
    }

    pub fn restore_original(&mut self) -> Result<()> {
        self.stop_video();
        let state_file = get_state_file();
        if !state_file.exists() {
            return Ok(());
        }
        let path = fs::read_to_string(&state_file)
            .with_context(|| format!("could not read {}", state_file.display()))?;
        self.set_static(Path::new(path.trim()), ImageScale::Fit)?;
        fs::remove_file(state_file).context("could not remove wallpaper restore state")?;
        Ok(())
    }
}

fn linux_picture_option(scale: ImageScale) -> &'static str {
    match scale {
        ImageScale::Fit => "'scaled'",
        ImageScale::FitVertical | ImageScale::FitHorizontal => "'stretched'",
        ImageScale::Original => "'centered'",
    }
}

fn xfce_image_style(scale: ImageScale) -> u8 {
    match scale {
        ImageScale::Fit => 4,
        ImageScale::FitVertical | ImageScale::FitHorizontal => 3,
        ImageScale::Original => 1,
    }
}

fn mpv_socket_path() -> std::path::PathBuf {
    get_config_dir().join("mpv-wallpaper.sock")
}

fn mpv_command(arguments: &[String]) -> Result<String> {
    let mut stream = UnixStream::connect(mpv_socket_path())
        .context("could not connect to the wallpaper player's control socket")?;
    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
    stream.set_write_timeout(Some(Duration::from_millis(250)))?;
    let values = arguments
        .iter()
        .map(|value| format!("\"{}\"", json_escape(value)))
        .collect::<Vec<_>>()
        .join(",");
    writeln!(stream, "{{\"command\":[{values}]}}")?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    Ok(response)
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

fn json_number(response: &str, key: &str) -> Option<f64> {
    let marker = format!("\"{key}\":");
    let value = response.split_once(&marker)?.1;
    let value = value
        .trim_start()
        .split(|character: char| character == ',' || character == '}')
        .next()?;
    value.parse().ok()
}

fn render_transition_preview(video: &Path) -> Result<std::path::PathBuf> {
    let config_dir = get_config_dir();
    fs::create_dir_all(&config_dir)
        .with_context(|| format!("could not create {}", config_dir.display()))?;
    let preview = config_dir.join("transition-preview.jpg");
    let output = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-threads",
            "1",
            "-i",
            video.to_string_lossy().as_ref(),
            "-frames:v",
            "1",
            "-vf",
            "scale=1920:1080:force_original_aspect_ratio=decrease",
            preview.to_string_lossy().as_ref(),
        ])
        .output()
        .context("ffmpeg is required to prepare smooth video wallpaper transitions")?;
    if !output.status.success() {
        bail!(
            "could not create a transition preview from {}: {}",
            video.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::{decode_file_uri, file_uri, json_escape, json_number, XWINWRAP_DESKTOP_FLAGS};
    use std::path::Path;

    #[test]
    fn file_uris_round_trip_spaces_and_unicode() {
        let path = Path::new("/tmp/café wallpaper.jpg");
        assert_eq!(decode_file_uri(&file_uri(path)), path.to_string_lossy());
    }

    #[test]
    fn file_uri_decoder_handles_raw_unicode_without_panicking() {
        assert_eq!(decode_file_uri("file:///tmp/café.jpg"), "/tmp/café.jpg");
    }

    #[test]
    fn json_escaping_covers_control_characters() {
        assert_eq!(json_escape("a\n\u{0001}\"\\"), "a\\n\\u0001\\\"\\\\");
    }

    #[test]
    fn json_number_reads_numeric_data() {
        assert_eq!(
            json_number(r#"{"data":12.5,"error":"success"}"#, "data"),
            Some(12.5)
        );
    }

    #[test]
    fn video_window_uses_desktop_layer_and_never_override_redirect() {
        assert!(XWINWRAP_DESKTOP_FLAGS.contains(&"-fdt"));
        assert!(XWINWRAP_DESKTOP_FLAGS.contains(&"-b"));
        assert!(!XWINWRAP_DESKTOP_FLAGS.contains(&"-ov"));
        assert!(!XWINWRAP_DESKTOP_FLAGS.contains(&"-ovr"));
    }
}
