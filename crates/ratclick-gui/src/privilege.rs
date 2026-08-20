//! Narrow privilege boundary for keyd configuration changes.
//!
//! The GUI never runs as root. Only the existing CLI shortcut subcommand is
//! elevated, with the current user's configuration path passed explicitly.

use std::process::Command;

use anyhow::{Context, Result};
use ratclick_core::config::Config;

#[derive(Debug, Clone, Copy)]
pub enum KeydAction {
    Apply,
    Clear,
}

impl KeydAction {
    fn verb(self) -> &'static str {
        match self {
            KeydAction::Apply => "apply",
            KeydAction::Clear => "clear",
        }
    }
}

pub fn run_keyd_action(action: KeydAction) -> Result<()> {
    let config = Config::path()?;
    let status = Command::new("pkexec")
        .arg("env")
        .arg(format!("RATCLICK_CONFIG={}", config.display()))
        .args(["ratclick", "shortcut", action.verb()])
        .status()
        .context("running pkexec")?;

    anyhow::ensure!(
        status.success(),
        "the privileged shortcut command was cancelled or failed"
    );
    Ok(())
}
