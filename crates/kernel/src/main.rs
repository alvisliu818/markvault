mod api;
mod auth;
mod conf;
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
        /// Write rolling logs to <workspace>/.markvault/logs/kernel.log (max 5MB x 3)
        #[arg(long, default_value_t = false)]
        log_file: bool,
    },
}

/// Simple size-capped rolling file appender: kernel.log -> kernel.1.log -> kernel.2.log
mod logroll {
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Mutex;

    const MAX_BYTES: u64 = 5 * 1024 * 1024;
    const _KEEP: usize = 3; // kernel.log + 2 rotated

    pub struct Roller {
        path: PathBuf,
        file: Option<File>,
        written: u64,
        lock: Mutex<()>,
    }

    impl Roller {
        pub fn new(path: PathBuf) -> Self {
            let written = path.metadata().map(|m| m.len()).unwrap_or(0);
            Roller { path, file: None, written, lock: Mutex::new(()) }
        }

        fn roll(&mut self) {
            let base = self.path.clone();
            let p1 = base.with_file_name("kernel.1.log");
            let p2 = base.with_file_name("kernel.2.log");
            if p2.exists() {
                let _ = std::fs::remove_file(&p2);
            }
            if p1.exists() {
                let _ = std::fs::rename(&p1, &p2);
            }
            if base.exists() {
                let _ = std::fs::rename(&base, &p1);
            }
            self.file = None;
            self.written = 0;
        }
    }

    impl Write for Roller {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            {
                let _g = self.lock.lock().unwrap();
                drop(_g);
            }
            if self.file.is_none() {
                if let Some(parent) = self.path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                self.file = Some(OpenOptions::new().create(true).append(true).open(&self.path)?);
            }
            if self.written + buf.len() as u64 > MAX_BYTES {
                self.roll();
                self.file = Some(OpenOptions::new().create(true).append(true).open(&self.path)?);
            }
            let n = self.file.as_mut().unwrap().write(buf)?;
            self.written += n as u64;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if let Some(f) = self.file.as_mut() {
                f.flush()?;
            }
            Ok(())
        }
    }
}

fn init_tracing(log_file: bool, workspace: &PathBuf) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info".into());
    if log_file {
        let roller = logroll::Roller::new(workspace.join(".markvault").join("logs").join("kernel.log"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::sync::Mutex::new(roller))
            .with_ansi(false)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Serve {
            workspace,
            port,
            network_serve,
            readonly,
            log_file,
        } => {
            init_tracing(log_file, &workspace);
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
