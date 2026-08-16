use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use niri_scratch::{Config, ControlRequest, default_config_path, run_daemon, send_control};

#[derive(Debug, Parser)]
#[command(name = "niri-scratch", version, about)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Daemon,
    Toggle { scratchpad: String },
    Show { scratchpad: String },
    Hide { scratchpad: String },
    HideAll,
    Status { scratchpad: Option<String> },
    List,
    Doctor,
    Ping,
    CheckConfig,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.config.unwrap_or(default_config_path()?);
    let config = Config::load(&path)?;
    if matches!(cli.command, Command::CheckConfig) {
        println!("configuration is valid: {}", path.display());
        return Ok(());
    }
    if matches!(cli.command, Command::Daemon) {
        return run_daemon(config);
    }
    let request = match cli.command {
        Command::Toggle { scratchpad } => ControlRequest::Toggle { scratchpad },
        Command::Show { scratchpad } => ControlRequest::Show { scratchpad },
        Command::Hide { scratchpad } => ControlRequest::Hide { scratchpad },
        Command::HideAll => ControlRequest::HideAll,
        Command::Status { scratchpad } => ControlRequest::Status { scratchpad },
        Command::List => ControlRequest::List,
        Command::Doctor => ControlRequest::Doctor,
        Command::Ping => ControlRequest::Ping,
        Command::Daemon | Command::CheckConfig => unreachable!(),
    };
    let response = send_control(&config, &request)?;
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else if let Some(data) = &response.data {
        println!(
            "{}\n{}",
            response.message,
            serde_json::to_string_pretty(data)?
        );
    } else {
        println!("{}", response.message);
    }
    if !response.ok {
        bail!("daemon rejected request");
    }
    Ok(())
}
