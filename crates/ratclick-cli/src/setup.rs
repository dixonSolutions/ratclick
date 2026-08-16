//! The terminal first-run wizard.
//!
//! Runs automatically when `ratclick` is invoked with an empty configuration,
//! and on demand via `ratclick setup`. The GUI has its own version of this flow
//! with live key capture; here the user types the accelerator, because reading
//! a raw key combination from a terminal is unreliable across terminals and
//! would be a worse experience than typing `<Super><Shift>c`.

use std::io::{self, IsTerminal, Write};

use anyhow::{Context, Result};
use ratclick_core::accel::Accel;
use ratclick_core::config::{Button, ClickMode, Config, ShortcutBackend, MAX_CPM, MIN_CPM};
use ratclick_core::shortcut;

pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn prompt(question: &str, default: &str) -> Result<String> {
    if default.is_empty() {
        print!("{question}: ");
    } else {
        print!("{question} [{default}]: ");
    }
    io::stdout().flush()?;
    let mut line = String::new();
    let n = io::stdin().read_line(&mut line).context("reading input")?;
    if n == 0 {
        // EOF — treat as "accept the default" rather than looping forever.
        println!();
        return Ok(default.to_string());
    }
    let line = line.trim();
    Ok(if line.is_empty() {
        default.to_string()
    } else {
        line.to_string()
    })
}

fn confirm(question: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "Y/n" } else { "y/N" };
    loop {
        let answer = prompt(&format!("{question} ({hint})"), "")?;
        match answer.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default_yes),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("  Please answer y or n."),
        }
    }
}

fn heading(text: &str) {
    println!("\n\x1b[1m{text}\x1b[0m");
}

/// Run the wizard and return the resulting config, already saved and applied.
pub async fn run(existing: Config) -> Result<Config> {
    let mut cfg = existing;

    println!("\n\x1b[1m🐀 Welcome to RatClick\x1b[0m");
    println!("Let's set up your auto-clicker. Press Enter to accept the [default].");

    // ---- Clicking ------------------------------------------------------
    heading("1. How fast should it click?");
    println!("  Clicks per minute. 600 is ten clicks a second; the maximum is {MAX_CPM}.");
    cfg.click.cpm = loop {
        let raw = prompt("  Clicks per minute", &cfg.click.cpm.to_string())?;
        match raw.parse::<u32>() {
            Ok(v) if (MIN_CPM..=MAX_CPM).contains(&v) => break v,
            Ok(_) => println!("  Enter a number between {MIN_CPM} and {MAX_CPM}."),
            Err(_) => println!("  That is not a number."),
        }
    };

    heading("2. Which mouse button?");
    cfg.click.button = loop {
        let raw = prompt("  Button (left/right/middle)", cfg.click.button.as_str())?;
        match Button::from_str_opt(&raw) {
            Some(b) => break b,
            None => println!("  Choose left, right or middle."),
        }
    };

    heading("3. Should a run stop on its own?");
    println!("  endless — clicks until you toggle it off");
    println!("  timed   — clicks for a fixed length of time, then stops");
    cfg.click.mode = loop {
        let raw = prompt("  Mode (endless/timed)", cfg.click.mode.as_str())?;
        match raw.trim().to_ascii_lowercase().as_str() {
            "endless" | "e" => break ClickMode::Endless,
            "timed" | "t" => break ClickMode::Timed,
            _ => println!("  Choose endless or timed."),
        }
    };

    if cfg.click.mode == ClickMode::Timed {
        let (dh, dm) = cfg.click.duration_hm();
        let hours = loop {
            let raw = prompt("  Hours", &dh.to_string())?;
            match raw.parse::<u32>() {
                Ok(v) if v <= 24 => break v,
                _ => println!("  Enter a whole number of hours, 0-24."),
            }
        };
        let minutes = loop {
            let raw = prompt("  Minutes", &dm.to_string())?;
            match raw.parse::<u32>() {
                Ok(v) if v < 60 => break v,
                _ => println!("  Enter a whole number of minutes, 0-59."),
            }
        };
        if hours == 0 && minutes == 0 {
            println!("  A zero-length run makes no sense; using 1 minute.");
        }
        cfg.click.set_duration_hm(hours, minutes);
        let (h, m) = cfg.click.duration_hm();
        println!(
            "  Each run will last {h}h {m:02}m ({} minutes).",
            cfg.click.duration_minutes
        );
    }

    // ---- Shortcut ------------------------------------------------------
    heading("4. How should the toggle shortcut be registered?");
    let statuses = shortcut::backend_statuses();
    for s in &statuses {
        let mark = if s.available {
            "\x1b[32m✓\x1b[0m"
        } else {
            "\x1b[33m✗\x1b[0m"
        };
        println!("  {mark} {:<10} {}", s.backend.as_str(), s.detail);
    }

    let default_backend = statuses
        .iter()
        .find(|s| s.available && s.backend != ShortcutBackend::None)
        .map(|s| s.backend)
        .unwrap_or(ShortcutBackend::None);

    cfg.shortcut.backend = loop {
        let raw = prompt(
            "  Backend (gnome/extension/keyd/none)",
            default_backend.as_str(),
        )?;
        match ShortcutBackend::from_str_opt(&raw) {
            Some(b) => {
                let st = statuses.iter().find(|s| s.backend == b);
                if let Some(st) = st {
                    if !st.available {
                        println!("  \x1b[33m{}\x1b[0m", st.detail);
                        if !confirm("  Use it anyway?", false)? {
                            continue;
                        }
                    }
                }
                break b;
            }
            None => println!("  Choose gnome, extension, keyd or none."),
        }
    };

    if cfg.shortcut.backend == ShortcutBackend::None {
        cfg.shortcut.bindings.clear();
        println!("  No global shortcut. Toggle with `ratclick toggle` or from the app.");
    } else {
        heading("5. Which key combination?");
        println!("  Examples: <Super><Shift>c   <Control><Alt>k   F9");
        cfg.shortcut.bindings = collect_bindings(&cfg.shortcut.bindings)?;
    }

    // ---- Save and apply ------------------------------------------------
    cfg.setup_complete = true;
    for note in cfg.normalise() {
        println!("  note: {note}");
    }
    cfg.save().context("saving the configuration")?;
    println!("\nSaved to {}", Config::path()?.display());

    apply_and_report(&cfg)?;

    heading("Ready");
    let (h, m) = cfg.click.duration_hm();
    let run_for = match cfg.click.mode {
        ClickMode::Endless => "until you toggle it off".to_string(),
        ClickMode::Timed => format!("for {h}h {m:02}m"),
    };
    println!(
        "  {} clicks/minute, {} button, running {run_for}.",
        cfg.click.cpm,
        cfg.click.button.as_str()
    );
    if let Some(a) = cfg.shortcut.bindings.first() {
        println!(
            "  Press \x1b[1m{}\x1b[0m to start and stop clicking.",
            a.to_display()
        );
    }
    println!("  `ratclick gui` opens the app, `ratclick status` shows what it is doing.");

    Ok(cfg)
}

/// Ask for one or more accelerators, checking each against the desktop.
fn collect_bindings(current: &[Accel]) -> Result<Vec<Accel>> {
    let default = current
        .first()
        .map(|a| a.to_gtk())
        .unwrap_or_else(|| "<Super><Shift>c".to_string());

    let mut chosen: Vec<Accel> = Vec::new();
    loop {
        let raw = if chosen.is_empty() {
            prompt("  Shortcut", &default)?
        } else {
            prompt("  Another shortcut (blank to finish)", "")?
        };
        if raw.trim().is_empty() {
            if chosen.is_empty() {
                println!("  You need at least one, or pick the `none` backend.");
                continue;
            }
            break;
        }

        let accel = match Accel::parse(&raw) {
            Ok(a) => a,
            Err(e) => {
                println!("  \x1b[31m{e}\x1b[0m");
                continue;
            }
        };
        if let Err(e) = accel.validate() {
            println!("  \x1b[31m{e}\x1b[0m");
            continue;
        }
        if chosen.contains(&accel) {
            println!("  Already added.");
            continue;
        }

        if !resolve_conflicts(&accel)? {
            continue;
        }

        println!("  \x1b[32m✓\x1b[0m {} is yours.", accel.to_display());
        chosen.push(accel);

        if !confirm("  Add another shortcut?", false)? {
            break;
        }
    }
    Ok(chosen)
}

/// Report anything already bound to `accel` and offer to take it.
///
/// Returns `false` if the user decided to pick a different key instead.
pub fn resolve_conflicts(accel: &Accel) -> Result<bool> {
    let conflicts = shortcut::conflicts(accel);
    if conflicts.is_empty() {
        return Ok(true);
    }

    println!(
        "  \x1b[33m{} is already taken by:\x1b[0m",
        accel.to_display()
    );
    for c in &conflicts {
        println!("    • {}", c.describe());
    }

    if !confirm("  Take it anyway?", true)? {
        return Ok(false);
    }

    let (taken, refused) = shortcut::force_take(accel)?;
    for t in &taken {
        println!("    unbound {t}");
    }
    for r in &refused {
        println!(
            "    \x1b[33mstill bound:\x1b[0m {r} — RatClick will not edit somebody else's keyd \
             mapping; remove it by hand if the shortcut misbehaves"
        );
    }
    Ok(true)
}

/// Install the shortcut and say what happened.
pub fn apply_and_report(cfg: &Config) -> Result<()> {
    if cfg.shortcut.backend == ShortcutBackend::None {
        return Ok(());
    }
    match shortcut::apply(cfg) {
        Ok(report) => {
            for note in &report.notes {
                println!("  {note}");
            }
            // Read the binding back rather than trusting the write, so a
            // silently-failing gsettings call cannot look like success.
            let installed = shortcut::installed(report.backend);
            let expected = &cfg.shortcut.bindings;
            if installed.iter().collect::<Vec<_>>() == expected.iter().collect::<Vec<_>>() {
                println!(
                    "  \x1b[32m✓\x1b[0m shortcut registered with the {} backend",
                    report.backend.as_str()
                );
            } else {
                println!(
                    "  \x1b[33m!\x1b[0m shortcut was written but reads back as {:?} — expected {:?}",
                    installed.iter().map(|a| a.to_gtk()).collect::<Vec<_>>(),
                    expected.iter().map(|a| a.to_gtk()).collect::<Vec<_>>()
                );
            }
            Ok(())
        }
        Err(e) => {
            println!("  \x1b[31m!\x1b[0m could not register the shortcut: {e}");
            if cfg.shortcut.backend == ShortcutBackend::Keyd {
                println!("    keyd writes to /etc/keyd, so try: sudo ratclick shortcut apply");
            }
            Ok(())
        }
    }
}
