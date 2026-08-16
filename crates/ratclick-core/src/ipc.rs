//! The session-bus contract between `ratclickd`, the CLI, the GUI and the
//! GNOME Shell extension.
//!
//! These constants are the single source of truth for the names; the extension
//! hard-codes the same strings in `extension/dbus.js` and the two must agree.

/// Well-known bus name owned by the daemon.
///
/// Deliberately *not* `io.github.dixonsolutions.RatClick`: that name belongs to
/// the GUI. `GtkApplication` claims its application ID on the session bus to
/// implement single-instance behaviour, and exports `org.gtk.Application` at
/// the path derived from it. If the daemon used the same name the two would
/// race for it — whichever started second would fail, and calls meant for the
/// daemon would land on the GUI and come back as `UnknownMethod`.
pub const BUS_NAME: &str = "io.github.dixonsolutions.RatClick.Daemon";
/// Object path the daemon exports.
pub const OBJECT_PATH: &str = "/io/github/dixonsolutions/RatClick/Daemon";
/// Interface carrying the methods and signals below.
pub const INTERFACE: &str = "io.github.dixonsolutions.RatClick1";

/// The GUI's `GtkApplication` ID, which is also the desktop-entry and icon
/// name. Kept here so the clash above stays visible in one place.
pub const APP_ID: &str = "io.github.dixonsolutions.RatClick";

/// Daemon state as reported by `Status` and the `StateChanged` signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Whether the click loop is currently running.
    pub running: bool,
    pub cpm: u32,
    pub button: String,
    /// `"endless"` or `"timed"`.
    pub mode: String,
    /// Seconds left in a timed run; `0` when endless or stopped.
    pub remaining_seconds: u32,
    /// Clicks emitted during the current run.
    pub clicks: u64,
}

impl Status {
    /// Compact one-line rendering shared by `ratclick status` and the panel
    /// indicator's tooltip.
    pub fn summary(&self) -> String {
        if !self.running {
            return format!("stopped ({} CPM, {})", self.cpm, self.mode);
        }
        if self.mode == "timed" {
            format!(
                "clicking {} at {} CPM — {} left",
                self.button,
                self.cpm,
                format_remaining(self.remaining_seconds)
            )
        } else {
            format!("clicking {} at {} CPM — endless", self.button, self.cpm)
        }
    }
}

/// Render a countdown the way both the CLI and the GUI want it.
pub fn format_remaining(seconds: u32) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daemon_name_does_not_collide_with_the_gui() {
        // GtkApplication owns APP_ID on the session bus and exports
        // org.gtk.Application at the path derived from it. If the daemon used
        // the same name, whichever process started second would fail to claim
        // it and every call meant for the daemon would hit the GUI instead,
        // coming back as `org.freedesktop.DBus.Error.UnknownMethod`.
        assert_ne!(BUS_NAME, APP_ID);

        // GtkApplication's path is the app ID with dots turned into slashes.
        let gtk_path = format!("/{}", APP_ID.replace('.', "/"));
        assert_ne!(OBJECT_PATH, gtk_path);
    }

    #[test]
    fn remaining_scales_with_magnitude() {
        assert_eq!(format_remaining(45), "45s");
        assert_eq!(format_remaining(125), "2m 05s");
        assert_eq!(format_remaining(3725), "1h 02m 05s");
    }

    #[test]
    fn summary_reflects_state() {
        let mut s = Status {
            running: false,
            cpm: 600,
            button: "left".into(),
            mode: "endless".into(),
            remaining_seconds: 0,
            clicks: 0,
        };
        assert!(s.summary().starts_with("stopped"));
        s.running = true;
        assert!(s.summary().contains("endless"));
        s.mode = "timed".into();
        s.remaining_seconds = 90;
        assert!(s.summary().contains("1m 30s"));
    }
}
