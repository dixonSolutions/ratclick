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
pub use config::{Button, ClickMode, Config, ShortcutBackend};
pub use ipc::Status;

/// Version of the RatClick suite, taken from the workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
