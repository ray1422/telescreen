//! The `telescreen` binary.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use telescreen_app::paths;
use telescreen_core::config::Config;
use telescreen_core::ids::ComputerId;
use telescreen_transport::{Fingerprint, Identity, TrustStore};

#[derive(Parser)]
#[command(
    version,
    about = "Control computers that share a display with one keyboard and mouse"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run Telescreen as the computer COMPUTER of the config (PEER-3).
    Run {
        /// The config file (docs/config.md).
        #[arg(long)]
        config: PathBuf,
        /// The ID of this computer in the config. Telescreen does not guess it.
        #[arg(long)]
        computer: String,
        /// The Unix socket of the debug protocol (dev builds only,
        /// docs/debug-protocol.md).
        #[cfg(feature = "debug-protocol")]
        #[arg(long)]
        debug_socket: Option<PathBuf>,
    },
    /// Show the fingerprint of this computer, for `telescreen trust` on the other peers.
    Fingerprint,
    /// Trust the key of a different computer (ADR 0004). Run it on each peer.
    Trust {
        computer: String,
        fingerprint: String,
    },
    /// Validate a config file. It does not run the operations.
    CheckConfig { config: PathBuf },
    /// Show the displays, their metadata for driver helpers, and the input
    /// capabilities, as JSON. It opens no window.
    Displays,
}

fn main() -> ExitCode {
    match run(Cli::parse().command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("telescreen: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Run {
            config,
            computer,
            #[cfg(feature = "debug-protocol")]
            debug_socket,
        } => {
            let parsed = Config::load(&config).map_err(|e| e.to_string())?;
            let computer = ComputerId(computer);
            if parsed.computer(&computer).is_none() {
                return Err(format!("the config has no computer {computer}"));
            }
            #[cfg(not(feature = "debug-protocol"))]
            let debug_socket = None;
            telescreen_app::ui::run(parsed, computer, config, debug_socket)
        }
        Command::Fingerprint => {
            let identity =
                Identity::load_or_create(&paths::identity_dir()?).map_err(|e| e.to_string())?;
            println!("{}", identity.fingerprint());
            Ok(())
        }
        Command::Trust {
            computer,
            fingerprint,
        } => {
            let fingerprint: Fingerprint = fingerprint.parse().map_err(|e| format!("{e}"))?;
            let path = paths::trust_file()?;
            let mut store = TrustStore::load(&path).map_err(|e| e.to_string())?;
            let added = store.trust(ComputerId(computer.clone()), fingerprint);
            store.save(&path).map_err(|e| e.to_string())?;
            if added {
                println!("trusted {fingerprint} for {computer}");
            } else {
                println!("{fingerprint} was already trusted for {computer}");
            }
            Ok(())
        }
        Command::Displays => {
            println!("{}", telescreen_app::ui::describe_displays()?);
            Ok(())
        }
        Command::CheckConfig { config } => {
            let parsed = Config::load(&config).map_err(|e| e.to_string())?;
            println!(
                "VALID: {} ({} computers; operations not run)",
                config.display(),
                parsed.computers.len()
            );
            Ok(())
        }
    }
}
