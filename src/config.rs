use anyhow::{bail, Context, Result};
use ini::Ini;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_INTERVAL_SECS: u64 = 300;

#[derive(Debug, Clone)]
pub struct Profile {
    pub folder: PathBuf,
    pub interval: u64,
    pub order: PlaylistOrder,
    pub mute_video: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistOrder {
    Random,
    Alphabetical,
}

impl PlaylistOrder {
    fn parse(value: &str, profile: &str) -> Result<Self> {
        match value {
            "random" => Ok(Self::Random),
            "alphabetical" => Ok(Self::Alphabetical),
            other => {
                bail!("profile '{profile}' has invalid order '{other}'; use random or alphabetical")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub active_profile: String,
    pub restore_on_exit: bool,
    pub profiles: HashMap<String, Profile>,
}

pub fn get_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("wallctl")
}

pub fn get_config_file() -> PathBuf {
    get_config_dir().join("config.ini")
}
pub fn get_state_file() -> PathBuf {
    get_config_dir().join("original-wallpaper")
}

pub fn ensure_config() -> Result<PathBuf> {
    let config_dir = get_config_dir();
    let config_file = get_config_file();
    let default_folder = config_dir.join("wallpapers").join("default");
    fs::create_dir_all(&default_folder)
        .with_context(|| format!("could not create {}", default_folder.display()))?;
    if !config_file.exists() {
        fs::write(&config_file, default_config(&default_folder))
            .with_context(|| format!("could not write {}", config_file.display()))?;
    }
    Ok(config_file)
}

fn default_config(folder: &Path) -> String {
    format!("# wallctl configuration\n# order can be random or alphabetical; interval is in seconds.\n\n[general]\nactive_profile = default\nrestore_on_exit = true\n\n[profile.default]\nfolder = {}\ninterval = {DEFAULT_INTERVAL_SECS}\norder = random\nmute_video = true\n", folder.to_string_lossy())
}

pub fn load_config() -> Result<Config> {
    let config_file = ensure_config()?;
    let ini = Ini::load_from_file(&config_file)
        .with_context(|| format!("could not parse {}", config_file.display()))?;
    let active_profile = ini
        .get_from(Some("general"), "active_profile")
        .unwrap_or("default")
        .trim()
        .to_owned();
    let restore_on_exit = ini
        .get_from(Some("general"), "restore_on_exit")
        .unwrap_or("true")
        .parse::<bool>()
        .context("general.restore_on_exit must be true or false")?;
    let mut profiles = HashMap::new();
    for (section, properties) in &ini {
        let Some(profile_name) = section.and_then(|name| name.strip_prefix("profile.")) else {
            continue;
        };
        if profile_name.is_empty() {
            bail!("profile section names must use [profile.<name>]");
        }
        let folder = properties
            .get("folder")
            .filter(|value| !value.trim().is_empty())
            .with_context(|| format!("profile '{profile_name}' requires a folder"))?;
        let interval = properties
            .get("interval")
            .unwrap_or("300")
            .parse::<u64>()
            .with_context(|| format!("profile '{profile_name}' has an invalid interval"))?;
        if interval == 0 {
            bail!("profile '{profile_name}' interval must be at least one second");
        }
        let order =
            PlaylistOrder::parse(properties.get("order").unwrap_or("random"), profile_name)?;
        let mute_video = properties
            .get("mute_video")
            .unwrap_or("true")
            .parse::<bool>()
            .with_context(|| format!("profile '{profile_name}' has an invalid mute_video value"))?;
        profiles.insert(
            profile_name.to_owned(),
            Profile {
                folder: PathBuf::from(shellexpand::tilde(folder).into_owned()),
                interval,
                order,
                mute_video,
            },
        );
    }
    if profiles.is_empty() {
        bail!("no profiles found in {}", config_file.display());
    }
    if !profiles.contains_key(&active_profile) {
        bail!(
            "active profile '{active_profile}' does not exist in {}",
            config_file.display()
        );
    }
    Ok(Config {
        active_profile,
        restore_on_exit,
        profiles,
    })
}

#[cfg(test)]
mod tests {
    use super::PlaylistOrder;
    #[test]
    fn parses_known_playlist_orders() {
        assert_eq!(
            PlaylistOrder::parse("random", "test").unwrap(),
            PlaylistOrder::Random
        );
        assert_eq!(
            PlaylistOrder::parse("alphabetical", "test").unwrap(),
            PlaylistOrder::Alphabetical
        );
        assert!(PlaylistOrder::parse("shuffle", "test").is_err());
    }
}
