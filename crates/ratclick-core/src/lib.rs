//! Shared foundations for the RatClick auto-clicker: configuration, keyboard
//! accelerators, the session-bus contract, and the global-shortcut backends.
//!
//! The CLI, the GUI and the daemon all sit on top of this crate so that there
//! is exactly one definition of what a shortcut is, where the config lives, and
//! what the D-Bus names are.

pub mod accel;
pub mod config;
pub mod daemon;
pub mod ipc;
pub mod shortcut;

pub use accel::{Accel, AccelError, Modifiers};
pub use config::{Button, ClickMode, Config, Effect, ShortcutBackend};
pub use ipc::Status;

/// Version of the RatClick suite, taken from the workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Undo everything RatClick has done to this account.
///
/// Removes the shortcut from every backend and deletes the configuration, so
/// the next launch behaves like a fresh install. It deliberately does *not*
/// uninstall the package, stop the daemon, or touch anything outside the user's
/// own settings — callers that want the daemon stopped should do that first,
/// since only they can speak to it.
///
/// Returns a description of each thing that was removed, and keeps going after
/// a failure so one stuck backend cannot block the rest.
pub fn reset_all() -> (Vec<String>, Vec<String>) {
    let mut done = Vec::new();
    let mut failed = Vec::new();

    for backend in [
        config::ShortcutBackend::Gnome,
        config::ShortcutBackend::Extension,
        config::ShortcutBackend::Keyd,
    ] {
        if shortcut::installed(backend).is_empty() {
            continue;
        }
        let result = match backend {
            config::ShortcutBackend::Gnome => shortcut::gnome::uninstall(),
            config::ShortcutBackend::Extension => shortcut::extension::uninstall(),
            config::ShortcutBackend::Keyd => shortcut::keyd::uninstall().map(|_| ()),
            config::ShortcutBackend::None => Ok(()),
        };
        match result {
            Ok(()) => done.push(format!("removed the {} shortcut", backend.as_str())),
            Err(e) => failed.push(format!(
                "could not remove the {} shortcut: {e}",
                backend.as_str()
            )),
        }
    }

    match config::Config::path() {
        Ok(path) => match std::fs::remove_file(&path) {
            Ok(()) => done.push(format!("deleted {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => failed.push(format!("could not delete {}: {e}", path.display())),
        },
        Err(e) => failed.push(format!("could not locate the configuration: {e}")),
    }

    (done, failed)
}
