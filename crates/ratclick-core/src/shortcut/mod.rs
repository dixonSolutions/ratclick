//! Global-shortcut installation and conflict detection.
//!
//! Three backends can own RatClick's toggle key, and they are deliberately
//! mutually exclusive — installing one uninstalls the others, so a shortcut can
//! never end up bound twice and toggling twice per press.

pub mod extension;
pub mod gnome;
pub mod gvariant;
pub mod keyd;

use anyhow::Result;

use crate::accel::Accel;
use crate::config::{Config, ShortcutBackend};

/// Which `gsettings` to run.
///
/// The system one is preferred over whatever `PATH` offers first. A `gsettings`
/// belonging to a second GLib installation — Homebrew, conda, a Nix profile —
/// usually cannot load the dconf GIO module, so it falls back to the in-memory
/// backend and *silently* discards every write. The shortcut then looks
/// installed and does nothing. Reaching for the distro binary avoids the whole
/// class of problem; falling back to `PATH` keeps this working on distributions
/// that put it elsewhere.
pub(crate) fn gsettings_bin() -> &'static str {
    const SYSTEM: &str = "/usr/bin/gsettings";
    if std::path::Path::new(SYSTEM).is_file() {
        SYSTEM
    } else {
        "gsettings"
    }
}

/// Command a GNOME custom keybinding runs.
pub const TOGGLE_COMMAND: &str = "ratclick toggle";
/// Command keyd runs. keyd executes as root, so it goes through the dispatcher
/// that re-enters the logged-in user's session bus.
pub const KEYD_DISPATCH: &str = "/usr/libexec/ratclick/ratclick-keyd-dispatch toggle";

/// Something else already owns an accelerator we want.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    Gnome(gnome::Binding),
    Keyd(keyd::Binding),
}

impl Conflict {
    pub fn describe(&self) -> String {
        match self {
            Conflict::Gnome(b) => b.owner.describe(),
            Conflict::Keyd(b) => b.describe(),
        }
    }

    pub fn accel(&self) -> &Accel {
        match self {
            Conflict::Gnome(b) => &b.accel,
            Conflict::Keyd(b) => &b.accel,
        }
    }
}

/// Whether a backend can be used right now, and why not if it cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendStatus {
    pub backend: ShortcutBackend,
    pub available: bool,
    pub detail: String,
}

/// Probe every backend so the wizard and the GUI can grey out what won't work.
pub fn backend_statuses() -> Vec<BackendStatus> {
    let gnome_ok = gnome::is_available();
    let ext = extension::availability();
    let keyd_av = keyd::availability();

    vec![
        BackendStatus {
            backend: ShortcutBackend::Gnome,
            available: gnome_ok,
            detail: if gnome_ok {
                "GNOME custom keybinding — no privileges needed".into()
            } else {
                "requires an active GNOME session with the media-keys schema".into()
            },
        },
        BackendStatus {
            backend: ShortcutBackend::Extension,
            available: ext.is_available,
            detail: ext.detail.clone(),
        },
        BackendStatus {
            backend: ShortcutBackend::Keyd,
            available: keyd_av.is_ready(),
            detail: keyd_av.explain(),
        },
        BackendStatus {
            backend: ShortcutBackend::None,
            available: true,
            detail: "no global shortcut; toggle from the app, CLI or panel".into(),
        },
    ]
}

/// Everything already bound to `accel`, across every backend we can see.
///
/// Scanning all backends regardless of the configured one is deliberate: a keyd
/// binding will fire even when RatClick is using the GNOME backend, so the user
/// needs to know about it.
pub fn conflicts(accel: &Accel) -> Vec<Conflict> {
    let mut out: Vec<Conflict> = Vec::new();
    if gnome::is_available() {
        out.extend(gnome::conflicts_for(accel).into_iter().map(Conflict::Gnome));
    }
    out.extend(keyd::conflicts_for(accel).into_iter().map(Conflict::Keyd));
    out
}

/// Is `accel` free to take?
pub fn is_available(accel: &Accel) -> bool {
    conflicts(accel).is_empty()
}

/// Take `accel` away from whatever holds it.
///
/// keyd conflicts are reported but not rewritten — editing somebody else's keyd
/// mapping could break their keyboard in a way they cannot easily undo, and
/// unlike gsettings there is no Settings UI to fix it from. Returns the list of
/// things that were changed, then the list that could not be.
pub fn force_take(accel: &Accel) -> Result<(Vec<String>, Vec<String>)> {
    let mut taken = Vec::new();
    let mut refused = Vec::new();

    if gnome::is_available() {
        taken.extend(gnome::steal(accel)?);
    }
    for b in keyd::conflicts_for(accel) {
        refused.push(b.describe());
    }

    Ok((taken, refused))
}

/// What was done by [`apply`], for reporting back to the user.
#[derive(Debug, Default, Clone)]
pub struct ApplyReport {
    pub backend: ShortcutBackend,
    pub installed: Vec<Accel>,
    /// Backends we removed a stale binding from.
    pub cleared: Vec<ShortcutBackend>,
    pub notes: Vec<String>,
}

/// Install the configured shortcuts, removing any left behind by other backends.
///
/// `keyd` needs root; when the process is unprivileged this returns an error
/// naming the elevation the caller should perform, rather than half-applying.
pub fn apply(cfg: &Config) -> Result<ApplyReport> {
    let mut report = ApplyReport {
        backend: cfg.shortcut.backend,
        ..Default::default()
    };
    let accels = &cfg.shortcut.bindings;

    // Clear the two backends we are not using, so a backend switch does not
    // leave the old binding live.
    for other in [
        ShortcutBackend::Gnome,
        ShortcutBackend::Extension,
        ShortcutBackend::Keyd,
    ] {
        if other == cfg.shortcut.backend {
            continue;
        }
        match other {
            ShortcutBackend::Gnome if gnome::is_available() && !gnome::installed().is_empty() => {
                gnome::uninstall()?;
                report.cleared.push(other);
            }
            ShortcutBackend::Extension if !extension::installed().is_empty() => {
                extension::uninstall()?;
                report.cleared.push(other);
            }
            ShortcutBackend::Keyd if !keyd::installed().is_empty() => {
                // Only attempt this when we can; otherwise say so and move on.
                match keyd::uninstall() {
                    Ok(files) if !files.is_empty() => report.cleared.push(other),
                    Ok(_) => {}
                    Err(e) => report
                        .notes
                        .push(format!("left the old keyd binding in place: {e}")),
                }
            }
            _ => {}
        }
    }

    match cfg.shortcut.backend {
        ShortcutBackend::Gnome => {
            anyhow::ensure!(
                gnome::is_available(),
                "the GNOME backend needs gsettings and the GNOME settings schemas"
            );
            gnome::install(accels, TOGGLE_COMMAND)?;
        }
        ShortcutBackend::Extension => {
            extension::install(accels)?;
        }
        ShortcutBackend::Keyd => {
            let av = keyd::availability();
            anyhow::ensure!(av.is_ready(), "{}", av.explain());
            let files = keyd::install(accels, KEYD_DISPATCH)?;
            if !files.is_empty() {
                report.notes.push(format!(
                    "updated {}",
                    files
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        ShortcutBackend::None => {}
    }

    report.installed = accels.clone();
    Ok(report)
}

/// Remove RatClick's shortcuts from every backend.
pub fn remove_all() -> Result<()> {
    if gnome::is_available() {
        gnome::uninstall()?;
    }
    extension::uninstall()?;
    if !keyd::installed().is_empty() {
        keyd::uninstall()?;
    }
    Ok(())
}

/// Read back what is actually installed, per backend — used to verify that an
/// apply really landed rather than trusting the config file.
pub fn installed(backend: ShortcutBackend) -> Vec<Accel> {
    match backend {
        ShortcutBackend::Gnome => gnome::installed(),
        ShortcutBackend::Extension => extension::installed(),
        ShortcutBackend::Keyd => keyd::installed(),
        ShortcutBackend::None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_backend_is_probed() {
        let statuses = backend_statuses();
        assert_eq!(statuses.len(), 4);
        // `None` is always usable, whatever the machine looks like.
        let none = statuses
            .iter()
            .find(|s| s.backend == ShortcutBackend::None)
            .unwrap();
        assert!(none.available);
    }

    #[test]
    fn conflicts_describe_themselves() {
        let c = Conflict::Gnome(gnome::Binding {
            accel: Accel::parse("<Super>c").unwrap(),
            owner: gnome::Owner::Setting {
                schema: "org.gnome.desktop.wm.keybindings".into(),
                key: "close".into(),
            },
        });
        assert_eq!(c.accel(), &Accel::parse("<Super>c").unwrap());
        assert!(c.describe().contains("Window management"));
    }
}
