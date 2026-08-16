//! `ratclick doctor` — check everything that has to be true for clicking to work.
//!
//! Every failure mode RatClick has in the field is an environment problem
//! rather than a bug: no `/dev/uinput`, not in the `input` group, keyd stopped,
//! extension disabled. This turns "it doesn't click" into a specific fix.

use std::path::Path;

use anyhow::Result;
use ratclick_core::config::{Config, ShortcutBackend};
use ratclick_core::shortcut;

use crate::client;

struct Report {
    problems: usize,
}

impl Report {
    fn ok(&self, what: &str, detail: impl AsRef<str>) {
        println!("  \x1b[32m✓\x1b[0m {what:<22} {}", detail.as_ref());
    }
    fn warn(&mut self, what: &str, detail: impl AsRef<str>) {
        println!("  \x1b[33m!\x1b[0m {what:<22} {}", detail.as_ref());
    }
    fn bad(&mut self, what: &str, detail: impl AsRef<str>) {
        self.problems += 1;
        println!("  \x1b[31m✗\x1b[0m {what:<22} {}", detail.as_ref());
    }
}

pub async fn run() -> Result<()> {
    let mut r = Report { problems: 0 };
    println!("\x1b[1mRatClick {}\x1b[0m", ratclick_core::VERSION);

    println!("\n\x1b[1mInput device\x1b[0m");
    check_uinput(&mut r);

    println!("\n\x1b[1mSession\x1b[0m");
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());
    r.ok("session", format!("{session} / {desktop}"));
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        r.bad("session bus", "DBUS_SESSION_BUS_ADDRESS is not set");
    } else {
        r.ok("session bus", "available");
    }

    println!("\n\x1b[1mDaemon\x1b[0m");
    if client::is_running().await {
        let v = client::connect()
            .await?
            .version()
            .await
            .unwrap_or_else(|_| "?".into());
        r.ok("ratclickd", format!("running (version {v})"));
    } else {
        r.warn("ratclickd", "not running (it starts on demand)");
    }
    if client::systemd_unit_exists() {
        r.ok("systemd unit", "ratclick.service is installed");
    } else {
        r.warn(
            "systemd unit",
            "not installed; the daemon is launched directly",
        );
    }

    println!("\n\x1b[1mConfiguration\x1b[0m");
    let cfg = match Config::load() {
        Ok(c) => {
            if c.needs_setup() {
                r.warn("config", "no setup yet — run `ratclick setup`");
            } else {
                r.ok(
                    "config",
                    format!(
                        "{} CPM, {} button, {}",
                        c.click.cpm,
                        c.click.button.as_str(),
                        c.click.mode.as_str()
                    ),
                );
            }
            c
        }
        Err(e) => {
            r.bad("config", e.to_string());
            Config::default()
        }
    };

    println!("\n\x1b[1mShortcut backends\x1b[0m");
    for s in shortcut::backend_statuses() {
        if s.backend == ShortcutBackend::None {
            continue;
        }
        let chosen = s.backend == cfg.shortcut.backend;
        let label = if chosen {
            format!("{} (in use)", s.backend.as_str())
        } else {
            s.backend.as_str().to_string()
        };
        if s.available {
            r.ok(&label, &s.detail);
        } else if chosen {
            r.bad(&label, &s.detail);
        } else {
            r.warn(&label, &s.detail);
        }
    }

    println!("\n\x1b[1mShortcut\x1b[0m");
    if cfg.shortcut.backend == ShortcutBackend::None {
        r.warn("binding", "no global shortcut configured");
    } else if cfg.shortcut.bindings.is_empty() {
        r.warn("binding", "backend selected but no key chosen");
    } else {
        let live = shortcut::installed(cfg.shortcut.backend);
        for a in &cfg.shortcut.bindings {
            if live.contains(a) {
                r.ok("binding", format!("{} is registered", a.to_display()));
            } else {
                r.bad(
                    "binding",
                    format!(
                        "{} is configured but not registered — run `ratclick shortcut apply`",
                        a.to_display()
                    ),
                );
            }
            let others = shortcut::conflicts(a);
            if !others.is_empty() {
                r.warn(
                    "conflict",
                    format!(
                        "{} is also bound to {}",
                        a.to_display(),
                        others
                            .iter()
                            .map(|c| c.describe())
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                );
            }
        }
    }

    println!();
    if r.problems == 0 {
        println!("\x1b[32mNo problems found.\x1b[0m");
    } else {
        println!("\x1b[31m{} problem(s) found.\x1b[0m", r.problems);
        std::process::exit(1);
    }
    Ok(())
}

fn check_uinput(r: &mut Report) {
    let path = Path::new("/dev/uinput");
    if !path.exists() {
        r.bad(
            "/dev/uinput",
            "missing — load the module with `sudo modprobe uinput`",
        );
        return;
    }

    // The only reliable test is to try: ACLs and supplementary groups make
    // permission bits alone a poor predictor.
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(_) => r.ok("/dev/uinput", "writable"),
        Err(e) => {
            r.bad(
                "/dev/uinput",
                format!(
                    "not writable ({e}) — add yourself with \
                     `sudo usermod -aG input $USER`, then log out and back in"
                ),
            );
        }
    }

    let in_group = std::process::Command::new("id")
        .arg("-nG")
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .any(|g| g == "input")
        })
        .unwrap_or(false);
    if in_group {
        r.ok("input group", "you are a member");
    } else {
        r.warn(
            "input group",
            "not a member — works today only because of a udev ACL",
        );
    }
}
