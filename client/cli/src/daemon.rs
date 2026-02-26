use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn pid_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let dir = home.join(".local/share/entanglement");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("tangle.pid"))
}

/// Check if daemon is running. Returns PID if alive.
pub fn check_running() -> anyhow::Result<Option<u32>> {
    let path = pid_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let pid_str = fs::read_to_string(&path)?;
    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
    };

    // Check if process is alive
    #[cfg(unix)]
    {
        let alive = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !alive {
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
    }

    Ok(Some(pid))
}

/// Start the daemon by spawning a background process.
pub fn start() -> anyhow::Result<u32> {
    if let Some(pid) = check_running()? {
        anyhow::bail!("Already running (pid {})", pid);
    }

    let exe = std::env::current_exe()?;
    let child = Command::new(&exe)
        .args(["start", "--foreground"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let pid = child.id();
    write_pid(pid)?;
    Ok(pid)
}

/// Write a PID to the PID file (used by foreground mode too).
pub fn write_pid(pid: u32) -> anyhow::Result<()> {
    let path = pid_path()?;
    fs::write(&path, pid.to_string())?;
    Ok(())
}

/// Remove the PID file.
pub fn remove_pid() -> anyhow::Result<()> {
    let path = pid_path()?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// Stop the daemon process.
pub fn stop() -> anyhow::Result<()> {
    match check_running()? {
        Some(pid) => {
            #[cfg(unix)]
            {
                Command::new("kill").args([&pid.to_string()]).status()?;
            }
            #[cfg(not(unix))]
            {
                Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .status()?;
            }
            let _ = remove_pid();
            println!("tangle stopped (pid {})", pid);
        }
        None => {
            println!("tangle not running");
        }
    }
    Ok(())
}
