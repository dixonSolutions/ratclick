/* prefs.js
 *
 * Preferences for the RatClick shell extension.
 *
 * The extension only owns the shell integration, so the single setting here is
 * the global toggle shortcut. Click rate, mouse button, run duration and the
 * choice of toggle effect belong to the RatClick application itself.
 *
 * The effect previews below are *not* settings. They exist because this
 * process cannot draw on the stage — it is not the shell — so each button asks
 * the running extension to play one over D-Bus. If the extension is disabled
 * there is nothing to ask and the button says so.
 */

import Adw from 'gi://Adw';
import Gdk from 'gi://Gdk';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Gtk from 'gi://Gtk';

import {
    ExtensionPreferences,
    gettext as _,
} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

const TOGGLE_KEY = 'toggle-clicking';

/* Must match extension.js. */
const PREVIEW_BUS_NAME = 'io.github.dixonsolutions.RatClick.ShellExtension';
const PREVIEW_OBJECT_PATH = '/io/github/dixonsolutions/RatClick/ShellExtension';
const PREVIEW_IFACE_NAME = 'io.github.dixonsolutions.RatClick.ShellExtension1';

/* 'none' is deliberately absent: there would be nothing to see. */
const PREVIEWABLE = ['ripple', 'pulse', 'logo'];

/* Navigation keys that must stay usable for moving around the UI. */
const FORBIDDEN_KEYVALS = [
    Gdk.KEY_Home, Gdk.KEY_Left, Gdk.KEY_Up, Gdk.KEY_Right, Gdk.KEY_Down,
    Gdk.KEY_Page_Up, Gdk.KEY_Page_Down, Gdk.KEY_End, Gdk.KEY_Tab,
    Gdk.KEY_KP_Enter, Gdk.KEY_Return, Gdk.KEY_Mode_switch,
];

/**
 * Reject combinations that would make the desktop unusable, mirroring
 * gnome-control-center's keyboard panel. Unmodified (or Shift-only)
 * alphanumerics are rejected; unmodified function keys are allowed.
 *
 * @param {number} mask - normalized modifier mask
 * @param {number} keycode - hardware keycode
 * @param {number} keyval - key symbol
 * @returns {boolean} whether the combination may be bound
 */
function isValidBinding(mask, keycode, keyval) {
    if ((mask === 0 || mask === Gdk.ModifierType.SHIFT_MASK) && keycode !== 0) {
        if ((keyval >= Gdk.KEY_a && keyval <= Gdk.KEY_z) ||
            (keyval >= Gdk.KEY_A && keyval <= Gdk.KEY_Z) ||
            (keyval >= Gdk.KEY_0 && keyval <= Gdk.KEY_9) ||
            (keyval >= Gdk.KEY_kana_fullstop && keyval <= Gdk.KEY_semivoicedsound) ||
            (keyval >= Gdk.KEY_Arabic_comma && keyval <= Gdk.KEY_Arabic_sukun) ||
            (keyval >= Gdk.KEY_Serbian_dje && keyval <= Gdk.KEY_Cyrillic_HARDSIGN) ||
            (keyval >= Gdk.KEY_Greek_ALPHAaccent && keyval <= Gdk.KEY_Greek_omega) ||
            (keyval >= Gdk.KEY_hebrew_doublelowline && keyval <= Gdk.KEY_hebrew_taf) ||
            (keyval >= Gdk.KEY_Thai_kokai && keyval <= Gdk.KEY_Thai_lekkao) ||
            (keyval >= Gdk.KEY_Hangul_Kiyeog && keyval <= Gdk.KEY_Hangul_J_YeorinHieuh) ||
            (keyval === Gdk.KEY_space && mask === 0) ||
            FORBIDDEN_KEYVALS.includes(keyval))
            return false;
    }
    return true;
}

/**
 * `Gtk.accelerator_valid` rejects Tab outright, but <Ctrl>Tab is fine.
 *
 * @param {number} mask - normalized modifier mask
 * @param {number} keyval - key symbol
 * @returns {boolean} whether GTK considers the accelerator usable
 */
function isValidAccel(mask, keyval) {
    return Gtk.accelerator_valid(keyval, mask) ||
        (keyval === Gdk.KEY_Tab && mask !== 0);
}

/**
 * An Adw.ActionRow that captures a keyboard shortcut into a GSettings `as` key.
 *
 * Uses a plain Adw.Window rather than Adw.Dialog: on libadwaita 1.9 an
 * Adw.Dialog breaks Gtk.EventControllerKey when the dialog is reopened.
 */
const ShortcutRow = GObject.registerClass({
    Properties: {
        'shortcut': GObject.ParamSpec.string(
            'shortcut', 'shortcut', 'shortcut',
            GObject.ParamFlags.READWRITE, ''),
    },
}, class ShortcutRow extends Adw.ActionRow {
    _init(settings, key, title, subtitle) {
        super._init({
            title,
            subtitle,
            activatable: true,
        });

        this._settings = settings;
        this._key = key;
        this._editor = null;

        this._label = new Gtk.ShortcutLabel({
            disabled_text: _('Disabled'),
            valign: Gtk.Align.CENTER,
        });

        this._clearButton = new Gtk.Button({
            icon_name: 'edit-clear-symbolic',
            tooltip_text: _('Clear shortcut'),
            valign: Gtk.Align.CENTER,
            css_classes: ['flat'],
        });
        this._clearButton.connect('clicked', () => this._save());

        const box = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL,
            spacing: 6,
            valign: Gtk.Align.CENTER,
        });
        box.append(this._label);
        box.append(this._clearButton);
        this.add_suffix(box);

        this.bind_property('shortcut', this._label, 'accelerator',
            GObject.BindingFlags.SYNC_CREATE);

        this._settingsChangedId = this._settings.connect(
            `changed::${this._key}`, () => this._load());
        this._load();

        this.connect('activated', () => this._openEditor());
        this.connect('destroy', () => this._onDestroy());
    }

    _load() {
        const strv = this._settings.get_strv(this._key);
        this.shortcut = strv.length > 0 ? strv[0] : '';
        this._clearButton.sensitive = this.shortcut !== '';
    }

    _openEditor() {
        const content = new Adw.StatusPage({
            title: _('Press the new shortcut'),
            description: _('Press Esc to cancel, or Backspace to clear the shortcut.'),
            icon_name: 'preferences-desktop-keyboard-shortcuts-symbolic',
        });

        this._editor = new Adw.Window({
            modal: true,
            transient_for: this.get_root(),
            width_request: 480,
            height_request: 320,
            content,
        });

        const controller = new Gtk.EventControllerKey();
        controller.connect('key-pressed',
            (_c, keyval, keycode, state) => this._onKeyPressed(keyval, keycode, state));
        this._editor.add_controller(controller);

        this._editor.connect('destroy', () => this._restoreSystemShortcuts());
        this._editor.connect('close-request', () => {
            this._restoreSystemShortcuts();
            return false;
        });

        this._editor.present();
        /* The surface only exists once the window is on screen. Without this,
         * the compositor swallows <Super> combos and they never reach us. */
        this._inhibitSystemShortcuts();
    }

    _editorSurface() {
        if (!this._editor)
            return null;
        /* On X11 the toplevel that actually holds the grab is the parent. */
        const isWayland = GLib.getenv('XDG_SESSION_TYPE') === 'wayland';
        const widget = isWayland ? this._editor : this.get_root();
        return widget?.get_surface() ?? null;
    }

    _inhibitSystemShortcuts() {
        try {
            this._editorSurface()?.inhibit_system_shortcuts(null);
        } catch {
            /* Best effort only; capture still works for most combinations. */
        }
    }

    _restoreSystemShortcuts() {
        try {
            this._editorSurface()?.restore_system_shortcuts();
        } catch {
            /* Nothing to do. */
        }
    }

    _onKeyPressed(keyval, keycode, state) {
        let mask = state & Gtk.accelerator_get_default_mod_mask();
        mask &= ~Gdk.ModifierType.LOCK_MASK;

        if (mask === 0 && keyval === Gdk.KEY_Escape) {
            this._closeEditor();
            return Gdk.EVENT_STOP;
        }

        if (keyval === Gdk.KEY_BackSpace && mask === 0) {
            this._save();
            return Gdk.EVENT_STOP;
        }

        if (!isValidBinding(mask, keycode, keyval) || !isValidAccel(mask, keyval))
            return Gdk.EVENT_STOP;

        this._save(keyval, keycode, mask);
        return Gdk.EVENT_STOP;
    }

    _save(keyval, keycode, mask) {
        const accelerator = keyval || keycode
            ? Gtk.accelerator_name_with_keycode(null, keyval, keycode, mask)
            : '';

        /* An empty list, not a list holding an empty string. */
        this._settings.set_strv(this._key, accelerator ? [accelerator] : []);
        this._closeEditor();
    }

    _closeEditor() {
        if (!this._editor)
            return;
        this._restoreSystemShortcuts();
        this._editor.destroy();
        this._editor = null;
    }

    _onDestroy() {
        this._closeEditor();
        if (this._settingsChangedId) {
            this._settings.disconnect(this._settingsChangedId);
            this._settingsChangedId = 0;
        }
    }
});

/**
 * Ask the running extension to draw an effect at the pointer.
 *
 * @param {Adw.PreferencesWindow} window - used to report failures
 * @param {string} effect - effect name
 * @param {boolean} on - true for the start variant, false for the stop one
 */
function requestPreview(window, effect, on) {
    Gio.DBus.session.call(
        PREVIEW_BUS_NAME,
        PREVIEW_OBJECT_PATH,
        PREVIEW_IFACE_NAME,
        'PreviewEffect',
        new GLib.Variant('(sb)', [effect, on]),
        null,
        Gio.DBusCallFlags.NONE,
        -1,
        null,
        (connection, result) => {
            try {
                connection.call_finish(result);
            } catch (error) {
                /* Almost always "the extension is switched off". Say that
                 * rather than leaking a bus error at the user. */
                window.add_toast(new Adw.Toast({
                    title: _('Enable the RatClick extension to see a preview.'),
                }));
                console.debug(`RatClick: preview failed: ${error.message}`);
            }
        });
}

/**
 * One row with a start-preview and a stop-preview button, so the two colours
 * can be compared directly.
 *
 * @param {Adw.PreferencesWindow} window - passed through to `requestPreview`
 * @param {string} title - user-visible effect name
 * @param {string} effect - effect name on the wire
 * @returns {Adw.ActionRow} the row
 */
function makePreviewRow(window, title, effect) {
    const row = new Adw.ActionRow({title});

    const box = new Gtk.Box({
        orientation: Gtk.Orientation.HORIZONTAL,
        spacing: 6,
        valign: Gtk.Align.CENTER,
    });

    for (const [label, on] of [[_('Start'), true], [_('Stop'), false]]) {
        const button = new Gtk.Button({
            label,
            valign: Gtk.Align.CENTER,
        });
        button.connect('clicked', () => requestPreview(window, effect, on));
        box.append(button);
    }

    row.add_suffix(box);
    return row;
}

export default class RatClickPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings();

        const page = new Adw.PreferencesPage({
            title: _('General'),
            icon_name: 'input-mouse-symbolic',
        });

        const shortcutGroup = new Adw.PreferencesGroup({
            title: _('Keyboard Shortcut'),
            description: _('Starts or stops the auto-clicker from anywhere.'),
        });
        shortcutGroup.add(new ShortcutRow(
            settings,
            TOGGLE_KEY,
            _('Toggle Clicking'),
            _('Click the row, then press the key combination you want.')));
        page.add(shortcutGroup);

        const aboutGroup = new Adw.PreferencesGroup({
            title: _('Click Settings'),
        });

        const appRow = new Adw.ActionRow({
            title: _('Configure RatClick'),
            subtitle: _('Click rate, mouse button and run duration are set in the RatClick app, not here.'),
        });
        appRow.add_prefix(new Gtk.Image({
            icon_name: 'io.github.dixonsolutions.RatClick-symbolic',
            valign: Gtk.Align.CENTER,
        }));

        const hint = new Gtk.Label({
            label: '<tt>ratclick gui</tt>',
            use_markup: true,
            valign: Gtk.Align.CENTER,
            css_classes: ['dim-label'],
            selectable: true,
        });
        appRow.add_suffix(hint);
        aboutGroup.add(appRow);
        page.add(aboutGroup);

        const previewGroup = new Adw.PreferencesGroup({
            title: _('Toggle Effects'),
            description: _('Which effect plays, and whether any plays at all, is set in the RatClick app. Preview them here — each is drawn at the pointer, green for starting and red for stopping.'),
        });
        for (const effect of PREVIEWABLE) {
            const title = {
                ripple: _('Ripple'),
                pulse: _('Pulse'),
                logo: _('Logo'),
            }[effect];
            previewGroup.add(makePreviewRow(window, title, effect));
        }
        page.add(previewGroup);

        window.add(page);
    }
}
