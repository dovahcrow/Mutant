use crate::cli::DaemonCommands;
use anyhow::Result;
use tokio::process::Command;

pub async fn handle_daemon(command: DaemonCommands, lock_file: String) -> Result<()> {
    match command {
        DaemonCommands::Start => start_daemon_with_lock(&lock_file).await,
        DaemonCommands::Stop => stop_daemon_with_lock(&lock_file).await,
        DaemonCommands::Restart => restart_daemon_with_lock(&lock_file).await,
        DaemonCommands::Status => status_daemon_with_lock(&lock_file).await,
        // DaemonCommands::Logs => logs_daemon().await,
        _ => Err(anyhow::anyhow!("Command not implemented")),
    }
}

pub async fn start_daemon() -> Result<()> {
    start_daemon_with_lock("/tmp/mutant-daemon.lock").await
}

pub async fn start_daemon_with_lock(lock_file: &str) -> Result<()> {
    match std::fs::read_to_string(lock_file) {
        Ok(_pid) => {
            return Ok(());
        }
        Err(_e) => {}
    };

    println!("Starting daemon...");

    let lock_file_arg = format!("--lock-file={}", lock_file);
    let daemon_cmd = format!("mutant-daemon --ignore-ctrl-c {} &", lock_file_arg);
    let _ = Command::new("bash").arg("-c").arg(&daemon_cmd).spawn()?;

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    Ok(())
}

async fn stop_daemon() -> Result<()> {
    stop_daemon_with_lock("/tmp/mutant-daemon.lock").await
}

async fn stop_daemon_with_lock(lock_file: &str) -> Result<()> {
    // read the pid from the lock file
    let pid = match std::fs::read_to_string(lock_file) {
        Ok(pid) => pid,
        Err(_e) => {
            println!("Daemon not running");
            return Ok(());
        }
    };
    match Command::new("kill")
        .arg("-15") // SIGTERM
        .arg(pid)
        .output()
        .await
    {
        Ok(_output) => {
            println!("Daemon stopped");
        }
        Err(e) => {
            println!("Failed to stop daemon: {}", e);
            return Ok(());
        }
    };

    Ok(())
}

async fn restart_daemon() -> Result<()> {
    restart_daemon_with_lock("/tmp/mutant-daemon.lock").await
}

async fn restart_daemon_with_lock(lock_file: &str) -> Result<()> {
    stop_daemon_with_lock(lock_file).await?;
    start_daemon_with_lock(lock_file).await?;
    Ok(())
}

async fn status_daemon() -> Result<()> {
    status_daemon_with_lock("/tmp/mutant-daemon.lock").await
}

async fn status_daemon_with_lock(lock_file: &str) -> Result<()> {
    match std::fs::read_to_string(lock_file) {
        Ok(pid) => println!("Daemon running with pid: {}", pid),
        Err(_e) => println!("Daemon not running"),
    };

    Ok(())
}
