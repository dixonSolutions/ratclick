//! The click engine: a virtual mouse plus the loop that presses it.
//!
//! Everything here runs on one dedicated OS thread. `evdev` writes are blocking
//! and the timing needs to be steady, so keeping the loop off the async runtime
//! avoids both blocking the executor and being descheduled by it.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode};

use ratclick_core::config::{Button, ClickConfig, ClickMode};

/// Name the virtual device advertises. Visible in `libinput list-devices`, and
/// deliberately obvious so nobody wonders what the extra mouse is.
const DEVICE_NAME: &str = "RatClick Virtual Pointer";

/// How long to wait after creating the uinput node before the first click.
///
/// udev has to see the device and libinput has to add it to the compositor's
/// seat; clicking into that window produces events nothing is listening for.
const SETTLE: Duration = Duration::from_millis(400);

#[derive(Debug)]
enum Cmd {
    Start(ClickConfig),
    Stop,
    Shutdown,
}

/// Snapshot of what the engine is doing, cheap enough to read on every D-Bus
/// call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EngineState {
    pub running: bool,
    pub clicks: u64,
    /// `None` for an endless run.
    pub remaining: Option<Duration>,
}

impl EngineState {
    pub fn remaining_seconds(&self) -> u32 {
        self.remaining
            .map(|d| d.as_secs().min(u32::MAX as u64) as u32)
            .unwrap_or(0)
    }
}

/// Handle to the click thread.
pub struct Engine {
    tx: Sender<Cmd>,
    state: Arc<Mutex<EngineState>>,
    /// Fired whenever `state.running` changes, so the D-Bus layer can emit
    /// `StateChanged` without polling.
    notify: tokio::sync::watch::Sender<EngineState>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Engine {
    /// Create the virtual device and start the click thread.
    ///
    /// Fails with an actionable message if `/dev/uinput` is not writable, which
    /// is the single most common installation problem.
    pub fn start() -> Result<Engine> {
        let device = open_device().context(
            "could not create the virtual mouse — check that /dev/uinput exists and that you \
             are in the `input` group (log out and back in after being added)",
        )?;

        let state = Arc::new(Mutex::new(EngineState::default()));
        let (notify, _) = tokio::sync::watch::channel(EngineState::default());
        let (tx, rx) = mpsc::channel();

        let handle = {
            let state = Arc::clone(&state);
            let notify = notify.clone();
            thread::Builder::new()
                .name("ratclick-engine".into())
                .spawn(move || run_loop(device, rx, state, notify))
                .context("spawning the click thread")?
        };

        Ok(Engine {
            tx,
            state,
            notify,
            handle: Some(handle),
        })
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<EngineState> {
        self.notify.subscribe()
    }

    pub fn state(&self) -> EngineState {
        self.state.lock().expect("engine state poisoned").clone()
    }

    pub fn is_running(&self) -> bool {
        self.state().running
    }

    pub fn start_clicking(&self, cfg: &ClickConfig) -> Result<()> {
        self.tx
            .send(Cmd::Start(cfg.clone()))
            .context("click thread is gone")?;
        Ok(())
    }

    pub fn stop_clicking(&self) -> Result<()> {
        self.tx.send(Cmd::Stop).context("click thread is gone")?;
        Ok(())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn open_device() -> Result<VirtualDevice> {
    // The relative axes matter even though we never move the pointer: libinput
    // only classifies a device as a pointer if it has them, and a device that
    // is not a pointer has its button events ignored by the compositor.
    let axes = AttributeSet::from_iter([RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y]);
    let keys =
        AttributeSet::from_iter([KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT, KeyCode::BTN_MIDDLE]);

    let device = VirtualDevice::builder()?
        .name(DEVICE_NAME)
        .with_relative_axes(&axes)?
        .with_keys(&keys)?
        .build()?;

    Ok(device)
}

fn key_for(button: Button) -> KeyCode {
    match button {
        Button::Left => KeyCode::BTN_LEFT,
        Button::Right => KeyCode::BTN_RIGHT,
        Button::Middle => KeyCode::BTN_MIDDLE,
    }
}

/// Emit one complete press/release pair.
///
/// Press and release go in separate `emit` calls (each of which appends its own
/// `SYN_REPORT`) because a press and release inside a single report frame is a
/// zero-length click that most toolkits discard.
fn click(device: &mut VirtualDevice, key: KeyCode) -> std::io::Result<()> {
    device.emit(&[InputEvent::new(EventType::KEY.0, key.0, 1)])?;
    device.emit(&[InputEvent::new(EventType::KEY.0, key.0, 0)])?;
    Ok(())
}

struct Run {
    interval: Duration,
    key: KeyCode,
    deadline: Option<Instant>,
    next: Instant,
}

fn run_loop(
    mut device: VirtualDevice,
    rx: Receiver<Cmd>,
    state: Arc<Mutex<EngineState>>,
    notify: tokio::sync::watch::Sender<EngineState>,
) {
    let created = Instant::now();
    let mut run: Option<Run> = None;

    let publish = |state: &Arc<Mutex<EngineState>>, new: EngineState| {
        *state.lock().expect("engine state poisoned") = new.clone();
        // A send failure only means nobody is listening yet.
        let _ = notify.send(new);
    };

    loop {
        // Idle: block until something happens.
        let Some(active) = run.as_mut() else {
            match rx.recv() {
                Ok(Cmd::Start(cfg)) => {
                    // Honour the settle delay for the very first run after the
                    // device was created.
                    let settle = SETTLE.saturating_sub(created.elapsed());
                    if !settle.is_zero() {
                        thread::sleep(settle);
                    }
                    let now = Instant::now();
                    run = Some(Run {
                        interval: cfg.interval(),
                        key: key_for(cfg.button),
                        deadline: cfg.duration().map(|d| now + d),
                        next: now,
                    });
                    let remaining = match cfg.mode {
                        ClickMode::Endless => None,
                        ClickMode::Timed => cfg.duration(),
                    };
                    publish(
                        &state,
                        EngineState {
                            running: true,
                            clicks: 0,
                            remaining,
                        },
                    );
                    tracing::info!(cpm = cfg.cpm, mode = cfg.mode.as_str(), "clicking started");
                }
                Ok(Cmd::Stop) => {}
                Ok(Cmd::Shutdown) | Err(_) => return,
            }
            continue;
        };

        // Timed run finished.
        if let Some(deadline) = active.deadline {
            if Instant::now() >= deadline {
                run = None;
                let clicks = state.lock().expect("engine state poisoned").clicks;
                publish(
                    &state,
                    EngineState {
                        running: false,
                        clicks,
                        remaining: None,
                    },
                );
                tracing::info!(clicks, "timed run finished");
                continue;
            }
        }

        // Sleep until the next click, but stay responsive to commands.
        let now = Instant::now();
        let mut wait = active.next.saturating_duration_since(now);
        if let Some(deadline) = active.deadline {
            wait = wait.min(deadline.saturating_duration_since(now));
        }

        match rx.recv_timeout(wait) {
            Ok(Cmd::Stop) => {
                run = None;
                let clicks = state.lock().expect("engine state poisoned").clicks;
                publish(
                    &state,
                    EngineState {
                        running: false,
                        clicks,
                        remaining: None,
                    },
                );
                tracing::info!(clicks, "clicking stopped");
                continue;
            }
            Ok(Cmd::Start(cfg)) => {
                // Restart with new settings without dropping out of the run.
                let now = Instant::now();
                *active = Run {
                    interval: cfg.interval(),
                    key: key_for(cfg.button),
                    deadline: cfg.duration().map(|d| now + d),
                    next: now,
                };
                continue;
            }
            Ok(Cmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }

        if Instant::now() < active.next {
            continue;
        }

        if let Err(e) = click(&mut device, active.key) {
            tracing::error!("click failed: {e}; stopping");
            run = None;
            publish(
                &state,
                EngineState {
                    running: false,
                    clicks: state.lock().expect("engine state poisoned").clicks,
                    remaining: None,
                },
            );
            continue;
        }

        // Advance on the schedule rather than from "now", so a slow click does
        // not make the whole run drift late. If we have fallen far behind
        // (suspend, heavy load) resync instead of trying to catch up with a
        // burst of clicks.
        active.next += active.interval;
        let now = Instant::now();
        if active.next + active.interval < now {
            active.next = now + active.interval;
        }

        let mut guard = state.lock().expect("engine state poisoned");
        guard.clicks += 1;
        guard.remaining = active.deadline.map(|d| d.saturating_duration_since(now));
        let snapshot = guard.clone();
        drop(guard);
        // Only the countdown changed, so this is a cheap update for anyone
        // rendering a timer; running-state transitions are published above.
        notify.send_if_modified(|cur| {
            let changed =
                cur.remaining.map(|d| d.as_secs()) != snapshot.remaining.map(|d| d.as_secs());
            *cur = snapshot;
            changed
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_seconds_rounds_down() {
        let s = EngineState {
            running: true,
            clicks: 0,
            remaining: Some(Duration::from_millis(1900)),
        };
        assert_eq!(s.remaining_seconds(), 1);
    }

    #[test]
    fn endless_runs_report_no_countdown() {
        let s = EngineState {
            running: true,
            clicks: 5,
            remaining: None,
        };
        assert_eq!(s.remaining_seconds(), 0);
    }

    #[test]
    fn buttons_map_to_the_right_evdev_codes() {
        assert_eq!(key_for(Button::Left), KeyCode::BTN_LEFT);
        assert_eq!(key_for(Button::Right), KeyCode::BTN_RIGHT);
        assert_eq!(key_for(Button::Middle), KeyCode::BTN_MIDDLE);
    }
}
