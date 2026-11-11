mod api;
mod auth;
mod conf;
mod id;
mod index;
mod parse;
mod serve;
mod state;
mod vault;
mod vt;
mod ws;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "markvault-kernel", version, about = "markvault kernel")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Serve HTTP API + frontend for a workspace
    Serve {
        /// Path to the vault workspace
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        port: Option<u16>,
        /// Serve on 0.0.0.0 (伺服) instead of loopback
        #[arg(long, default_value_t = false)]
        network_serve: bool,
        #[arg(long, default_value_t = false)]
        readonly: bool,
    },
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Cmd::Serve {
            workspace,
            port,
            network_serve,
            readonly,
        } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                serve::serve(serve::ServeOpts {
                    workspace,
                    port,
                    network_serve,
                    readonly,
                })
                .await
            });
        }
    }
}
