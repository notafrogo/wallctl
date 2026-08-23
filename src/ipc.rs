use crate::config::get_config_dir;
use anyhow::{Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

pub fn get_socket_path() -> PathBuf {
    get_config_dir().join("daemon.sock")
}
pub fn get_pid_path() -> PathBuf {
    get_config_dir().join("daemon.pid")
}

pub fn cleanup_stale_files() {
    let _ = fs::remove_file(get_socket_path());
    let _ = fs::remove_file(get_pid_path());
}

pub fn is_daemon_running() -> bool {
    matches!(send_command("PING"), Ok(response) if response == "PONG")
}

pub fn send_command(command: &str) -> Result<String> {
    let socket_path = get_socket_path();
    let mut stream = UnixStream::connect(&socket_path)
        .with_context(|| format!("wallctl daemon is not running ({})", socket_path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(command.as_bytes())?;
    stream.shutdown(Shutdown::Write)?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

pub fn create_listener() -> Result<UnixListener> {
    let socket_path = get_socket_path();
    if socket_path.exists() {
        fs::remove_file(&socket_path)?;
    }
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("could not bind {}", socket_path.display()))?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}
