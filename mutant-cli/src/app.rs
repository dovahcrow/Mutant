use crate::cli::{Cli, Commands};
use crate::commands;
use anyhow::Result;
use clap::Parser;
use mutant_client::MutantClient;

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    // If no command is provided, run the ls command without history
    if cli.command.is_none() {
        commands::daemon::start_daemon().await?;
        return commands::ls::handle_ls(false).await;
    }

    // We know command is Some at this point, so we can safely unwrap
    let command = cli.command.unwrap();

    // Start daemon for all commands except Daemon
    if !matches!(command, Commands::Daemon { .. }) {
        commands::daemon::start_daemon().await?;
    }

    // Process the command
    match command {
        Commands::Put {
            key,
            file,
            background,
            no_verify,
            public,
            mode,
        } => {
            commands::put::handle_put(
                key,
                file,
                public,
                mode.into(),
                no_verify,
                background,
                cli.quiet,
            )
            .await?;
        }
        Commands::Get {
            key,
            destination_path,
            background,
            public,
            stdout,
        } => {
            commands::get::handle_get(key, destination_path, public, background, stdout, cli.quiet).await?;
        }
        Commands::Rm { key } => {
            commands::rm::handle_rm(key).await?;
        }
        Commands::Mv { old_key, new_key } => {
            commands::mv::handle_mv(old_key, new_key).await?;
        }
        Commands::Ls { history } => {
            commands::ls::handle_ls(history).await?;
        }
        Commands::Stats => {
            commands::stats::handle_stats().await?;
        }
        Commands::Tasks { command } => {
            commands::tasks::handle_tasks(command).await?;
        }
        Commands::Daemon { command, lock_file } => {
            commands::daemon::handle_daemon(command, lock_file).await?;
        }
        Commands::Sync {
            background,
            push_force,
        } => {
            commands::sync::handle_sync(background, push_force, cli.quiet).await?;
        }
        Commands::Purge {
            aggressive,
            background,
        } => {
            commands::purge::handle_purge(aggressive, background, cli.quiet).await?;
        }
        Commands::Import { file_path } => {
            commands::import::handle_import(file_path).await?;
        }
        Commands::Export { destination_path } => {
            commands::export::handle_export(destination_path).await?;
        }
        Commands::HealthCheck {
            key_name,
            background,
            recycle,
        } => {
            commands::health_check::handle_health_check(key_name, background, recycle, cli.quiet)
                .await?;
        }
    }

    Ok(())
}

pub async fn connect_to_daemon() -> Result<MutantClient> {
    let mut client = MutantClient::new();
    client.connect(get_daemon_url()).await?;
    Ok(client)
}

pub fn get_daemon_url() -> &'static str {
    "ws://localhost:3001/ws"
}
