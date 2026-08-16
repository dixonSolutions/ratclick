//! Background bridge between the GTK main loop and the session bus.
//!
//! GTK wants a glib main loop and zbus wants an async executor, so rather than
//! trying to marry the two this runs zbus's blocking API on its own thread.
//! Commands go in over a `std` channel (sending never blocks the UI) and
//! snapshots come back over an async channel the GUI awaits with
//! `glib::spawn_future_local`.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use ratclick_core::daemon;

/// How often the daemon is polled.
///
/// Fast enough that the countdown ticks smoothly and that a toggle from the
/// global shortcut shows up almost immediately, slow enough to be free.
const POLL: Duration = Duration::from_millis(400);

#[zbus::proxy(
    interface = "io.github.dixonsolutions.RatClick1",
    default_service = "io.github.dixonsolutions.RatClick.Daemon",
    default_path = "/io/github/dixonsolutions/RatClick/Daemon"
)]
pub trait RatClick {
    fn start(&self) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;
    fn toggle(&self) -> zbus::Result<bool>;
    fn status(&self) -> zbus::Result<(bool, u32, String, String, u32, u64)>;
    fn reload_config(&self) -> zbus::Result<()>;
    fn quit(&self) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    /// Start clicking if stopped, stop it if running.
    Toggle,
    /// Tell the daemon to re-read config.toml.
    Reload,
    /// Launch the daemon.
    StartDaemon,
    /// Shut the daemon down.
    StopDaemon,
    /// Poll now instead of waiting for the next tick.
    Refresh,
}

/// Everything the UI needs to render itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub daemon_up: bool,
    pub running: bool,
    pub cpm: u32,
    pub button: String,
    pub mode: String,
    pub remaining_seconds: u32,
    pub clicks: u64,
    /// Set when the last operation failed, for a toast.
    pub error: Option<String>,
}

pub struct Bridge {
    tx: mpsc::Sender<Cmd>,
    pub events: async_channel::Receiver<Snapshot>,
}

impl Bridge {
    pub fn start() -> Bridge {
        let (tx, rx) = mpsc::channel::<Cmd>();
        // A small bounded queue: if the UI falls behind, the newest snapshot is
        // the only one that matters, so old ones are dropped rather than queued.
        let (event_tx, events) = async_channel::bounded::<Snapshot>(4);

        thread::Builder::new()
            .name("ratclick-bus".into())
            .spawn(move || worker(rx, event_tx))
            .expect("spawning the bus thread");

        Bridge { tx, events }
    }

    /// Queue a command. Never blocks; a dead worker is silently ignored because
    /// the UI will already be showing the disconnected state.
    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

fn worker(rx: mpsc::Receiver<Cmd>, tx: async_channel::Sender<Snapshot>) {
    let mut last: Option<Snapshot> = None;

    loop {
        let cmd = match rx.recv_timeout(POLL) {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };

        let mut error = None;
        if let Some(cmd) = cmd {
            if let Err(e) = handle(cmd) {
                error = Some(e);
            }
        }

        let mut snap = poll_status();
        snap.error = error;

        // Only wake the UI when something actually changed, so an idle window
        // does no work at all.
        if last.as_ref() != Some(&snap) || snap.error.is_some() {
            last = Some(snap.clone());
            // force_send keeps the newest value even when the queue is full.
            if tx.force_send(snap).is_err() {
                return;
            }
        }
    }
}

fn proxy() -> Option<RatClickProxyBlocking<'static>> {
    let conn = zbus::blocking::Connection::session().ok()?;
    RatClickProxyBlocking::new(&conn).ok()
}

fn name_has_owner() -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else {
        return false;
    };
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else {
        return false;
    };
    ratclick_core::ipc::BUS_NAME
        .try_into()
        .ok()
        .and_then(|n| dbus.name_has_owner(n).ok())
        .unwrap_or(false)
}

fn poll_status() -> Snapshot {
    if !name_has_owner() {
        return Snapshot::default();
    }
    let Some(p) = proxy() else {
        return Snapshot::default();
    };
    match p.status() {
        Ok((running, cpm, button, mode, remaining_seconds, clicks)) => Snapshot {
            daemon_up: true,
            running,
            cpm,
            button,
            mode,
            remaining_seconds,
            clicks,
            error: None,
        },
        Err(_) => Snapshot::default(),
    }
}

fn handle(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Refresh => Ok(()),
        Cmd::StartDaemon => start_daemon(),
        Cmd::StopDaemon => {
            if !daemon::stop_via_systemd() {
                if let Some(p) = proxy() {
                    let _ = p.quit();
                }
            }
            wait_for(false)
        }
        Cmd::Reload => {
            if !name_has_owner() {
                // Nothing to tell; the daemon reads the file when it starts.
                return Ok(());
            }
            proxy()
                .ok_or_else(|| "cannot reach the service".to_string())?
                .reload_config()
                .map_err(|e| e.to_string())
        }
        Cmd::Toggle => {
            // Starting the daemon on demand is what makes the big button work
            // straight after install without a separate "start service" step.
            if !name_has_owner() {
                start_daemon()?;
            }
            proxy()
                .ok_or_else(|| "cannot reach the service".to_string())?
                .toggle()
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }
}

/// Start the daemon and wait for it to reach *this* session bus.
///
/// systemd keeps its own `DBUS_SESSION_BUS_ADDRESS`, so a unit start can put
/// the daemon on a different bus than the one we are talking to. If the name
/// does not turn up, run the binary ourselves so it inherits our environment.
fn start_daemon() -> Result<(), String> {
    let how = daemon::spawn().map_err(|e| format!("{e:#}"))?;
    if wait_for(true).is_ok() {
        return Ok(());
    }
    if how == daemon::Launched::Systemd {
        daemon::spawn_direct().map_err(|e| format!("{e:#}"))?;
        return wait_for(true);
    }
    Err("the RatClick service did not start — run `ratclick doctor` in a terminal".into())
}

/// Block until the daemon's presence on the bus matches `want`.
fn wait_for(want: bool) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if name_has_owner() == want {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(80));
    }
    Err(if want {
        "the RatClick service did not start — run `ratclick doctor` in a terminal".into()
    } else {
        "the RatClick service did not stop".into()
    })
}
