//! The "press a key combination" dialog.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use gtk::glib;
use ratclick_core::accel::{Accel, Modifiers};
use ratclick_core::shortcut;

/// Show the capture dialog. `on_done` is called with the chosen accelerator, or
/// not at all if the user cancels.
pub fn present<F>(parent: &impl IsA<gtk::Widget>, on_done: F)
where
    F: Fn(Accel) + 'static,
{
    let dialog = adw::Dialog::builder()
        .title("Set Shortcut")
        .content_width(420)
        .can_close(true)
        .build();

    let status = adw::StatusPage::builder()
        .icon_name("preferences-desktop-keyboard-shortcuts-symbolic")
        .title("Press a key combination")
        .description(
            "Hold a modifier such as Super, Ctrl or Alt. Press Esc to cancel, Backspace to clear.",
        )
        .build();

    let feedback = gtk::Label::builder()
        .wrap(true)
        .justify(gtk::Justification::Center)
        .margin_top(6)
        .build();
    feedback.add_css_class("dim-label");

    let confirm = gtk::Button::builder()
        .label("Use This Shortcut")
        .halign(gtk::Align::Center)
        .margin_top(12)
        .sensitive(false)
        .build();
    confirm.add_css_class("suggested-action");
    confirm.add_css_class("pill");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&feedback);
    content.append(&confirm);
    status.set_child(Some(&content));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&status));
    dialog.set_child(Some(&toolbar));

    let chosen: Rc<RefCell<Option<Accel>>> = Rc::new(RefCell::new(None));

    let keys = gtk::EventControllerKey::new();
    // Capture phase, so the combination is seen before any widget can treat it
    // as an activation or a mnemonic.
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let chosen = chosen.clone();
        let feedback = feedback.clone();
        let confirm = confirm.clone();
        let status = status.clone();
        let dialog = dialog.clone();
        move |_, keyval, _keycode, state| {
            // A bare modifier press is the user still assembling the chord.
            if is_modifier(keyval) {
                return glib::Propagation::Stop;
            }

            if keyval == gdk::Key::Escape && state.is_empty() {
                dialog.close();
                return glib::Propagation::Stop;
            }
            if keyval == gdk::Key::BackSpace && state.is_empty() {
                *chosen.borrow_mut() = None;
                status.set_title("Press a key combination");
                feedback.set_text("");
                confirm.set_sensitive(false);
                return glib::Propagation::Stop;
            }

            match accel_from_event(keyval, state) {
                Some(accel) => match accel.validate() {
                    Ok(()) => {
                        status.set_title(&accel.to_display());
                        describe_conflicts(&accel, &feedback);
                        *chosen.borrow_mut() = Some(accel);
                        confirm.set_sensitive(true);
                    }
                    Err(e) => {
                        status.set_title(&accel.to_display());
                        feedback.set_markup(&format!(
                            "<span foreground=\"#e01b24\">{}</span>",
                            glib::markup_escape_text(&e.to_string())
                        ));
                        *chosen.borrow_mut() = None;
                        confirm.set_sensitive(false);
                    }
                },
                None => {
                    feedback.set_text("RatClick cannot bind that key.");
                    *chosen.borrow_mut() = None;
                    confirm.set_sensitive(false);
                }
            }
            glib::Propagation::Stop
        }
    });
    dialog.add_controller(keys);

    confirm.connect_clicked({
        let chosen = chosen.clone();
        let dialog = dialog.clone();
        move |_| {
            let picked = chosen.borrow().clone();
            if let Some(accel) = picked {
                dialog.close();
                on_done(accel);
            }
        }
    });

    // On Wayland the compositor swallows combinations like Super+S before the
    // app ever sees them, so ask it to stand down for as long as this dialog is
    // up. Without this, half the useful shortcuts are uncapturable.
    dialog.connect_realize(|d| {
        if let Some(toplevel) = toplevel_of(d) {
            toplevel.inhibit_system_shortcuts(None::<&gdk::ButtonEvent>);
        }
    });
    dialog.connect_closed(|d| {
        if let Some(toplevel) = toplevel_of(d) {
            toplevel.restore_system_shortcuts();
        }
    });

    dialog.present(Some(parent));
}

fn toplevel_of(widget: &impl IsA<gtk::Widget>) -> Option<gdk::Toplevel> {
    widget
        .as_ref()
        .native()
        .and_then(|n| n.surface())
        .and_then(|s| s.downcast::<gdk::Toplevel>().ok())
}

fn describe_conflicts(accel: &Accel, label: &gtk::Label) {
    let conflicts = shortcut::conflicts(accel);
    if conflicts.is_empty() {
        label.set_markup("<span foreground=\"#2ec27e\">This combination is free.</span>");
        return;
    }
    let list = conflicts
        .iter()
        .map(|c| format!("• {}", glib::markup_escape_text(&c.describe())))
        .collect::<Vec<_>>()
        .join("\n");
    label.set_markup(&format!(
        "<span foreground=\"#e5a50a\">Already used by:</span>\n{list}\n\
         <small>RatClick will take it.</small>"
    ));
}

fn is_modifier(key: gdk::Key) -> bool {
    matches!(
        key,
        gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::Hyper_L
            | gdk::Key::Hyper_R
            | gdk::Key::ISO_Level3_Shift
            | gdk::Key::Caps_Lock
            | gdk::Key::Num_Lock
    )
}

/// Turn a GDK key event into a RatClick accelerator.
fn accel_from_event(keyval: gdk::Key, state: gdk::ModifierType) -> Option<Accel> {
    let mut mods = Modifiers::NONE;
    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        mods |= Modifiers::CONTROL;
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        mods |= Modifiers::SHIFT;
    }
    if state.contains(gdk::ModifierType::ALT_MASK) {
        mods |= Modifiers::ALT;
    }
    if state.contains(gdk::ModifierType::SUPER_MASK) {
        mods |= Modifiers::SUPER;
    }

    // Use the *unshifted* name where we can: with Shift held, GDK reports `C`
    // for the C key and `exclam` for 1, but the binding must be stored as the
    // physical key plus a Shift modifier or it will never match.
    let name = keyval
        .to_lower()
        .name()
        .map(|s| s.to_string())
        .or_else(|| keyval.name().map(|s| s.to_string()))?;

    // `Accel::parse` also does the GTK-keysym-to-evdev-name translation, and
    // rejects keys we have no way to express to keyd.
    Accel::parse(&format!("{}{}", gtk_mod_prefix(mods), name)).ok()
}

fn gtk_mod_prefix(mods: Modifiers) -> String {
    let mut s = String::new();
    if mods.contains(Modifiers::CONTROL) {
        s.push_str("<Control>");
    }
    if mods.contains(Modifiers::ALT) {
        s.push_str("<Alt>");
    }
    if mods.contains(Modifiers::SHIFT) {
        s.push_str("<Shift>");
    }
    if mods.contains(Modifiers::SUPER) {
        s.push_str("<Super>");
    }
    s
}
