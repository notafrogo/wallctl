use crate::config::get_config_dir;
use std::fs;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
    let pid_path = get_pid_path();
    if let Ok(content) = fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            // Check if process is actually running via kill -0
            if let Ok(status) = Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                if status.success() {
                    return true;
                }
            }
        }
    }
    cleanup_stale_files();
    false
}

pub fn force_kill_daemon() {
    let pid_path = get_pid_path();
    if let Ok(content) = fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            let _ = Command::new("kill")
                .args(["-15", &pid.to_string()])
                .status();
        }
    }
    cleanup_stale_files();
}

pub fn send_command(command: &str) -> Result<String, String> {
    let socket_path = get_socket_path();
    if !socket_path.exists() {
        return Err("Daemon is not running.".into());
    }

    let mut stream = UnixStream::connect(socket_path).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;

    stream.write_all(command.as_bytes()).map_err(|e| e.to_string())?;
    stream.shutdown(Shutdown::Write).map_err(|e| e.to_string())?;

    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| e.to_string())?;
    Ok(response)
}

pub fn create_listener() -> std::io::Result<UnixListener> {
    let socket_path = get_socket_path();
    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }
    let listener = UnixListener::bind(&socket_path)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}
