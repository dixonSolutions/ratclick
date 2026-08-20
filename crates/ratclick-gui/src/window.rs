//! The main RatClick window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use ratclick_core::accel::Accel;
use ratclick_core::config::{Button, ClickMode, Config, Effect, ShortcutBackend, MIN_CPM};
use ratclick_core::{ipc, shortcut};

use crate::bridge::{Bridge, Cmd, Snapshot};
use crate::capture;
use crate::privilege::{self, KeydAction};

/// The GUI owns this name on the session bus (GtkApplication does it for
/// us); the daemon deliberately uses a different one. See `ipc::BUS_NAME`.
pub const APP_ID: &str = ratclick_core::ipc::APP_ID;
const ICON: &str = "io.github.dixonsolutions.RatClick";

const BUTTONS: &[(Button, &str)] = &[
    (Button::Left, "Left"),
    (Button::Right, "Right"),
    (Button::Middle, "Middle"),
];

/// Order shown in the drop-downs; index maps straight to the combo row.
const EFFECTS: &[Effect] = Effect::ALL;

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

    // Effects
    effects_enabled: adw::SwitchRow,
    effect_on: adw::ComboRow,
    effect_off: adw::ComboRow,
    effect_rows: adw::PreferencesGroup,

    // Service
    service_row: adw::ActionRow,
    service_button: gtk::Button,

    // Unsaved-changes bar
    save_bar: gtk::ActionBar,
    save_badge: gtk::Label,
    changes_label: gtk::Label,

    /// The config as edited in the widgets, not yet necessarily on disk.
    config: RefCell<Config>,
    /// The config as last written to disk — the baseline `config` is diffed
    /// against to decide whether there are unsaved changes.
    saved: RefCell<Config>,
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

    // ---- Effects --------------------------------------------------------
    let effects_group = adw::PreferencesGroup::builder()
        .title("Toggle effect")
        .description(
            "A flourish drawn at the pointer when clicking starts or stops. \
             Needs the RatClick GNOME Shell extension.",
        )
        .build();

    let effects_enabled = adw::SwitchRow::builder()
        .title("Show an effect when toggling")
        .build();
    effects_group.add(&effects_enabled);

    // Separate group so both drop-downs can be hidden together when the master
    // switch is off, without leaving a stranded header.
    let effect_rows = adw::PreferencesGroup::new();
    let effect_on = adw::ComboRow::builder()
        .title("When clicking starts")
        .subtitle("Drawn in green")
        .build();
    effect_on.set_model(Some(&string_list(EFFECTS.iter().map(|e| e.label()))));
    let effect_off = adw::ComboRow::builder()
        .title("When clicking stops")
        .subtitle("Drawn in red")
        .build();
    effect_off.set_model(Some(&string_list(EFFECTS.iter().map(|e| e.label()))));
    effect_rows.add(&effect_on);
    effect_rows.add(&effect_off);

    page.add(&effects_group);
    page.add(&effect_rows);

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

    // ---- Danger zone ----------------------------------------------------
    let danger_group = adw::PreferencesGroup::builder()
        .title("Danger zone")
        .build();
    let reset_row = adw::ActionRow::builder()
        .title("Reset RatClick")
        .subtitle("Delete all settings and unbind the shortcut")
        .build();
    let reset_button = gtk::Button::builder()
        .label("Reset…")
        .valign(gtk::Align::Center)
        .build();
    reset_button.add_css_class("destructive-action");
    reset_row.add_suffix(&reset_button);
    danger_group.add(&reset_row);
    page.add(&danger_group);

    // ---- Unsaved-changes bar --------------------------------------------
    // Hidden until something is actually edited; wired up in `wire_up`.
    install_badge_css();
    let save_badge = gtk::Label::new(Some("0"));
    save_badge.add_css_class("rc-change-badge");

    let changes_label = gtk::Label::new(Some("unsaved change"));
    changes_label.add_css_class("dim-label");

    let changes_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    changes_box.append(&save_badge);
    changes_box.append(&changes_label);

    let discard_button = gtk::Button::with_label("Discard");
    let save_button = gtk::Button::with_label("Save & Restart");
    save_button.add_css_class("suggested-action");

    let save_bar = gtk::ActionBar::new();
    save_bar.pack_start(&changes_box);
    save_bar.pack_end(&save_button);
    save_bar.pack_end(&discard_button);
    save_bar.set_revealed(false);

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
    toolbar.add_bottom_bar(&save_bar);
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
        effects_enabled: effects_enabled.clone(),
        effect_on: effect_on.clone(),
        effect_off: effect_off.clone(),
        effect_rows,
        service_row,
        service_button: service_button.clone(),
        save_bar,
        save_badge,
        changes_label,
        saved: RefCell::new(config.clone()),
        config: RefCell::new(config),
        bridge,
        loading: Cell::new(false),
    });

    ui.load_into_widgets();
    ui.wire_up(
        &set_button,
        &clear_button,
        &save_button,
        &discard_button,
        &reset_button,
    );
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

/// A small pill-shaped counter for the unsaved-changes bar. Libadwaita has no
/// stock "badge" widget, so this is the one bit of custom CSS in the app.
fn install_badge_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        ".rc-change-badge {
            background-color: @accent_bg_color;
            color: @accent_fg_color;
            font-weight: bold;
            font-size: 0.85em;
            min-width: 1.4em;
            min-height: 1.4em;
            padding: 0 0.35em;
            border-radius: 999px;
        }",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
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

        self.effects_enabled.set_active(cfg.effects.enabled);
        self.effect_on.set_selected(
            EFFECTS
                .iter()
                .position(|e| *e == cfg.effects.on)
                .unwrap_or(0) as u32,
        );
        self.effect_off.set_selected(
            EFFECTS
                .iter()
                .position(|e| *e == cfg.effects.off)
                .unwrap_or(0) as u32,
        );
        self.effect_rows.set_visible(cfg.effects.enabled);

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

    /// Write `config` to disk and restart the running click loop with it, if
    /// any is running. Used both by settings that apply immediately (the
    /// shortcut section) and by the explicit Save button.
    ///
    /// Returns whether the save actually happened, so callers that show their
    /// own success toast do not report success on failure.
    fn save(self: &Rc<Self>) -> bool {
        if self.loading.get() {
            return false;
        }
        let mut cfg = self.config.borrow_mut();
        cfg.setup_complete = true;
        for note in cfg.normalise() {
            self.toast(&note);
        }
        if let Err(e) = cfg.save() {
            self.toast(&format!("Could not save settings: {e}"));
            return false;
        }
        let saved = cfg.clone();
        drop(cfg);
        *self.saved.borrow_mut() = saved;
        self.bridge.send(Cmd::Reload);
        self.mark_dirty();
        true
    }

    /// Number of top-level settings that differ between the edited config and
    /// what is actually on disk.
    fn changed_field_count(&self) -> usize {
        let saved = self.saved.borrow();
        let cur = self.config.borrow();
        [
            cur.click.cpm != saved.click.cpm,
            cur.click.button != saved.click.button,
            cur.click.mode != saved.click.mode,
            cur.click.duration_minutes != saved.click.duration_minutes,
            cur.start_clicking_on_launch != saved.start_clicking_on_launch,
            cur.shortcut.backend != saved.shortcut.backend,
            cur.shortcut.bindings != saved.shortcut.bindings,
        ]
        .into_iter()
        .filter(|changed| *changed)
        .count()
    }

    /// Recompute the unsaved-changes badge and show or hide the save bar.
    /// Called after every edit instead of an immediate `save()` for settings
    /// that should wait for an explicit Save.
    fn mark_dirty(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        let n = self.changed_field_count();
        self.save_badge.set_label(&n.to_string());
        self.changes_label.set_label(if n == 1 {
            "unsaved change"
        } else {
            "unsaved changes"
        });
        self.save_bar.set_revealed(n > 0);
    }

    /// Throw away edits since the last save and put the widgets back to
    /// match what is on disk.
    fn discard(self: &Rc<Self>) {
        *self.config.borrow_mut() = self.saved.borrow().clone();
        self.load_into_widgets();
        self.mark_dirty();
        self.toast("Changes discarded");
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    // ---- signals ---------------------------------------------------------

    fn wire_up(
        self: &Rc<Self>,
        set_button: &gtk::Button,
        clear_button: &gtk::Button,
        save_button: &gtk::Button,
        discard_button: &gtk::Button,
        reset_button: &gtk::Button,
    ) {
        let ui = self.clone();
        self.effects_enabled.connect_active_notify(move |row| {
            if ui.loading.get() {
                return;
            }
            let on = row.is_active();
            ui.config.borrow_mut().effects.enabled = on;
            ui.effect_rows.set_visible(on);
            ui.save();
        });

        for (row, is_on) in [(&self.effect_on, true), (&self.effect_off, false)] {
            let ui = self.clone();
            row.connect_selected_notify(move |row| {
                if ui.loading.get() {
                    return;
                }
                let Some(effect) = EFFECTS.get(row.selected() as usize).copied() else {
                    return;
                };
                {
                    let mut cfg = ui.config.borrow_mut();
                    if is_on {
                        cfg.effects.on = effect;
                    } else {
                        cfg.effects.off = effect;
                    }
                }
                ui.save();
            });
        }

        let ui = self.clone();
        reset_button.connect_clicked(move |_| ui.confirm_reset());

        // These four settings are batched: edits only update the in-memory
        // config and the unsaved-changes bar, they do not touch disk or the
        // running service until Save is pressed.
        let ui = self.clone();
        self.cpm.connect_value_notify(move |row| {
            if ui.loading.get() {
                return;
            }
            ui.config.borrow_mut().click.cpm = row.value().round() as u32;
            ui.mark_dirty();
        });

        let ui = self.clone();
        self.button.connect_selected_notify(move |row| {
            if ui.loading.get() {
                return;
            }
            let idx = row.selected() as usize;
            if let Some((b, _)) = BUTTONS.get(idx) {
                ui.config.borrow_mut().click.button = *b;
                ui.mark_dirty();
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
            ui.mark_dirty();
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
                ui.mark_dirty();
            });
        }

        let ui = self.clone();
        self.autostart.connect_active_notify(move |row| {
            if ui.loading.get() {
                return;
            }
            ui.config.borrow_mut().start_clicking_on_launch = row.is_active();
            ui.mark_dirty();
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

        let ui = self.clone();
        save_button.connect_clicked(move |_| {
            if ui.save() {
                ui.toast("Saved — the service has been restarted with your changes");
            }
        });

        let ui = self.clone();
        discard_button.connect_clicked(move |_| {
            ui.discard();
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
            Err(e)
                if cfg.shortcut.backend == ShortcutBackend::Keyd && !shortcut::keyd::is_root() =>
            {
                // keyd lives in /etc, so this needs a privileged helper.
                self.escalate_keyd(&e.to_string());
            }
            Err(e) => self.toast(&format!("Could not set the shortcut: {e}")),
        }
    }

    /// Confirm, then undo everything RatClick has done to this account.
    fn confirm_reset(self: &Rc<Self>) {
        let dialog = adw::AlertDialog::new(
            Some("Reset RatClick?"),
            Some(
                "This deletes your settings — click rate, run length, effects — and unbinds the \
                 toggle shortcut from every backend.\n\nRatClick itself stays installed, and the \
                 next time you open it you will be taken through setup again. This cannot be \
                 undone.",
            ),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("reset", "Reset Everything");
        dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
        // Cancel on Escape and as the default, so a stray Return cannot wipe
        // somebody's configuration.
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let ui = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "reset" {
                return;
            }
            ui.perform_reset();
        });
        dialog.present(Some(&self.window));
    }

    fn perform_reset(self: &Rc<Self>) {
        // Stop clicking first: leaving the engine running with no config to
        // describe it would be the one genuinely confusing outcome.
        self.bridge.send(Cmd::StopDaemon);

        let (done, failed) = ratclick_core::reset_all();
        for line in &failed {
            self.toast(line);
        }

        // keyd lives in /etc, so its removal needs root; offer that rather than
        // silently leaving a live shortcut behind.
        if !shortcut::keyd::installed().is_empty() {
            self.escalate_keyd_action(KeydAction::Clear, "the keyd shortcut still needs removing");
        }

        *self.config.borrow_mut() = Config::default();
        self.load_into_widgets();

        if failed.is_empty() {
            self.toast(&format!("RatClick has been reset ({} item(s))", done.len()));
        }
    }

    /// Ask for admin rights and re-run the keyd installation as root.
    /// Ask for admin rights and run one privileged `ratclick` subcommand.
    fn escalate_keyd(self: &Rc<Self>, why: &str) {
        self.escalate_keyd_action(KeydAction::Apply, why);
    }

    fn escalate_keyd_action(self: &Rc<Self>, action: KeydAction, why: &str) {
        let action_label = match action {
            KeydAction::Apply => "install",
            KeydAction::Clear => "remove",
        };
        let dialog = adw::AlertDialog::new(
            Some("Administrator access needed"),
            Some(&format!(
                "keyd shortcuts live in /etc/keyd, so RatClick needs to run one command as \
                 root to {action_label} them.\n\n{why}"
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
            match privilege::run_keyd_action(action) {
                Ok(()) => {
                    ui.refresh_shortcut_row();
                    ui.toast(match action {
                        KeydAction::Apply => "keyd shortcut installed",
                        KeydAction::Clear => "keyd shortcut removed",
                    });
                }
                Err(error) => ui.toast(&format!("Could not update keyd: {error:#}")),
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
        .license_type(gtk::License::MitX11)
        .comments("A desktop-independent auto-clicker for Linux.")
        .build();
    about.present(Some(parent.as_ref()));
}
