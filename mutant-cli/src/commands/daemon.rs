use crate::cli::DaemonCommands;
use anyhow::Result;
use tokio::process::Command;

pub async fn handle_daemon(command: DaemonCommands) -> Result<()> {
    match command {
        DaemonCommands::Start => start_daemon().await,
        DaemonCommands::Stop => stop_daemon().await,
        DaemonCommands::Restart => restart_daemon().await,
        DaemonCommands::Status => status_daemon().await,
        // DaemonCommands::Logs => logs_daemon().await,
        _ => Err(anyhow::anyhow!("Command not implemented")),
    }
}

pub async fn start_daemon() -> Result<()> {
    match std::fs::read_to_string("/tmp/mutant-daemon.lock") {
        Ok(_pid) => {
            return Ok(());
        }
        Err(_e) => {}
    };

    println!("Starting daemon...");

    let _ = Command::new("mutant-daemon").arg("--ignore-ctrl-c").spawn()?;

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    Ok(())
}

async fn stop_daemon() -> Result<()> {
    // read the pid from the /tmp/mutant-daemon.lock file
    let pid = match std::fs::read_to_string("/tmp/mutant-daemon.lock") {
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
    stop_daemon().await?;
    start_daemon().await?;
    Ok(())
}

async fn status_daemon() -> Result<()> {
    match std::fs::read_to_string("/tmp/mutant-daemon.lock") {
        Ok(pid) => println!("Daemon running with pid: {}", pid),
        Err(_e) => println!("Daemon not running"),
    };

    Ok(())
}
