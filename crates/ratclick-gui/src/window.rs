//! The main RatClick window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use ratclick_core::accel::Accel;
use ratclick_core::config::{Button, ClickMode, Config, ShortcutBackend, MIN_CPM};
use ratclick_core::{ipc, shortcut};

use crate::bridge::{Bridge, Cmd, Snapshot};
use crate::capture;

/// The GUI owns this name on the session bus (GtkApplication does it for
/// us); the daemon deliberately uses a different one. See `ipc::BUS_NAME`.
pub const APP_ID: &str = ratclick_core::ipc::APP_ID;
const ICON: &str = "io.github.dixonsolutions.RatClick";

const BUTTONS: &[(Button, &str)] = &[
    (Button::Left, "Left"),
    (Button::Right, "Right"),
    (Button::Middle, "Middle"),
];

const BACKENDS: &[(ShortcutBackend, &str)] = &[
    (ShortcutBackend::Gnome, "GNOME keyboard shortcut"),
    (ShortcutBackend::Extension, "GNOME Shell extension"),
    (ShortcutBackend::Keyd, "keyd (system-wide)"),
    (ShortcutBackend::None, "No shortcut"),
];

/// Widgets and state the callbacks need to reach.
struct Ui {
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,

    // Status
    status_icon: gtk::Image,
    status_title: gtk::Label,
    status_detail: gtk::Label,
    toggle_button: gtk::Button,

    // Clicking
    cpm: adw::SpinRow,
    button: adw::ComboRow,
    mode: adw::ComboRow,
    duration_group: adw::PreferencesGroup,
    hours: adw::SpinRow,
    minutes: adw::SpinRow,
    autostart: adw::SwitchRow,

    // Shortcut
    backend: adw::ComboRow,
    shortcut_row: adw::ActionRow,
    backend_hint: adw::ActionRow,

    // Service
    service_row: adw::ActionRow,
    service_button: gtk::Button,

    config: RefCell<Config>,
    bridge: Bridge,
    /// Set while pushing config values into widgets, so the change handlers do
    /// not write the file back and fight the update.
    loading: Cell<bool>,
}

pub fn build(app: &adw::Application, config: Config) -> adw::ApplicationWindow {
    let bridge = Bridge::start();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("RatClick")
        .default_width(460)
        .default_height(760)
        .width_request(360)
        .height_request(400)
        .build();

    let toasts = adw::ToastOverlay::new();
    let page = adw::PreferencesPage::new();

    // ---- Status ---------------------------------------------------------
    let status_group = adw::PreferencesGroup::new();
    let status_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(12)
        .halign(gtk::Align::Center)
        .build();

    let status_icon = gtk::Image::builder().icon_name(ICON).pixel_size(96).build();
    let status_title = gtk::Label::new(Some("Ready"));
    status_title.add_css_class("title-1");
    let status_detail = gtk::Label::new(Some(""));
    status_detail.add_css_class("dim-label");
    status_detail.set_wrap(true);
    status_detail.set_justify(gtk::Justification::Center);

    let toggle_button = gtk::Button::builder()
        .label("Start Clicking")
        .halign(gtk::Align::Center)
        .margin_top(12)
        .build();
    toggle_button.add_css_class("pill");
    toggle_button.add_css_class("suggested-action");

    status_box.append(&status_icon);
    status_box.append(&status_title);
    status_box.append(&status_detail);
    status_box.append(&toggle_button);
    status_group.add(&status_box);
    page.add(&status_group);

    // ---- Clicking -------------------------------------------------------
    let click_group = adw::PreferencesGroup::builder().title("Clicking").build();

    // No upper bound on speed; the widget still needs a finite ceiling, so
    // give it one far past anything a human would type.
    let cpm = adw::SpinRow::with_range(MIN_CPM as f64, u32::MAX as f64, 10.0);
    cpm.set_title("Clicks per minute");
    cpm.set_subtitle("600 is ten clicks a second — no maximum");

    let button = adw::ComboRow::builder().title("Mouse button").build();
    button.set_model(Some(&string_list(BUTTONS.iter().map(|(_, l)| *l))));

    let mode = adw::ComboRow::builder()
        .title("Run length")
        .subtitle("Endless runs until you stop it")
        .build();
    mode.set_model(Some(&string_list(["Endless", "Timed"])));

    let autostart = adw::SwitchRow::builder()
        .title("Start clicking on login")
        .subtitle("Begin as soon as the service starts")
        .build();

    click_group.add(&cpm);
    click_group.add(&button);
    click_group.add(&mode);
    click_group.add(&autostart);
    page.add(&click_group);

    // Duration is its own group so it can be hidden wholesale in endless mode.
    let duration_group = adw::PreferencesGroup::builder()
        .title("Run for")
        .description("Each run stops itself after this long")
        .build();
    let hours = adw::SpinRow::with_range(0.0, 24.0, 1.0);
    hours.set_title("Hours");
    let minutes = adw::SpinRow::with_range(0.0, 59.0, 1.0);
    minutes.set_title("Minutes");
    duration_group.add(&hours);
    duration_group.add(&minutes);
    page.add(&duration_group);

    // ---- Shortcut -------------------------------------------------------
    let shortcut_group = adw::PreferencesGroup::builder()
        .title("Toggle shortcut")
        .description("One key combination starts and stops clicking")
        .build();

    let backend = adw::ComboRow::builder().title("Registered with").build();
    backend.set_model(Some(&string_list(BACKENDS.iter().map(|(_, l)| *l))));

    let backend_hint = adw::ActionRow::builder().build();
    backend_hint.add_css_class("dim-label");

    let shortcut_row = adw::ActionRow::builder()
        .title("Shortcut")
        .subtitle("Not set")
        .build();
    let set_button = gtk::Button::builder()
        .label("Set…")
        .valign(gtk::Align::Center)
        .build();
    let clear_button = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text("Remove the shortcut")
        .valign(gtk::Align::Center)
        .build();
    clear_button.add_css_class("flat");
    shortcut_row.add_suffix(&set_button);
    shortcut_row.add_suffix(&clear_button);

    shortcut_group.add(&backend);
    shortcut_group.add(&backend_hint);
    shortcut_group.add(&shortcut_row);
    page.add(&shortcut_group);

    // ---- Service --------------------------------------------------------
    let service_group = adw::PreferencesGroup::builder()
        .title("Background service")
        .description("RatClick needs this running to click. It starts on demand.")
        .build();
    let service_row = adw::ActionRow::builder()
        .title("Service")
        .subtitle("Checking…")
        .build();
    let service_button = gtk::Button::builder()
        .label("Start")
        .valign(gtk::Align::Center)
        .build();
    service_row.add_suffix(&service_button);
    service_group.add(&service_row);
    page.add(&service_group);

    // ---- Chrome ---------------------------------------------------------
    let header = adw::HeaderBar::new();
    let menu = gio_menu();
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .tooltip_text("Main Menu")
        .build();
    menu_button.set_primary(true);
    header.pack_end(&menu_button);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&gtk::ScrolledWindow::builder().child(&page).build()));
    toasts.set_child(Some(&toolbar));
    window.set_content(Some(&toasts));

    let ui = Rc::new(Ui {
        window: window.clone(),
        toasts,
        status_icon,
        status_title,
        status_detail,
        toggle_button: toggle_button.clone(),
        cpm: cpm.clone(),
        button: button.clone(),
        mode: mode.clone(),
        duration_group,
        hours: hours.clone(),
        minutes: minutes.clone(),
        autostart: autostart.clone(),
        backend: backend.clone(),
        shortcut_row,
        backend_hint,
        service_row,
        service_button: service_button.clone(),
        config: RefCell::new(config),
        bridge,
        loading: Cell::new(false),
    });

    ui.load_into_widgets();
    ui.wire_up(&set_button, &clear_button);
    ui.watch_daemon();

    window
}

fn string_list<'a>(items: impl IntoIterator<Item = &'a str>) -> gtk::StringList {
    let list = gtk::StringList::new(&[]);
    for i in items {
        list.append(i);
    }
    list
}

fn gio_menu() -> gtk::gio::Menu {
    let menu = gtk::gio::Menu::new();
    menu.append(Some("Run Setup Again"), Some("app.setup"));
    menu.append(Some("Check Installation"), Some("app.doctor"));
    menu.append(Some("About RatClick"), Some("app.about"));
    menu
}

impl Ui {
    // ---- config <-> widgets ---------------------------------------------

    fn load_into_widgets(self: &Rc<Self>) {
        self.loading.set(true);
        let cfg = self.config.borrow().clone();

        self.cpm.set_value(cfg.click.cpm as f64);
        self.button.set_selected(
            BUTTONS
                .iter()
                .position(|(b, _)| *b == cfg.click.button)
                .unwrap_or(0) as u32,
        );
        self.mode
            .set_selected(if cfg.click.mode == ClickMode::Timed {
                1
            } else {
                0
            });
        let (h, m) = cfg.click.duration_hm();
        self.hours.set_value(h as f64);
        self.minutes.set_value(m as f64);
        self.autostart.set_active(cfg.start_clicking_on_launch);
        self.backend.set_selected(
            BACKENDS
                .iter()
                .position(|(b, _)| *b == cfg.shortcut.backend)
                .unwrap_or(0) as u32,
        );

        self.duration_group
            .set_visible(cfg.click.mode == ClickMode::Timed);
        self.refresh_shortcut_row();
        self.refresh_backend_hint();

        self.loading.set(false);
    }

    fn refresh_shortcut_row(&self) {
        let cfg = self.config.borrow();
        let enabled = cfg.shortcut.backend != ShortcutBackend::None;
        self.shortcut_row.set_sensitive(enabled);

        if cfg.shortcut.bindings.is_empty() {
            self.shortcut_row.set_subtitle("Not set");
            return;
        }
        let labels = cfg
            .shortcut
            .bindings
            .iter()
            .map(|a| a.to_display())
            .collect::<Vec<_>>()
            .join(", ");

        // Read the binding back from the backend rather than trusting the file,
        // so a shortcut that silently failed to register is visible here.
        let live = shortcut::installed(cfg.shortcut.backend);
        if live == cfg.shortcut.bindings {
            self.shortcut_row.set_subtitle(&labels);
        } else {
            self.shortcut_row
                .set_subtitle(&format!("{labels} — not registered"));
        }
    }

    fn refresh_backend_hint(&self) {
        let cfg = self.config.borrow();
        let statuses = shortcut::backend_statuses();
        if let Some(s) = statuses.iter().find(|s| s.backend == cfg.shortcut.backend) {
            self.backend_hint.set_title(&s.detail);
            self.backend_hint.set_visible(true);
        } else {
            self.backend_hint.set_visible(false);
        }
    }

    fn save(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        let mut cfg = self.config.borrow_mut();
        cfg.setup_complete = true;
        for note in cfg.normalise() {
            self.toast(&note);
        }
        if let Err(e) = cfg.save() {
            self.toast(&format!("Could not save settings: {e}"));
            return;
        }
        drop(cfg);
        self.bridge.send(Cmd::Reload);
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    // ---- signals ---------------------------------------------------------

    fn wire_up(self: &Rc<Self>, set_button: &gtk::Button, clear_button: &gtk::Button) {
        let ui = self.clone();
        self.cpm.connect_value_notify(move |row| {
            if ui.loading.get() {
                return;
            }
            ui.config.borrow_mut().click.cpm = row.value().round() as u32;
            ui.save();
        });

        let ui = self.clone();
        self.button.connect_selected_notify(move |row| {
            if ui.loading.get() {
                return;
            }
            let idx = row.selected() as usize;
            if let Some((b, _)) = BUTTONS.get(idx) {
                ui.config.borrow_mut().click.button = *b;
                ui.save();
            }
        });

        let ui = self.clone();
        self.mode.connect_selected_notify(move |row| {
            if ui.loading.get() {
                return;
            }
            let timed = row.selected() == 1;
            ui.config.borrow_mut().click.mode = if timed {
                ClickMode::Timed
            } else {
                ClickMode::Endless
            };
            ui.duration_group.set_visible(timed);
            ui.save();
        });

        for row in [&self.hours, &self.minutes] {
            let ui = self.clone();
            row.connect_value_notify(move |_| {
                if ui.loading.get() {
                    return;
                }
                let h = ui.hours.value().round() as u32;
                let m = ui.minutes.value().round() as u32;
                if h == 0 && m == 0 {
                    // A zero-length run would stop the instant it started.
                    ui.loading.set(true);
                    ui.minutes.set_value(1.0);
                    ui.loading.set(false);
                    ui.toast("A run has to last at least a minute");
                    ui.config.borrow_mut().click.set_duration_hm(0, 1);
                } else {
                    ui.config.borrow_mut().click.set_duration_hm(h, m);
                }
                ui.save();
            });
        }

        let ui = self.clone();
        self.autostart.connect_active_notify(move |row| {
            if ui.loading.get() {
                return;
            }
            ui.config.borrow_mut().start_clicking_on_launch = row.is_active();
            ui.save();
        });

        let ui = self.clone();
        self.backend.connect_selected_notify(move |row| {
            if ui.loading.get() {
                return;
            }
            let Some((backend, _)) = BACKENDS.get(row.selected() as usize) else {
                return;
            };
            ui.config.borrow_mut().shortcut.backend = *backend;
            ui.save();
            ui.apply_shortcut();
            ui.refresh_backend_hint();
            ui.refresh_shortcut_row();
        });

        let ui = self.clone();
        set_button.connect_clicked(move |_| {
            let ui2 = ui.clone();
            capture::present(&ui.window, move |accel| {
                ui2.set_shortcut(accel);
            });
        });

        let ui = self.clone();
        clear_button.connect_clicked(move |_| {
            if let Err(e) = shortcut::remove_all() {
                ui.toast(&format!("Could not remove the shortcut: {e}"));
            }
            ui.config.borrow_mut().shortcut.bindings.clear();
            ui.save();
            ui.refresh_shortcut_row();
            ui.toast("Shortcut removed");
        });

        let ui = self.clone();
        self.toggle_button.connect_clicked(move |_| {
            ui.bridge.send(Cmd::Toggle);
        });

        let ui = self.clone();
        self.service_button.connect_clicked(move |btn| {
            // The label is the source of truth for which way we are going, and
            // it is derived from the last snapshot.
            if btn.label().as_deref() == Some("Stop") {
                ui.bridge.send(Cmd::StopDaemon);
            } else {
                ui.bridge.send(Cmd::StartDaemon);
            }
        });
    }

    fn set_shortcut(self: &Rc<Self>, accel: Accel) {
        // The capture dialog already showed any conflict and the user chose to
        // continue, so take it.
        match shortcut::force_take(&accel) {
            Ok((taken, refused)) => {
                for t in taken {
                    self.toast(&format!("Unbound {t}"));
                }
                for r in refused {
                    self.toast(&format!("Still bound elsewhere: {r}"));
                }
            }
            Err(e) => self.toast(&format!("Could not free the shortcut: {e}")),
        }

        {
            let mut cfg = self.config.borrow_mut();
            if cfg.shortcut.backend == ShortcutBackend::None {
                cfg.shortcut.backend = ShortcutBackend::Gnome;
            }
            cfg.shortcut.bindings = vec![accel.clone()];
        }
        self.load_into_widgets();
        self.save();
        self.apply_shortcut();
    }

    /// Install the configured shortcut, escalating for keyd when needed.
    fn apply_shortcut(self: &Rc<Self>) {
        let cfg = self.config.borrow().clone();
        if cfg.shortcut.backend == ShortcutBackend::None || cfg.shortcut.bindings.is_empty() {
            return;
        }

        match shortcut::apply(&cfg) {
            Ok(_) => {
                self.refresh_shortcut_row();
                let live = shortcut::installed(cfg.shortcut.backend);
                if live == cfg.shortcut.bindings {
                    self.toast(&format!(
                        "{} will toggle clicking",
                        cfg.shortcut.bindings[0].to_display()
                    ));
                } else {
                    self.toast("The shortcut did not register — see `ratclick doctor`");
                }
            }
            Err(e) if cfg.shortcut.backend == ShortcutBackend::Keyd => {
                // keyd lives in /etc, so this needs a privileged helper.
                self.escalate_keyd(&e.to_string());
            }
            Err(e) => self.toast(&format!("Could not set the shortcut: {e}")),
        }
    }

    /// Ask for admin rights and re-run the keyd installation as root.
    fn escalate_keyd(self: &Rc<Self>, why: &str) {
        let dialog = adw::AlertDialog::new(
            Some("Administrator access needed"),
            Some(&format!(
                "keyd shortcuts live in /etc/keyd, so RatClick needs to run one command as \
                 root to install them.\n\n{why}"
            )),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Continue");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));

        let ui = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "ok" {
                return;
            }
            match std::process::Command::new("pkexec")
                .args(["ratclick", "shortcut", "apply"])
                .status()
            {
                Ok(s) if s.success() => {
                    ui.refresh_shortcut_row();
                    ui.toast("keyd shortcut installed");
                }
                Ok(_) => ui.toast("The privileged step was cancelled or failed"),
                Err(e) => ui.toast(&format!("Could not run pkexec: {e}")),
            }
        });
        dialog.present(Some(&self.window));
    }

    // ---- live state ------------------------------------------------------

    fn watch_daemon(self: &Rc<Self>) {
        let ui = self.clone();
        let events = self.bridge.events.clone();
        glib::spawn_future_local(async move {
            while let Ok(snap) = events.recv().await {
                ui.render(&snap);
            }
        });
        self.bridge.send(Cmd::Refresh);
    }

    fn render(self: &Rc<Self>, snap: &Snapshot) {
        if let Some(err) = &snap.error {
            self.toast(err);
        }

        if !snap.daemon_up {
            self.status_title.set_text("Service stopped");
            self.status_detail
                .set_text("Press Start Clicking and RatClick will start it for you.");
            self.status_icon.remove_css_class("accent");
            self.toggle_button.set_label("Start Clicking");
            self.toggle_button.remove_css_class("destructive-action");
            self.toggle_button.add_css_class("suggested-action");
            self.service_row.set_subtitle("Not running");
            self.service_button.set_label("Start");
            return;
        }

        self.service_row.set_subtitle("Running");
        self.service_button.set_label("Stop");

        if snap.running {
            self.status_title.set_text("Clicking");
            self.status_icon.add_css_class("accent");
            let detail = if snap.mode == "timed" {
                format!(
                    "{} clicks/min · {} left · {} clicks so far",
                    snap.cpm,
                    ipc::format_remaining(snap.remaining_seconds),
                    snap.clicks
                )
            } else {
                format!(
                    "{} clicks/min · endless · {} clicks so far",
                    snap.cpm, snap.clicks
                )
            };
            self.status_detail.set_text(&detail);
            self.toggle_button.set_label("Stop Clicking");
            self.toggle_button.remove_css_class("suggested-action");
            self.toggle_button.add_css_class("destructive-action");
        } else {
            self.status_title.set_text("Ready");
            self.status_icon.remove_css_class("accent");
            let cfg = self.config.borrow();
            let hint = match cfg.shortcut.bindings.first() {
                Some(a) => format!("Press {} or use the button below.", a.to_display()),
                None => "Set a shortcut below to toggle from anywhere.".to_string(),
            };
            self.status_detail.set_text(&hint);
            self.toggle_button.set_label("Start Clicking");
            self.toggle_button.remove_css_class("destructive-action");
            self.toggle_button.add_css_class("suggested-action");
        }
    }
}

/// Shared by the menu action and the wizard's finish step.
pub fn present_about(parent: &impl IsA<gtk::Widget>) {
    let about = adw::AboutDialog::builder()
        .application_name("RatClick")
        .application_icon(ICON)
        .developer_name("dixonSolutions")
        .version(ratclick_core::VERSION)
        .website("https://github.com/dixonSolutions/ratclick")
        .issue_url("https://github.com/dixonSolutions/ratclick/issues")
        .license_type(gtk::License::Gpl30)
        .comments("A configurable auto-clicker for GNOME.")
        .build();
    about.present(Some(parent.as_ref()));
}
