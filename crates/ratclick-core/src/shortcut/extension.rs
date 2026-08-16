//! The GNOME Shell extension shortcut backend.
//!
//! The extension registers the accelerator with the compositor directly via
//! `Main.wm.addKeybinding`, which is the most reliable way to get a global key
//! on Wayland. All this module does is write the accelerator into the
//! extension's own gsettings schema; the extension picks the change up live.

use std::process::Command;

use anyhow::{Context, Result};

use super::gvariant::{format_string_list, parse_string_list};
use crate::accel::Accel;

pub const UUID: &str = "ratclick@dixonsolutions.github.io";
pub const SCHEMA: &str = "org.gnome.shell.extensions.ratclick";
pub const KEY: &str = "toggle-clicking";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub is_available: bool,
    pub detail: String,
}

/// The extension is usable when its schema is installed *and* the extension is
/// enabled — an installed-but-disabled extension will not register the key.
pub fn availability() -> Status {
    if !schema_installed() {
        return Status {
            is_available: false,
            detail: "the RatClick GNOME Shell extension is not installed".into(),
        };
    }
    match enabled_state() {
        Some(true) => Status {
            is_available: true,
            detail: "RatClick Shell extension is enabled".into(),
        },
        Some(false) => Status {
            is_available: false,
            detail: format!("extension installed but disabled (`gnome-extensions enable {UUID}`)"),
        },
        None => Status {
            is_available: false,
            detail: "could not query gnome-extensions".into(),
        },
    }
}

fn schema_installed() -> bool {
    Command::new(super::gsettings_bin())
        .args(["list-keys", SCHEMA])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn enabled_state() -> Option<bool> {
    let out = Command::new("gnome-extensions")
        .args(["info", UUID])
        .output()
        .ok()?;
    if !out.status.success() {
        return Some(false);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(
        text.lines()
            .find_map(|l| l.trim().strip_prefix("State:"))
            .map(|s| s.trim().eq_ignore_ascii_case("ENABLED"))
            .unwrap_or(false),
    )
}

fn gsettings(args: &[&str]) -> Result<String> {
    let out = Command::new(super::gsettings_bin())
        .args(args)
        .output()
        .with_context(|| format!("running `gsettings {}`", args.join(" ")))?;
    if !out.status.success() {
        anyhow::bail!(
            "`gsettings {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn install(accels: &[Accel]) -> Result<()> {
    anyhow::ensure!(
        schema_installed(),
        "the RatClick Shell extension's settings schema is not installed"
    );
    let list: Vec<String> = accels.iter().map(|a| a.to_gtk()).collect();
    gsettings(&["set", SCHEMA, KEY, &format_string_list(&list)])?;
    Ok(())
}

pub fn uninstall() -> Result<()> {
    if !schema_installed() {
        return Ok(());
    }
    gsettings(&["set", SCHEMA, KEY, "@as []"])?;
    Ok(())
}

pub fn installed() -> Vec<Accel> {
    if !schema_installed() {
        return Vec::new();
    }
    Command::new(super::gsettings_bin())
        .args(["get", SCHEMA, KEY])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            parse_string_list(&String::from_utf8_lossy(&o.stdout))
                .iter()
                .filter_map(|s| Accel::parse(s).ok())
                .collect()
        })
        .unwrap_or_default()
}
