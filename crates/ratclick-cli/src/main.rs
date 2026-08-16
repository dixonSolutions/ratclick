//! `ratclick` — the RatClick command line.

mod client;
mod doctor;
mod setup;

use std::process::Command;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use ratclick_core::accel::Accel;
use ratclick_core::config::{Button, ClickMode, Config, ShortcutBackend, MAX_CPM, MIN_CPM};
use ratclick_core::{ipc, shortcut};

#[derive(Parser, Debug)]
#[command(
    name = "ratclick",
    version,
    about = "RatClick — a configurable auto-clicker for GNOME",
    long_about = "RatClick clicks your mouse for you.\n\n\
                  Run `ratclick gui` for the app, or use the subcommands below. \
                  The first run walks you through setup.",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Open the RatClick window.
    Gui,
    /// Start clicking.
    Start,
    /// Stop clicking.
    Stop,
    /// Start clicking if stopped, stop it if running. This is what the shortcut runs.
    Toggle,
    /// Show what RatClick is doing.
    Status {
        /// Print machine-readable `key=value` lines instead of prose.
        #[arg(long)]
        porcelain: bool,
    },
    /// Run the guided first-time setup again.
    Setup,
    /// Start, stop and inspect the background service.
    #[command(subcommand)]
    Daemon(DaemonCmd),
    /// Read and change settings.
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Manage the global toggle shortcut.
    #[command(subcommand)]
    Shortcut(ShortcutCmd),
    /// Check the installation and report anything that would stop RatClick working.
    Doctor,
}

#[derive(Subcommand, Debug)]
enum DaemonCmd {
    /// Launch the background service.
    Start,
    /// Shut the background service down.
    Stop,
    /// Restart the background service.
    Restart,
    /// Is the background service running?
    Status,
    /// Run the service in the foreground (for debugging).
    Run,
}

#[derive(Subcommand, Debug)]
enum ConfigCmd {
    /// Print the current configuration.
    Show,
    /// Print the path of the configuration file.
    Path,
    /// Change a setting.
    Set(ConfigSet),
    /// Delete the configuration, so the next run starts the wizard again.
    Reset {
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args, Debug)]
struct ConfigSet {
    /// One of: cpm, button, mode, duration, hours, minutes, autostart
    key: String,
    /// The new value. `duration` accepts `90`, `1h30m` or `1:30`.
    value: String,
}

#[derive(Subcommand, Debug)]
enum ShortcutCmd {
    /// Show RatClick's shortcut and which backend holds it.
    Show,
    /// List every keyboard shortcut the desktop has bound.
    List {
        /// Only show shortcuts matching this text.
        filter: Option<String>,
    },
    /// Report whether a key combination is free.
    Check { accel: String },
    /// Bind one or more key combinations to the toggle.
    Set(ShortcutSet),
    /// Remove RatClick's shortcut from every backend.
    Clear,
    /// Re-install the configured shortcut. Use with sudo for the keyd backend.
    Apply,
}

#[derive(Args, Debug)]
struct ShortcutSet {
    /// Accelerators, e.g. `<Super><Shift>c`. Give more than one to bind several.
    #[arg(required = true)]
    accels: Vec<String>,
    /// Which mechanism registers the key.
    #[arg(long, value_parser = parse_backend)]
    backend: Option<ShortcutBackend>,
    /// Take the combination even if something else already has it.
    #[arg(long)]
    force: bool,
}

fn parse_backend(s: &str) -> Result<ShortcutBackend, String> {
    ShortcutBackend::from_str_opt(s)
        .ok_or_else(|| format!("expected gnome, extension, keyd or none, got `{s}`"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("\x1b[31mratclick:\x1b[0m {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    // No subcommand: open the GUI if there is a display, otherwise show help.
    let command = match cli.command {
        Some(c) => c,
        None => {
            if std::env::var_os("WAYLAND_DISPLAY").is_some()
                || std::env::var_os("DISPLAY").is_some()
            {
                Cmd::Gui
            } else {
                use clap::CommandFactory;
                Cli::command().print_help()?;
                println!();
                return Ok(());
            }
        }
    };

    match command {
        Cmd::Gui => launch_gui(),
        Cmd::Start => {
            ensure_setup().await?;
            client::connect_or_start().await?.start().await?;
            println!("clicking");
            Ok(())
        }
        Cmd::Stop => {
            // Stopping something that is not running is a success, not an error.
            if !client::is_running().await {
                println!("not running");
                return Ok(());
            }
            client::connect().await?.stop().await?;
            println!("stopped");
            Ok(())
        }
        Cmd::Toggle => {
            ensure_setup().await?;
            let now_running = client::connect_or_start().await?.toggle().await?;
            println!("{}", if now_running { "clicking" } else { "stopped" });
            Ok(())
        }
        Cmd::Status { porcelain } => status(porcelain).await,
        Cmd::Setup => {
            let cfg = Config::load()?;
            setup::run(cfg).await?;
            client::nudge_reload().await;
            Ok(())
        }
        Cmd::Daemon(sub) => daemon(sub).await,
        Cmd::Config(sub) => config_cmd(sub).await,
        Cmd::Shortcut(sub) => shortcut_cmd(sub).await,
        Cmd::Doctor => doctor::run().await,
    }
}

/// Run the wizard if the config is still empty.
///
/// A non-interactive caller (the global shortcut, a script) gets sane defaults
/// written for it instead — being unable to click because nobody answered a
/// prompt would be worse than picking 600 CPM.
async fn ensure_setup() -> Result<()> {
    let cfg = Config::load()?;
    if !cfg.needs_setup() {
        return Ok(());
    }

    if setup::is_interactive() {
        setup::run(cfg).await?;
        client::nudge_reload().await;
    } else {
        let mut cfg = cfg;
        cfg.setup_complete = true;
        cfg.save()?;
        eprintln!(
            "ratclick: no configuration yet — using defaults. Run `ratclick setup` or \
             `ratclick gui` to choose your own."
        );
    }
    Ok(())
}

fn launch_gui() -> Result<()> {
    let candidates = gui_candidates();
    for path in &candidates {
        match Command::new(path).spawn() {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("launching {}", path.display())),
        }
    }
    anyhow::bail!(
        "cannot find `ratclick-gui`. Install the `ratclick` package, or build the workspace \
         with `cargo build --release`."
    )
}

fn gui_candidates() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    // A sibling of this binary first, so a dev build launches the dev GUI.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("ratclick-gui"));
        }
    }
    out.push("/usr/bin/ratclick-gui".into());
    out.push("ratclick-gui".into());
    out
}

async fn status(porcelain: bool) -> Result<()> {
    if !client::is_running().await {
        if porcelain {
            println!("daemon=stopped");
        } else {
            println!("RatClick is not running.");
        }
        return Ok(());
    }

    let proxy = client::connect().await?;
    let s = proxy.status().await?;
    let st = ipc::Status {
        running: s.0,
        cpm: s.1,
        button: s.2,
        mode: s.3,
        remaining_seconds: s.4,
        clicks: s.5,
    };

    if porcelain {
        println!("daemon=running");
        println!("clicking={}", st.running);
        println!("cpm={}", st.cpm);
        println!("button={}", st.button);
        println!("mode={}", st.mode);
        println!("remaining_seconds={}", st.remaining_seconds);
        println!("clicks={}", st.clicks);
    } else {
        println!("RatClick is {}", st.summary());
        if st.running {
            println!("  {} clicks so far this run", st.clicks);
        }
    }
    Ok(())
}

async fn daemon(sub: DaemonCmd) -> Result<()> {
    match sub {
        DaemonCmd::Start => {
            if client::is_running().await {
                println!("already running");
                return Ok(());
            }
            client::spawn_daemon()?;
            client::wait_until_up(std::time::Duration::from_secs(5)).await?;
            println!("started");
            Ok(())
        }
        DaemonCmd::Stop => {
            if !client::is_running().await {
                println!("not running");
                return Ok(());
            }
            // Prefer systemd so the unit is marked inactive rather than being
            // restarted by it a moment later.
            if client::systemd_unit_exists() {
                let _ = Command::new("systemctl")
                    .args(["--user", "stop", "ratclick.service"])
                    .status();
            } else {
                let _ = client::connect().await?.quit().await;
            }
            client::wait_until_down(std::time::Duration::from_secs(5)).await?;
            println!("stopped");
            Ok(())
        }
        DaemonCmd::Restart => {
            if client::is_running().await {
                Box::pin(daemon(DaemonCmd::Stop)).await?;
            }
            Box::pin(daemon(DaemonCmd::Start)).await
        }
        DaemonCmd::Status => {
            if client::is_running().await {
                let v = client::connect().await?.version().await.unwrap_or_default();
                println!("running (ratclickd {v})");
            } else {
                println!("stopped");
            }
            Ok(())
        }
        DaemonCmd::Run => {
            let path = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("ratclickd")))
                .filter(|p| p.is_file())
                .unwrap_or_else(|| "/usr/libexec/ratclick/ratclickd".into());
            let err = exec_replacing(&path, &["--verbose"]);
            Err(err).with_context(|| format!("running {}", path.display()))
        }
    }
}

/// Replace this process with another binary, so signals and the exit status
/// belong to the daemon rather than to a wrapper.
fn exec_replacing(path: &std::path::Path, args: &[&str]) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    Command::new(path).args(args).exec()
}

async fn config_cmd(sub: ConfigCmd) -> Result<()> {
    match sub {
        ConfigCmd::Path => {
            println!("{}", Config::path()?.display());
            Ok(())
        }
        ConfigCmd::Show => {
            let cfg = Config::load()?;
            let (h, m) = cfg.click.duration_hm();
            println!("config      {}", Config::path()?.display());
            println!("setup done  {}", cfg.setup_complete);
            println!("cpm         {}", cfg.click.cpm);
            println!("button      {}", cfg.click.button.as_str());
            println!("mode        {}", cfg.click.mode.as_str());
            if cfg.click.mode == ClickMode::Timed {
                println!(
                    "duration    {h}h {m:02}m ({} minutes)",
                    cfg.click.duration_minutes
                );
            }
            println!("autostart   {}", cfg.start_clicking_on_launch);
            println!("backend     {}", cfg.shortcut.backend.as_str());
            if cfg.shortcut.bindings.is_empty() {
                println!("shortcut    (none)");
            } else {
                for a in &cfg.shortcut.bindings {
                    println!("shortcut    {}  ({})", a.to_display(), a.to_gtk());
                }
            }
            Ok(())
        }
        ConfigCmd::Set(set) => {
            let mut cfg = Config::load()?;
            apply_setting(&mut cfg, &set.key, &set.value)?;
            for note in cfg.normalise() {
                println!("note: {note}");
            }
            cfg.setup_complete = true;
            cfg.save()?;
            client::nudge_reload().await;
            println!("{} = {}", set.key, set.value);
            Ok(())
        }
        ConfigCmd::Reset { yes } => {
            let path = Config::path()?;
            if !yes && setup::is_interactive() {
                println!("This deletes {} and unbinds the shortcut.", path.display());
                anyhow::ensure!(ask_yes_no("Continue?")?, "cancelled");
            }
            let _ = shortcut::remove_all();
            match std::fs::remove_file(&path) {
                Ok(()) => println!("removed {}", path.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("nothing to remove")
                }
                Err(e) => return Err(e).context("removing the configuration"),
            }
            Ok(())
        }
    }
}

fn ask_yes_no(q: &str) -> Result<bool> {
    use std::io::Write;
    print!("{q} (y/N): ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn apply_setting(cfg: &mut Config, key: &str, value: &str) -> Result<()> {
    match key.trim().to_ascii_lowercase().as_str() {
        "cpm" | "clicks-per-minute" => {
            let v: u32 = value.parse().context("cpm must be a whole number")?;
            anyhow::ensure!(
                (MIN_CPM..=MAX_CPM).contains(&v),
                "cpm must be between {MIN_CPM} and {MAX_CPM}"
            );
            cfg.click.cpm = v;
        }
        "button" => {
            cfg.click.button =
                Button::from_str_opt(value).context("button must be left, right or middle")?;
        }
        "mode" => {
            cfg.click.mode = match value.trim().to_ascii_lowercase().as_str() {
                "endless" | "e" => ClickMode::Endless,
                "timed" | "t" => ClickMode::Timed,
                _ => anyhow::bail!("mode must be endless or timed"),
            };
        }
        "duration" => {
            cfg.click.duration_minutes = parse_duration_minutes(value)?;
            cfg.click.mode = ClickMode::Timed;
        }
        "hours" => {
            let h: u32 = value.parse().context("hours must be a whole number")?;
            let (_, m) = cfg.click.duration_hm();
            cfg.click.set_duration_hm(h, m);
            cfg.click.mode = ClickMode::Timed;
        }
        "minutes" => {
            let m: u32 = value.parse().context("minutes must be a whole number")?;
            anyhow::ensure!(m < 60, "minutes must be 0-59; use `duration` for longer");
            let (h, _) = cfg.click.duration_hm();
            cfg.click.set_duration_hm(h, m);
            cfg.click.mode = ClickMode::Timed;
        }
        "autostart" | "start-on-launch" => {
            cfg.start_clicking_on_launch = matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
        other => anyhow::bail!(
            "unknown setting `{other}` — try cpm, button, mode, duration, hours, minutes or autostart"
        ),
    }
    Ok(())
}

/// Accept `90`, `1h30m`, `1h`, `30m` or `1:30`.
fn parse_duration_minutes(s: &str) -> Result<u32> {
    let s = s.trim().to_ascii_lowercase();

    if let Ok(v) = s.parse::<u32>() {
        anyhow::ensure!(v > 0, "duration must be at least 1 minute");
        return Ok(v);
    }

    if let Some((h, m)) = s.split_once(':') {
        let h: u32 = h.trim().parse().context("bad hours in duration")?;
        let m: u32 = m.trim().parse().context("bad minutes in duration")?;
        let total = h * 60 + m;
        anyhow::ensure!(total > 0, "duration must be at least 1 minute");
        return Ok(total);
    }

    let mut total = 0u32;
    let mut num = String::new();
    let mut saw_unit = false;
    for c in s.chars() {
        match c {
            '0'..='9' => num.push(c),
            'h' => {
                total += num.parse::<u32>().unwrap_or(0) * 60;
                num.clear();
                saw_unit = true;
            }
            'm' => {
                total += num.parse::<u32>().unwrap_or(0);
                num.clear();
                saw_unit = true;
            }
            ' ' => {}
            _ => anyhow::bail!("cannot read `{s}` as a duration — try 90, 1h30m or 1:30"),
        }
    }
    anyhow::ensure!(saw_unit && total > 0, "duration must be at least 1 minute");
    Ok(total)
}

async fn shortcut_cmd(sub: ShortcutCmd) -> Result<()> {
    match sub {
        ShortcutCmd::Show => {
            let cfg = Config::load()?;
            println!("backend    {}", cfg.shortcut.backend.as_str());
            if cfg.shortcut.bindings.is_empty() {
                println!("configured (none)");
            }
            for a in &cfg.shortcut.bindings {
                println!("configured {}", a.to_display());
            }
            let live = shortcut::installed(cfg.shortcut.backend);
            if live.is_empty() {
                println!("installed  (none)");
            }
            for a in &live {
                println!("installed  {}", a.to_display());
            }
            if live != cfg.shortcut.bindings {
                println!(
                    "\n\x1b[33mThe installed shortcut does not match the configuration.\x1b[0m\n\
                     Run `ratclick shortcut apply` to fix it."
                );
            }
            Ok(())
        }
        ShortcutCmd::List { filter } => {
            let by_accel = shortcut::gnome::by_accel();
            let needle = filter.map(|f| f.to_lowercase());
            let mut shown = 0;
            for (accel, owners) in &by_accel {
                let line = format!("{}  {}", accel.to_display(), owners[0].describe());
                if let Some(n) = &needle {
                    if !line.to_lowercase().contains(n) {
                        continue;
                    }
                }
                println!("{:<24} {}", accel.to_display(), owners[0].describe());
                for extra in &owners[1..] {
                    println!("{:<24} {}", "", extra.describe());
                }
                shown += 1;
            }
            for b in shortcut::keyd::scan() {
                let line = b.describe();
                if let Some(n) = &needle {
                    if !line.to_lowercase().contains(n) {
                        continue;
                    }
                }
                println!("{:<24} {}", b.accel.to_display(), line);
                shown += 1;
            }
            if shown == 0 {
                println!("(nothing matched)");
            }
            Ok(())
        }
        ShortcutCmd::Check { accel } => {
            let accel = Accel::parse(&accel)?;
            if let Err(e) = accel.validate() {
                println!("\x1b[31m✗\x1b[0m {e}");
                return Ok(());
            }
            let conflicts = shortcut::conflicts(&accel);
            if conflicts.is_empty() {
                println!("\x1b[32m✓\x1b[0m {} is free", accel.to_display());
            } else {
                println!("\x1b[33m✗\x1b[0m {} is taken by:", accel.to_display());
                for c in conflicts {
                    println!("    • {}", c.describe());
                }
                println!(
                    "\nUse `ratclick shortcut set '{}' --force` to take it.",
                    accel.to_gtk()
                );
            }
            Ok(())
        }
        ShortcutCmd::Set(set) => {
            let mut cfg = Config::load()?;
            if let Some(b) = set.backend {
                cfg.shortcut.backend = b;
            }
            if cfg.shortcut.backend == ShortcutBackend::None {
                cfg.shortcut.backend = ShortcutBackend::Gnome;
            }

            let mut accels = Vec::new();
            for raw in &set.accels {
                let a = Accel::parse(raw)?;
                a.validate()?;

                let conflicts = shortcut::conflicts(&a);
                if !conflicts.is_empty() {
                    if !set.force {
                        eprintln!("{} is taken by:", a.to_display());
                        for c in &conflicts {
                            eprintln!("    • {}", c.describe());
                        }
                        anyhow::bail!("pass --force to take it anyway");
                    }
                    let (taken, refused) = shortcut::force_take(&a)?;
                    for t in taken {
                        println!("unbound {t}");
                    }
                    for r in refused {
                        println!("\x1b[33mstill bound:\x1b[0m {r}");
                    }
                }
                accels.push(a);
            }

            cfg.shortcut.bindings = accels;
            cfg.setup_complete = true;
            cfg.save()?;
            setup::apply_and_report(&cfg)?;
            client::nudge_reload().await;
            Ok(())
        }
        ShortcutCmd::Clear => {
            let mut cfg = Config::load()?;
            shortcut::remove_all()?;
            cfg.shortcut.bindings.clear();
            cfg.save()?;
            println!("shortcut removed");
            Ok(())
        }
        ShortcutCmd::Apply => {
            let cfg = Config::load()?;
            setup::apply_and_report(&cfg)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_parse_in_every_spelling() {
        assert_eq!(parse_duration_minutes("90").unwrap(), 90);
        assert_eq!(parse_duration_minutes("1h30m").unwrap(), 90);
        assert_eq!(parse_duration_minutes("1:30").unwrap(), 90);
        assert_eq!(parse_duration_minutes("2h").unwrap(), 120);
        assert_eq!(parse_duration_minutes("45m").unwrap(), 45);
        assert_eq!(parse_duration_minutes("1h 30m").unwrap(), 90);
    }

    #[test]
    fn zero_and_nonsense_durations_are_rejected() {
        assert!(parse_duration_minutes("0").is_err());
        assert!(parse_duration_minutes("0h0m").is_err());
        assert!(parse_duration_minutes("soon").is_err());
        assert!(parse_duration_minutes("").is_err());
    }

    #[test]
    fn setting_duration_switches_to_timed_mode() {
        let mut cfg = Config::default();
        assert_eq!(cfg.click.mode, ClickMode::Endless);
        apply_setting(&mut cfg, "duration", "1h30m").unwrap();
        assert_eq!(cfg.click.mode, ClickMode::Timed);
        assert_eq!(cfg.click.duration_minutes, 90);
    }

    #[test]
    fn cpm_is_range_checked() {
        let mut cfg = Config::default();
        assert!(apply_setting(&mut cfg, "cpm", "0").is_err());
        assert!(apply_setting(&mut cfg, "cpm", "999999").is_err());
        apply_setting(&mut cfg, "cpm", "1200").unwrap();
        assert_eq!(cfg.click.cpm, 1200);
    }

    #[test]
    fn unknown_settings_are_named_in_the_error() {
        let mut cfg = Config::default();
        let e = apply_setting(&mut cfg, "colour", "red").unwrap_err();
        assert!(e.to_string().contains("colour"));
    }

    #[test]
    fn hours_and_minutes_compose() {
        let mut cfg = Config::default();
        apply_setting(&mut cfg, "hours", "2").unwrap();
        apply_setting(&mut cfg, "minutes", "15").unwrap();
        assert_eq!(cfg.click.duration_minutes, 135);
    }

    #[test]
    fn cli_parses_the_documented_invocations() {
        use clap::Parser;
        for args in [
            vec!["ratclick", "gui"],
            vec!["ratclick", "toggle"],
            vec!["ratclick", "status", "--porcelain"],
            vec!["ratclick", "daemon", "restart"],
            vec!["ratclick", "config", "set", "cpm", "900"],
            vec!["ratclick", "shortcut", "set", "<Super>F9", "--force"],
            vec![
                "ratclick",
                "shortcut",
                "set",
                "<Super>F9",
                "--backend",
                "keyd",
            ],
            vec!["ratclick", "shortcut", "check", "<Super>c"],
        ] {
            Cli::try_parse_from(&args).unwrap_or_else(|e| panic!("{args:?} failed: {e}"));
        }
    }
}
