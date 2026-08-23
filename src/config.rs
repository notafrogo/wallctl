use ini::Ini;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Profile {
    #[allow(dead_code)]
    pub name: String,
    pub folder: PathBuf,
    pub interval: u64,
    pub order: String,
    pub mute_video: bool,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub active_profile: String,
    pub restore_on_exit: bool,
    pub profiles: HashMap<String, Profile>,
}

pub fn get_config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("wallctl")
    } else if let Some(home) = dirs::home_dir() {
        home.join(".config").join("wallctl")
    } else {
        PathBuf::from(".config").join("wallctl")
    }
}

pub fn get_config_file() -> PathBuf {
    get_config_dir().join("config.ini")
}

fn generate_black_image(path: &Path) {
    let width: u32 = 1920;
    let height: u32 = 1080;
    let row_size = (width * 3 + 3) & !3;
    let image_size = row_size * height;
    let file_size = 54 + image_size;

    let mut bmp = Vec::with_capacity(file_size as usize);
    // BMP Header (14 bytes)
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&54u32.to_le_bytes());

    // DIB Header (40 bytes)
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(width as i32).to_le_bytes());
    bmp.extend_from_slice(&(height as i32).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(image_size as u32).to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());

    // 0x00 pixel data = solid black
    bmp.resize(file_size as usize, 0);

    let _ = fs::write(path, bmp);
}

pub fn load_or_create_config() -> Config {
    let config_dir = get_config_dir();
    let config_file = get_config_file();
    let default_folder = config_dir.join("wallpapers").join("default");

    if !config_file.exists() {
        let _ = fs::create_dir_all(&default_folder);

        let black_image_path = default_folder.join("black.bmp");
        generate_black_image(&black_image_path);

        let default_ini = format!(
            "[general]\nactive_profile = default\nrestore_on_exit = true\n\n[profile.default]\nfolder = {}\ninterval = 300\norder = random\nmute_video = true\n",
            default_folder.to_string_lossy()
        );
        let _ = fs::write(&config_file, default_ini);
    }

    let ini = Ini::load_from_file(&config_file).unwrap_or_else(|_| Ini::new());

    let active_profile = ini
        .get_from(Some("general"), "active_profile")
        .unwrap_or("default")
        .to_string();

    let restore_on_exit = ini
        .get_from(Some("general"), "restore_on_exit")
        .unwrap_or("true")
        .parse::<bool>()
        .unwrap_or(true);

    let mut profiles = HashMap::new();

    for (section, prop) in &ini {
        if let Some(sec_name) = section {
            if let Some(profile_name) = sec_name.strip_prefix("profile.") {
                let folder_str = prop.get("folder").unwrap_or("");
                let expanded_folder = shellexpand::tilde(folder_str).to_string();

                let profile = Profile {
                    name: profile_name.to_string(),
                    folder: PathBuf::from(expanded_folder),
                    interval: prop.get("interval").and_then(|v| v.parse().ok()).unwrap_or(300),
                    order: prop.get("order").unwrap_or("random").to_string(),
                    mute_video: prop.get("mute_video").and_then(|v| v.parse().ok()).unwrap_or(true),
                };
                profiles.insert(profile_name.to_string(), profile);
            }
        }
    }

    Config {
        active_profile,
        restore_on_exit,
        profiles,
    }
}
