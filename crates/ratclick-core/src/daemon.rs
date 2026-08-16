//! Finding and launching `ratclickd`.
//!
//! Shared by the CLI and the GUI so there is one answer to "where is the
//! daemon and how do I start it", and so a developer build never accidentally
//! drives an installed copy.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Where the package installs the daemon. It lives in libexec because it is not
/// meant to be run by hand.
pub const LIBEXEC_DAEMON: &str = "/usr/libexec/ratclick/ratclickd";

/// The systemd user unit the package ships.
pub const UNIT: &str = "ratclick.service";

/// Locate the daemon binary.
///
/// A sibling of the current executable wins, so `target/debug/ratclick` starts
/// `target/debug/ratclickd` rather than whatever is installed system-wide.
pub fn find() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(sibling) = exe.parent().map(|d| d.join("ratclickd")) {
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }
    if Path::new(LIBEXEC_DAEMON).is_file() {
        return Some(PathBuf::from(LIBEXEC_DAEMON));
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join("ratclickd"))
            .find(|p| p.is_file())
    })
}

/// Is the systemd user unit installed?
pub fn systemd_unit_exists() -> bool {
    Command::new("systemctl")
        .args(["--user", "cat", UNIT])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// How the daemon was launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launched {
    /// `systemctl --user start` reported success.
    Systemd,
    /// The binary was run directly.
    Direct,
}

/// Start the daemon, detached from this process.
///
/// Prefers systemd so that `systemctl --user status ratclick` reflects reality
/// and the daemon outlives the shell that started it.
///
/// Note that systemd starting the unit does **not** guarantee the daemon lands
/// on the bus the caller is talking to: the user manager has its own
/// `DBUS_SESSION_BUS_ADDRESS`, so under a scratch or nested bus the daemon
/// appears somewhere else entirely. Callers must wait for the name and fall
/// back to [`spawn_direct`] if it never shows up — which is why this reports
/// how it started things.
pub fn spawn() -> Result<Launched> {
    if systemd_unit_exists() {
        let started = Command::new("systemctl")
            .args(["--user", "start", UNIT])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if started {
            return Ok(Launched::Systemd);
        }
    }
    spawn_direct().map(|()| Launched::Direct)
}

/// Run the daemon binary directly, inheriting this process's environment — and
/// so this process's session bus.
pub fn spawn_direct() -> Result<()> {
    let path = find().context(
        "cannot find `ratclickd` — install the ratclick package, or build the workspace first",
    )?;
    Command::new(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("launching {}", path.display()))?;
    Ok(())
}

/// Ask systemd to stop the daemon. Returns false when there is no unit to stop,
/// in which case the caller should fall back to the D-Bus `Quit` method.
pub fn stop_via_systemd() -> bool {
    if !systemd_unit_exists() {
        return false;
    }
    Command::new("systemctl")
        .args(["--user", "stop", UNIT])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
