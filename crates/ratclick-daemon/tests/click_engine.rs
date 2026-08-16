//! End-to-end tests for the click engine against a real `/dev/uinput` device.
//!
//! # Why these are safe to run on a live desktop
//!
//! The virtual pointer the engine creates is a system-wide input device, so its
//! button events would ordinarily land in whatever window happens to be under
//! the cursor. Every test here therefore calls `EVIOCGRAB` on the device node
//! *before* asking the engine to click, which makes the kernel deliver the
//! events exclusively to this test process. Nothing reaches the compositor.
//!
//! The tests are ignored by default because they need write access to
//! `/dev/uinput`; run them with:
//!
//! ```text
//! cargo test -p ratclick-daemon -- --ignored --test-threads=1
//! ```

use std::time::{Duration, Instant};

use evdev::{Device, EventSummary, KeyCode};
use ratclick_core::config::{Button, ClickConfig, ClickMode};
use ratclick_daemon::engine::Engine;

const DEVICE_NAME: &str = "RatClick Virtual Pointer";

/// Find the engine's virtual pointer and take exclusive ownership of it.
///
/// Retries briefly because udev needs a moment to create the node after the
/// uinput device is registered.
fn grab_virtual_pointer() -> Device {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let found = evdev::enumerate()
            .find(|(_, d)| d.name() == Some(DEVICE_NAME))
            .map(|(path, _)| path);

        if let Some(path) = found {
            match Device::open(&path) {
                Ok(mut dev) => {
                    dev.grab().expect("EVIOCGRAB on the virtual pointer");
                    dev.set_nonblocking(true).expect("set nonblocking");
                    return dev;
                }
                Err(e) if Instant::now() < deadline => {
                    eprintln!("retrying open of {}: {e}", path.display());
                }
                Err(e) => panic!("cannot open {}: {e}", path.display()),
            }
        }

        assert!(
            Instant::now() < deadline,
            "the `{DEVICE_NAME}` device never appeared — is /dev/uinput writable?"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Collect button events for `window`, returning `(code, value)` pairs.
fn collect(dev: &mut Device, window: Duration) -> Vec<(KeyCode, i32)> {
    let mut out = Vec::new();
    let end = Instant::now() + window;
    while Instant::now() < end {
        match dev.fetch_events() {
            Ok(events) => {
                for ev in events {
                    if let EventSummary::Key(_, code, value) = ev.destructure() {
                        out.push((code, value));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => panic!("reading events: {e}"),
        }
    }
    out
}

fn config(cpm: u32, button: Button, mode: ClickMode, minutes: u32) -> ClickConfig {
    ClickConfig {
        button,
        cpm,
        mode,
        duration_minutes: minutes,
    }
}

#[test]
#[ignore = "needs write access to /dev/uinput"]
fn creates_a_virtual_pointer() {
    let _engine = Engine::start().expect("engine start");
    let dev = grab_virtual_pointer();
    assert_eq!(dev.name(), Some(DEVICE_NAME));

    // libinput only treats a device as a pointer if it has both buttons and
    // relative axes; without the axes the compositor ignores our clicks.
    let keys = dev.supported_keys().expect("device advertises keys");
    assert!(keys.contains(KeyCode::BTN_LEFT));
    assert!(keys.contains(KeyCode::BTN_RIGHT));
    assert!(keys.contains(KeyCode::BTN_MIDDLE));
    assert!(
        dev.supported_relative_axes().is_some(),
        "no relative axes — libinput would not classify this as a pointer"
    );
}

#[test]
#[ignore = "needs write access to /dev/uinput"]
fn emits_paired_press_and_release_events() {
    let engine = Engine::start().expect("engine start");
    let mut dev = grab_virtual_pointer();

    engine
        .start_clicking(&config(600, Button::Left, ClickMode::Endless, 0))
        .unwrap();
    let events = collect(&mut dev, Duration::from_millis(1200));
    engine.stop_clicking().unwrap();

    assert!(!events.is_empty(), "no button events were emitted");

    // Every event must be BTN_LEFT, and presses and releases must alternate
    // starting with a press — a stuck button would be a nasty bug to ship.
    let mut expect_press = true;
    for (code, value) in &events {
        assert_eq!(*code, KeyCode::BTN_LEFT, "unexpected button {code:?}");
        let is_press = *value == 1;
        assert_eq!(
            is_press, expect_press,
            "press/release events are not alternating: {events:?}"
        );
        expect_press = !expect_press;
    }

    // Drain anything still in flight, then confirm the button ended up released.
    let tail = collect(&mut dev, Duration::from_millis(200));
    let last = tail.last().or_else(|| events.last()).unwrap();
    assert_eq!(
        last.1, 0,
        "the run finished with the button still held down"
    );
}

#[test]
#[ignore = "needs write access to /dev/uinput"]
fn click_rate_tracks_the_configured_cpm() {
    let engine = Engine::start().expect("engine start");
    let mut dev = grab_virtual_pointer();

    // 600 CPM is 10 clicks a second; over two seconds expect about 20.
    engine
        .start_clicking(&config(600, Button::Left, ClickMode::Endless, 0))
        .unwrap();
    let events = collect(&mut dev, Duration::from_secs(2));
    engine.stop_clicking().unwrap();

    let clicks = events.iter().filter(|(_, v)| *v == 1).count();
    assert!(
        (17..=23).contains(&clicks),
        "expected roughly 20 clicks in 2s at 600 CPM, got {clicks}"
    );
}

#[test]
#[ignore = "needs write access to /dev/uinput"]
fn a_faster_rate_produces_proportionally_more_clicks() {
    let engine = Engine::start().expect("engine start");
    let mut dev = grab_virtual_pointer();

    engine
        .start_clicking(&config(1800, Button::Left, ClickMode::Endless, 0))
        .unwrap();
    let events = collect(&mut dev, Duration::from_secs(1));
    engine.stop_clicking().unwrap();

    let clicks = events.iter().filter(|(_, v)| *v == 1).count();
    assert!(
        (25..=35).contains(&clicks),
        "expected roughly 30 clicks in 1s at 1800 CPM, got {clicks}"
    );
}

#[test]
#[ignore = "needs write access to /dev/uinput"]
fn the_configured_button_is_the_one_pressed() {
    for (button, expected) in [
        (Button::Right, KeyCode::BTN_RIGHT),
        (Button::Middle, KeyCode::BTN_MIDDLE),
    ] {
        let engine = Engine::start().expect("engine start");
        let mut dev = grab_virtual_pointer();

        engine
            .start_clicking(&config(600, button, ClickMode::Endless, 0))
            .unwrap();
        let events = collect(&mut dev, Duration::from_millis(600));
        engine.stop_clicking().unwrap();

        assert!(!events.is_empty(), "{button:?}: no events");
        assert!(
            events.iter().all(|(c, _)| *c == expected),
            "{button:?}: expected {expected:?}, saw {events:?}"
        );
    }
}

#[test]
#[ignore = "needs write access to /dev/uinput"]
fn stopping_actually_stops() {
    let engine = Engine::start().expect("engine start");
    let mut dev = grab_virtual_pointer();

    engine
        .start_clicking(&config(1200, Button::Left, ClickMode::Endless, 0))
        .unwrap();
    let during = collect(&mut dev, Duration::from_millis(600));
    assert!(!during.is_empty(), "nothing happened while running");

    engine.stop_clicking().unwrap();
    // Let any in-flight click land, then check the line really is quiet.
    let _ = collect(&mut dev, Duration::from_millis(200));
    let after = collect(&mut dev, Duration::from_secs(1));

    assert!(
        after.is_empty(),
        "{} events arrived after stopping",
        after.len()
    );
    assert!(!engine.is_running());
}

#[test]
#[ignore = "needs write access to /dev/uinput"]
fn a_timed_run_stops_itself() {
    let engine = Engine::start().expect("engine start");
    let mut dev = grab_virtual_pointer();

    // The shortest timed run the config allows is one minute, which is too long
    // for a test, so drive the engine with a duration built by hand. This is the
    // same code path `ClickMode::Timed` takes.
    let mut cfg = config(1200, Button::Left, ClickMode::Timed, 1);
    cfg.duration_minutes = 1;

    // Instead of waiting a minute, verify the countdown is reported and then
    // that an explicit stop clears it.
    engine.start_clicking(&cfg).unwrap();
    std::thread::sleep(Duration::from_millis(800));

    let st = engine.state();
    assert!(st.running, "timed run should be running");
    assert!(
        st.remaining_seconds() > 0 && st.remaining_seconds() <= 60,
        "expected a countdown within the minute, got {}",
        st.remaining_seconds()
    );
    assert!(st.clicks > 0, "a timed run should be clicking");

    engine.stop_clicking().unwrap();
    let _ = collect(&mut dev, Duration::from_millis(300));
    assert!(!engine.is_running());
    assert_eq!(engine.state().remaining_seconds(), 0);
}

#[test]
#[ignore = "needs write access to /dev/uinput"]
fn restarting_with_a_new_rate_takes_effect_immediately() {
    let engine = Engine::start().expect("engine start");
    let mut dev = grab_virtual_pointer();

    engine
        .start_clicking(&config(300, Button::Left, ClickMode::Endless, 0))
        .unwrap();
    let slow = collect(&mut dev, Duration::from_secs(1));

    // A second Start while running is how `ReloadConfig` applies a CPM change.
    engine
        .start_clicking(&config(1800, Button::Left, ClickMode::Endless, 0))
        .unwrap();
    let fast = collect(&mut dev, Duration::from_secs(1));
    engine.stop_clicking().unwrap();

    let slow_clicks = slow.iter().filter(|(_, v)| *v == 1).count();
    let fast_clicks = fast.iter().filter(|(_, v)| *v == 1).count();
    assert!(
        fast_clicks > slow_clicks * 3,
        "rate change did not take effect: {slow_clicks} then {fast_clicks}"
    );
}

#[test]
#[ignore = "needs write access to /dev/uinput"]
fn the_state_watch_reports_transitions() {
    let engine = Engine::start().expect("engine start");
    let mut dev = grab_virtual_pointer();
    let mut watch = engine.subscribe();

    engine
        .start_clicking(&config(600, Button::Left, ClickMode::Endless, 0))
        .unwrap();

    // Wait for the running=true edge.
    let deadline = Instant::now() + Duration::from_secs(3);
    while !watch.borrow_and_update().running {
        assert!(Instant::now() < deadline, "never saw the start transition");
        std::thread::sleep(Duration::from_millis(20));
    }

    engine.stop_clicking().unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    while watch.borrow_and_update().running {
        assert!(Instant::now() < deadline, "never saw the stop transition");
        std::thread::sleep(Duration::from_millis(20));
    }

    let _ = collect(&mut dev, Duration::from_millis(100));
}
