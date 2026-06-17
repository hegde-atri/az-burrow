mod azure;
mod config;
mod model;
mod tui;

use crate::azure::cert::CertManager;
use crate::azure::tunnel::TunnelManager;
use crate::model::Machine;
use color_eyre::eyre::Result;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;

const VERSION: &str = "0.2.0";

fn print_help() {
    print!(
        r#"az-burrow v{VERSION} - A cosy TUI for managing Azure Bastion SSH tunnels

Usage:
  az-burrow [config-file]
  az-burrow -h | --help
  az-burrow --version

Arguments:
  config-file    Path to YAML configuration file (default: burrow.config.yaml)

Configuration:
  Looks for a config file in this order:
    1. The path you pass as an argument
    2. ./burrow.config.yaml
    3. ~/.config/burrow.config.yaml

For more information:
  https://github.com/hegde-atri/az-burrow
"#
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(first) = args.first() {
        match first.as_str() {
            "-h" | "--help" => { print_help(); return Ok(()); }
            "--version" => { println!("Az-Burrow v{VERSION}"); return Ok(()); }
            _ => {}
        }
    }

    let config_path = config::resolve_config_path(args.first().map(|s| s.as_str()))?;
    let cfg = config::load(&config_path)?;

    let machines: Vec<Machine> = cfg.machines.into_iter().map(|m| Machine {
        name: m.name, resource_group: m.resource_group, target_resource_id: m.target_resource_id,
        bastion_name: m.bastion_name, bastion_resource_group: m.bastion_resource_group,
        bastion_subscription: m.bastion_subscription, ssh_config_path: m.ssh_config_path,
    }).collect();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let tunnel_mgr = TunnelManager::new(tx.clone());
    let cert_mgr = CertManager::new(tx.clone());

    for m in &machines {
        if let Some(p) = &m.ssh_config_path {
            if !p.is_empty() { cert_mgr.register(&m.name, p); }
        }
    }
    cert_mgr.start_monitoring();

    install_panic_hook();
    enable_raw_mode()?;
    // If entering the alternate screen fails after raw mode is enabled, restore
    // raw mode before returning so we never leave the terminal in a broken state
    // (the panic hook only covers panics, not `?` early returns).
    if let Err(e) = execute!(stdout(), EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(e.into());
    }
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut app = tui::app::App::new(VERSION.to_string(), machines, tunnel_mgr, cert_mgr);
    let run_result = app.run(&mut terminal, rx).await;

    // Belt-and-suspenders: ensure no `az` child survives regardless of how run()
    // exited. The happy path already called stop_all() inside run(); this is
    // idempotent and also covers the case where run() returned an error early.
    app.tunnel_mgr.stop_all();

    // Always restore the terminal; ignore teardown errors so they can't mask the
    // real run result.
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);

    run_result
}

/// Restore the terminal before printing a panic, so a crash never leaves a broken TTY.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
        original(info);
    }));
}
