//! `ratclick-gui` — the libadwaita front end.
//!
//! Launched by `ratclick gui`, by the desktop entry, and by the GNOME Shell
//! extension's Settings item.

mod bridge;
mod capture;
mod window;
mod wizard;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
use ratclick_core::config::Config;

fn main() -> gtk::glib::ExitCode {
    // A GUI has nowhere to print a panic, so surface config problems as a
    // dialog rather than dying silently from the desktop entry.
    let app = adw::Application::builder()
        .application_id(window::APP_ID)
        .flags(gio::ApplicationFlags::default())
        .build();

    let held: Rc<RefCell<Option<adw::ApplicationWindow>>> = Rc::new(RefCell::new(None));

    app.connect_activate({
        let held = held.clone();
        move |app| {
            // Already open: just raise it.
            if let Some(w) = held.borrow().as_ref() {
                w.present();
                return;
            }
            let win = open(app, held.clone());
            *held.borrow_mut() = Some(win.clone());
            win.present();
        }
    });

    register_actions(&app, held.clone());
    app.run()
}

/// Decide between the wizard and the main window.
fn open(
    app: &adw::Application,
    held: Rc<RefCell<Option<adw::ApplicationWindow>>>,
) -> adw::ApplicationWindow {
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            // A corrupt config should not lock the user out of the app; start
            // from defaults and say so.
            let win = error_window(app, &format!("{e:#}"));
            return win;
        }
    };

    if config.needs_setup() {
        let app2 = app.clone();
        let held2 = held.clone();
        wizard::build(app, config, move |cfg| {
            // The wizard finished: swap in the real window.
            let win = window::build(&app2, cfg);
            *held2.borrow_mut() = Some(win.clone());
            win.present();
        })
    } else {
        window::build(app, config)
    }
}

fn error_window(app: &adw::Application, message: &str) -> adw::ApplicationWindow {
    let win = adw::ApplicationWindow::builder()
        .application(app)
        .title("RatClick")
        .default_width(460)
        .default_height(340)
        .build();

    let status = adw::StatusPage::builder()
        .icon_name("dialog-warning-symbolic")
        .title("RatClick cannot read its settings")
        .description(message)
        .build();

    let reset = gtk::Button::builder()
        .label("Start Over")
        .halign(gtk::Align::Center)
        .build();
    reset.add_css_class("pill");
    reset.add_css_class("destructive-action");
    {
        let win = win.clone();
        reset.connect_clicked(move |_| {
            if let Ok(path) = Config::path() {
                let _ = std::fs::remove_file(path);
            }
            win.close();
        });
    }
    status.set_child(Some(&reset));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&status));
    win.set_content(Some(&toolbar));
    win
}

fn register_actions(app: &adw::Application, held: Rc<RefCell<Option<adw::ApplicationWindow>>>) {
    let about = gio::SimpleAction::new("about", None);
    {
        let held = held.clone();
        about.connect_activate(move |_, _| {
            if let Some(w) = held.borrow().as_ref() {
                window::present_about(w);
            }
        });
    }
    app.add_action(&about);

    // Re-run the wizard: clearing the flag is enough, the next open picks it up.
    let setup = gio::SimpleAction::new("setup", None);
    {
        let held = held.clone();
        let app2 = app.clone();
        setup.connect_activate(move |_, _| {
            let mut cfg = Config::load().unwrap_or_default();
            cfg.setup_complete = false;
            let _ = cfg.save();

            if let Some(w) = held.borrow().as_ref() {
                w.close();
            }
            let app3 = app2.clone();
            let held2 = held.clone();
            let win = wizard::build(&app2, cfg, move |done| {
                let w = window::build(&app3, done);
                *held2.borrow_mut() = Some(w.clone());
                w.present();
            });
            *held.borrow_mut() = Some(win.clone());
            win.present();
        });
    }
    app.add_action(&setup);

    // `ratclick doctor` is a terminal tool; the GUI just shows what it would.
    let doctor = gio::SimpleAction::new("doctor", None);
    {
        let held = held.clone();
        doctor.connect_activate(move |_, _| {
            if let Some(w) = held.borrow().as_ref() {
                present_doctor(w);
            }
        });
    }
    app.add_action(&doctor);

    app.set_accels_for_action("app.about", &["F1"]);
    app.set_accels_for_action("window.close", &["<Control>w"]);
}

/// A GUI rendering of the same checks `ratclick doctor` runs.
fn present_doctor(parent: &impl IsA<gtk::Widget>) {
    use ratclick_core::config::ShortcutBackend;
    use ratclick_core::shortcut;

    let dialog = adw::Dialog::builder()
        .title("Installation Check")
        .content_width(520)
        .content_height(560)
        .build();

    let page = adw::PreferencesPage::new();

    let input = adw::PreferencesGroup::builder()
        .title("Input device")
        .build();
    let uinput_ok = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok();
    input.add(&check_row(
        "/dev/uinput",
        uinput_ok,
        if uinput_ok {
            "Writable — RatClick can create its virtual mouse".to_string()
        } else {
            "Not writable. Run `sudo usermod -aG input $USER`, then log out and back in."
                .to_string()
        },
    ));
    page.add(&input);

    let cfg = Config::load().unwrap_or_default();
    let backends = adw::PreferencesGroup::builder()
        .title("Shortcut backends")
        .build();
    for s in shortcut::backend_statuses() {
        if s.backend == ShortcutBackend::None {
            continue;
        }
        let label = if s.backend == cfg.shortcut.backend {
            format!("{} (in use)", s.backend.as_str())
        } else {
            s.backend.as_str().to_string()
        };
        backends.add(&check_row(&label, s.available, s.detail));
    }
    page.add(&backends);

    let sc = adw::PreferencesGroup::builder().title("Shortcut").build();
    if cfg.shortcut.bindings.is_empty() {
        sc.add(&check_row(
            "Binding",
            false,
            "No shortcut configured".into(),
        ));
    } else {
        let live = shortcut::installed(cfg.shortcut.backend);
        for a in &cfg.shortcut.bindings {
            let ok = live.contains(a);
            sc.add(&check_row(
                &a.to_display(),
                ok,
                if ok {
                    "Registered".into()
                } else {
                    "Configured but not registered".into()
                },
            ));
        }
    }
    page.add(&sc);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&gtk::ScrolledWindow::builder().child(&page).build()));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(parent.as_ref()));
}

fn check_row(title: &str, ok: bool, detail: String) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(&detail)
        .build();
    let icon = gtk::Image::from_icon_name(if ok {
        "emblem-ok-symbolic"
    } else {
        "dialog-warning-symbolic"
    });
    icon.add_css_class(if ok { "success" } else { "warning" });
    row.add_prefix(&icon);
    row
}
