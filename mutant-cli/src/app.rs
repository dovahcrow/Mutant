use crate::cli::{Cli, Commands};
use crate::commands;
use anyhow::Result;
use clap::Parser;
use mutant_client::MutantClient;

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Put {
            key,
            file,
            background,
            no_verify,
            public,
            mode,
        } => {
            commands::put::handle_put(key, file, public, mode.into(), no_verify, background)
                .await?;
        }
        Commands::Get {
            key,
            destination_path,
            background,
            public,
        } => {
            commands::get::handle_get(key, destination_path, public, background).await?;
        }
        Commands::Rm { key } => {
            commands::rm::handle_rm(key).await?;
        }
        Commands::Ls => {
            commands::ls::handle_ls().await?;
        }
        Commands::Stats => {
            commands::stats::handle_stats().await?;
        }
        Commands::Tasks { command } => {
            commands::tasks::handle_tasks(command).await?;
        }
        Commands::Sync {
            background,
            push_force,
        } => {
            commands::sync::handle_sync(background, push_force).await?;
        }
    }

    Ok(())
}

pub async fn connect_to_daemon() -> Result<MutantClient> {
    let mut client = MutantClient::new();
    client.connect("ws://localhost:3030/ws").await?;
    Ok(client)
}
