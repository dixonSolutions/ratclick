//! First-run guided setup.
//!
//! Shown when `config.toml` is missing or `setup_complete` is false. It walks
//! through the same four decisions as `ratclick setup` — speed, button, run
//! length, shortcut — but with live key capture, which the terminal cannot do.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use ratclick_core::accel::Accel;
use ratclick_core::config::{Button, ClickMode, Config, ShortcutBackend, MIN_CPM};
use ratclick_core::shortcut;

use crate::capture;

const ICON: &str = "io.github.dixonsolutions.RatClick";

struct State {
    config: RefCell<Config>,
    nav: adw::NavigationView,
    window: adw::ApplicationWindow,
    /// Called by the last page with the finished configuration.
    on_finish: Box<dyn Fn(Config)>,
}

/// Build the wizard window. `on_finish` receives the completed configuration.
pub fn build<F>(app: &adw::Application, config: Config, on_finish: F) -> adw::ApplicationWindow
where
    F: Fn(Config) + 'static,
{
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("RatClick Setup")
        .default_width(500)
        .default_height(640)
        .build();

    let nav = adw::NavigationView::new();
    window.set_content(Some(&nav));

    let state = Rc::new(State {
        config: RefCell::new(config),
        nav: nav.clone(),
        window: window.clone(),
        on_finish: Box::new(on_finish),
    });

    nav.push(&welcome_page(&state));
    window
}

fn page(title: &str, child: &impl IsA<gtk::Widget>, tag: &str) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .child(child)
            .vexpand(true)
            .build(),
    ));
    let p = adw::NavigationPage::new(&toolbar, title);
    p.set_tag(Some(tag));
    p
}

fn next_button(label: &str) -> gtk::Button {
    let b = gtk::Button::builder()
        .label(label)
        .halign(gtk::Align::Center)
        .margin_top(24)
        .margin_bottom(24)
        .build();
    b.add_css_class("pill");
    b.add_css_class("suggested-action");
    b
}

// ---- 1. Welcome ---------------------------------------------------------

fn welcome_page(state: &Rc<State>) -> adw::NavigationPage {
    let status = adw::StatusPage::builder()
        .icon_name(ICON)
        .title("Welcome to RatClick")
        .description(
            "RatClick clicks your mouse for you, as fast as you like, for as long as you like.\n\n\
             This takes about a minute. You can change anything afterwards.",
        )
        .build();

    let start = next_button("Get Started");
    status.set_child(Some(&start));

    let state = state.clone();
    start.connect_clicked(move |_| {
        state.nav.push(&speed_page(&state));
    });

    page("Welcome", &status, "welcome")
}

// ---- 2. Speed and button ------------------------------------------------

fn speed_page(state: &Rc<State>) -> adw::NavigationPage {
    let p = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title("How should it click?")
        .description("You can change these at any time.")
        .build();

    let cfg = state.config.borrow().clone();

    // No upper bound on speed; the widget still needs a finite ceiling, so
    // give it one far past anything a human would type.
    let cpm = adw::SpinRow::with_range(MIN_CPM as f64, u32::MAX as f64, 10.0);
    cpm.set_title("Clicks per minute");
    cpm.set_subtitle("600 is ten clicks a second — no maximum");
    cpm.set_value(cfg.click.cpm as f64);

    let button = adw::ComboRow::builder().title("Mouse button").build();
    let list = gtk::StringList::new(&["Left", "Right", "Middle"]);
    button.set_model(Some(&list));
    button.set_selected(match cfg.click.button {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
    });

    group.add(&cpm);
    group.add(&button);
    p.add(&group);

    let next = next_button("Continue");
    let holder = adw::PreferencesGroup::new();
    holder.add(&next);
    p.add(&holder);

    let state2 = state.clone();
    next.connect_clicked(move |_| {
        {
            let mut cfg = state2.config.borrow_mut();
            cfg.click.cpm = cpm.value().round() as u32;
            cfg.click.button = match button.selected() {
                1 => Button::Right,
                2 => Button::Middle,
                _ => Button::Left,
            };
        }
        state2.nav.push(&duration_page(&state2));
    });

    page("Speed", &p, "speed")
}

// ---- 3. Run length ------------------------------------------------------

fn duration_page(state: &Rc<State>) -> adw::NavigationPage {
    let p = adw::PreferencesPage::new();
    let cfg = state.config.borrow().clone();

    let group = adw::PreferencesGroup::builder()
        .title("How long should a run last?")
        .build();

    let mode = adw::ComboRow::builder().title("Run length").build();
    mode.set_model(Some(&gtk::StringList::new(&[
        "Endless — until I stop it",
        "Timed — stop by itself",
    ])));
    mode.set_selected(if cfg.click.mode == ClickMode::Timed {
        1
    } else {
        0
    });
    group.add(&mode);
    p.add(&group);

    let duration = adw::PreferencesGroup::builder()
        .title("Run for")
        .visible(cfg.click.mode == ClickMode::Timed)
        .build();
    let (h0, m0) = cfg.click.duration_hm();
    let hours = adw::SpinRow::with_range(0.0, 24.0, 1.0);
    hours.set_title("Hours");
    hours.set_value(h0 as f64);
    let minutes = adw::SpinRow::with_range(0.0, 59.0, 1.0);
    minutes.set_title("Minutes");
    minutes.set_value(m0 as f64);
    duration.add(&hours);
    duration.add(&minutes);
    p.add(&duration);

    {
        let duration = duration.clone();
        mode.connect_selected_notify(move |row| {
            duration.set_visible(row.selected() == 1);
        });
    }

    let next = next_button("Continue");
    let holder = adw::PreferencesGroup::new();
    holder.add(&next);
    p.add(&holder);

    let state2 = state.clone();
    next.connect_clicked(move |_| {
        {
            let mut cfg = state2.config.borrow_mut();
            if mode.selected() == 1 {
                cfg.click.mode = ClickMode::Timed;
                cfg.click
                    .set_duration_hm(hours.value().round() as u32, minutes.value().round() as u32);
            } else {
                cfg.click.mode = ClickMode::Endless;
            }
        }
        state2.nav.push(&shortcut_page(&state2));
    });

    page("Run length", &p, "duration")
}

// ---- 4. Shortcut --------------------------------------------------------

fn shortcut_page(state: &Rc<State>) -> adw::NavigationPage {
    let p = adw::PreferencesPage::new();
    let cfg = state.config.borrow().clone();
    let statuses = shortcut::backend_statuses();

    let group = adw::PreferencesGroup::builder()
        .title("Pick a toggle shortcut")
        .description("One key combination starts and stops clicking from anywhere.")
        .build();

    let backend = adw::ComboRow::builder().title("Registered with").build();
    let labels: Vec<String> = statuses
        .iter()
        .map(|s| {
            let name = match s.backend {
                ShortcutBackend::Gnome => "GNOME keyboard shortcut",
                ShortcutBackend::Extension => "GNOME Shell extension",
                ShortcutBackend::Keyd => "keyd (system-wide)",
                ShortcutBackend::None => "No shortcut",
            };
            if s.available {
                name.to_string()
            } else {
                format!("{name} — unavailable")
            }
        })
        .collect();
    let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    backend.set_model(Some(&gtk::StringList::new(&refs)));
    // Default to the first backend that actually works here.
    let default_idx = statuses
        .iter()
        .position(|s| s.available && s.backend != ShortcutBackend::None)
        .unwrap_or(statuses.len() - 1);
    backend.set_selected(default_idx as u32);

    let hint = adw::ActionRow::builder()
        .title(&statuses[default_idx].detail)
        .build();
    hint.add_css_class("dim-label");

    let shortcut_row = adw::ActionRow::builder()
        .title("Shortcut")
        .subtitle("Not set")
        .build();
    let set_button = gtk::Button::builder()
        .label("Press keys…")
        .valign(gtk::Align::Center)
        .build();
    set_button.add_css_class("suggested-action");
    shortcut_row.add_suffix(&set_button);

    group.add(&backend);
    group.add(&hint);
    group.add(&shortcut_row);
    p.add(&group);

    let chosen: Rc<RefCell<Option<Accel>>> =
        Rc::new(RefCell::new(cfg.shortcut.bindings.first().cloned()));

    let next = next_button("Continue");
    let holder = adw::PreferencesGroup::new();
    holder.add(&next);
    p.add(&holder);

    // Selecting "No shortcut" should not leave the user stuck on an unset row.
    {
        let statuses = statuses.clone();
        let hint = hint.clone();
        let shortcut_row = shortcut_row.clone();
        backend.connect_selected_notify(move |row| {
            let idx = row.selected() as usize;
            if let Some(s) = statuses.get(idx) {
                hint.set_title(&s.detail);
                shortcut_row.set_sensitive(s.backend != ShortcutBackend::None);
            }
        });
    }

    {
        let chosen = chosen.clone();
        let shortcut_row = shortcut_row.clone();
        let window = state.window.clone();
        set_button.connect_clicked(move |_| {
            let chosen = chosen.clone();
            let shortcut_row = shortcut_row.clone();
            capture::present(&window, move |accel| {
                shortcut_row.set_subtitle(&accel.to_display());
                *chosen.borrow_mut() = Some(accel);
            });
        });
    }
    if let Some(a) = chosen.borrow().as_ref() {
        shortcut_row.set_subtitle(&a.to_display());
    }

    let state2 = state.clone();
    let statuses2 = statuses.clone();
    next.connect_clicked(move |_| {
        {
            let mut cfg = state2.config.borrow_mut();
            let idx = backend.selected() as usize;
            cfg.shortcut.backend = statuses2
                .get(idx)
                .map(|s| s.backend)
                .unwrap_or(ShortcutBackend::None);
            cfg.shortcut.bindings = match (cfg.shortcut.backend, chosen.borrow().clone()) {
                (ShortcutBackend::None, _) | (_, None) => Vec::new(),
                (_, Some(a)) => vec![a],
            };
            // A backend with no key chosen is the same as no shortcut.
            if cfg.shortcut.bindings.is_empty() {
                cfg.shortcut.backend = ShortcutBackend::None;
            }
        }
        state2.nav.push(&finish_page(&state2));
    });

    page("Shortcut", &p, "shortcut")
}

// ---- 5. Finish ----------------------------------------------------------

fn finish_page(state: &Rc<State>) -> adw::NavigationPage {
    let mut cfg = state.config.borrow().clone();
    cfg.setup_complete = true;
    cfg.normalise();

    let mut problems: Vec<String> = Vec::new();

    if let Err(e) = cfg.save() {
        problems.push(format!("Could not save settings: {e}"));
    }

    // Install the shortcut and verify it landed, rather than claiming success.
    if cfg.shortcut.backend != ShortcutBackend::None {
        if let Some(accel) = cfg.shortcut.bindings.first() {
            if let Ok((taken, refused)) = shortcut::force_take(accel) {
                for r in refused {
                    problems.push(format!("Still bound elsewhere: {r}"));
                }
                let _ = taken;
            }
        }
        match shortcut::apply(&cfg) {
            Ok(_) => {
                let live = shortcut::installed(cfg.shortcut.backend);
                if live != cfg.shortcut.bindings {
                    problems.push(
                        "The shortcut was written but did not register. Run `ratclick doctor` \
                         in a terminal for details."
                            .into(),
                    );
                }
            }
            Err(e) => problems.push(match cfg.shortcut.backend {
                ShortcutBackend::Keyd => format!(
                    "keyd needs administrator rights: run `sudo ratclick shortcut apply` \
                     in a terminal. ({e})"
                ),
                _ => format!("Could not register the shortcut: {e}"),
            }),
        }
    }

    *state.config.borrow_mut() = cfg.clone();

    let (h, m) = cfg.click.duration_hm();
    let run_for = match cfg.click.mode {
        ClickMode::Endless => "until you stop it".to_string(),
        ClickMode::Timed => format!("for {h}h {m:02}m"),
    };
    let shortcut_line = match cfg.shortcut.bindings.first() {
        Some(a) => format!("Press {} to start and stop.", a.to_display()),
        None => "No shortcut — use the window or `ratclick toggle`.".to_string(),
    };

    let status = adw::StatusPage::builder()
        // The app's own icon, not a themed checkmark: it ships in the package,
        // so it cannot go missing the way `emblem-ok-symbolic` did when Adwaita
        // 50 dropped the legacy emblems. Bookends the welcome page too.
        .icon_name(ICON)
        .title("You're set")
        .description(format!(
            "{} clicks a minute with the {} button, running {run_for}.\n{shortcut_line}",
            cfg.click.cpm,
            cfg.click.button.as_str()
        ))
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);

    for p in &problems {
        let banner = adw::Banner::builder().title(p).revealed(true).build();
        content.append(&banner);
    }

    let open = next_button("Open RatClick");
    content.append(&open);
    status.set_child(Some(&content));

    let state2 = state.clone();
    open.connect_clicked(move |_| {
        let cfg = state2.config.borrow().clone();
        (state2.on_finish)(cfg);
        state2.window.close();
    });

    page("Done", &status, "finish")
}
