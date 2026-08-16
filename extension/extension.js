/* extension.js
 *
 * RatClick — GNOME Shell front-end for the RatClick auto-clicker daemon.
 *
 * Provides a Quick Settings toggle, a panel indicator that is visible only
 * while the clicker is armed, and a global keyboard shortcut.
 */

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as QuickSettings from 'resource:///org/gnome/shell/ui/quickSettings.js';
import {Extension, gettext as _} from 'resource:///org/gnome/shell/extensions/extension.js';

import {RatClickClient} from './dbus.js';

const TOGGLE_KEY = 'toggle-clicking';

/* Shipped with the RatClick application. Falls back to a stock icon so the
 * extension is usable before/without the application's icon theme files. */
const ICON_NAME = 'io.github.dixonsolutions.RatClick-symbolic';
const FALLBACK_ICON_NAME = 'input-mouse-symbolic';

/* A GThemedIcon with several names resolves to the first one the icon theme
 * actually has, which gives us the fallback for free. */
const createIcon = () =>
    Gio.ThemedIcon.new_from_names([ICON_NAME, FALLBACK_ICON_NAME]);

/**
 * Format a duration for the quick settings header.
 *
 * @param {number} totalSeconds - seconds remaining
 * @returns {string} e.g. "9:07" or "1:02:03"
 */
function formatDuration(totalSeconds) {
    const seconds = Math.max(0, Math.trunc(totalSeconds));
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = seconds % 60;
    const pad = n => `${n}`.padStart(2, '0');
    return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/** Launch the main RatClick application, which owns the click settings. */
function launchApplication() {
    try {
        const appInfo = Gio.AppInfo.create_from_commandline(
            'ratclick-gui', 'RatClick', Gio.AppInfoCreateFlags.NONE);
        appInfo.launch([], global.create_app_launch_context(0, -1));
    } catch (error) {
        console.warn(`RatClick: could not launch ratclick-gui: ${error.message}`);
    }
}

const RatClickToggle = GObject.registerClass(
class RatClickToggle extends QuickSettings.QuickMenuToggle {
    _init(client) {
        super._init({
            title: _('RatClick'),
            gicon: createIcon(),
            /* The daemon owns the state, so we never flip `checked` locally:
             * it is only ever set from Status()/StateChanged. */
            toggleMode: false,
        });

        this._client = client;

        this.menu.setHeader(createIcon(), _('RatClick'));

        this._settingsItem = this.menu.addAction(
            _('Settings'), () => {
                Main.panel.statusArea.quickSettings.menu.close();
                launchApplication();
            });

        this._clickedId = this.connect('clicked',
            () => this._client.toggle().catch(() => {}));

        this.connect('destroy', () => this._onDestroy());

        this.sync();
    }

    /** Mirror the cached daemon state into the widget. */
    sync() {
        if (!this._client)
            return;

        const {available, running, cpm, mode, remainingSeconds} = this._client;

        this.checked = running;
        this.reactive = true;

        let subtitle;
        if (!available)
            subtitle = _('Not running');
        else if (!running)
            subtitle = _('Idle');
        else if (mode === 'timed')
            subtitle = formatDuration(remainingSeconds);
        else
            subtitle = _('Clicking');

        this.subtitle = subtitle;

        let headerSubtitle;
        if (!available) {
            headerSubtitle = _('RatClick is not installed or not running');
        } else {
            const rate = cpm > 0
                /* Translators: %d is a number of clicks per minute. */
                ? `${cpm} ${_('CPM')}`
                : _('Unknown rate');
            if (running && mode === 'timed') {
                /* Translators: %s is a remaining time such as "2:31". */
                headerSubtitle = `${rate} · ${formatDuration(remainingSeconds)} ${_('left')}`;
            } else if (running) {
                headerSubtitle = `${rate} · ${_('endless')}`;
            } else {
                headerSubtitle = `${rate} · ${_('stopped')}`;
            }
        }

        this.menu.setHeader(createIcon(), _('RatClick'), headerSubtitle);
    }

    _onDestroy() {
        if (this._clickedId) {
            this.disconnect(this._clickedId);
            this._clickedId = 0;
        }
        this._settingsItem = null;
        this._client = null;
    }
});

const RatClickIndicator = GObject.registerClass(
class RatClickIndicator extends QuickSettings.SystemIndicator {
    _init(client) {
        super._init();

        this._client = client;
        this._timerId = 0;

        /* Panel icon: shown only while the clicker is actually armed.
         * SystemIndicator hides itself automatically when no child is visible. */
        this._panelIcon = this._addIndicator();
        this._panelIcon.gicon = createIcon();
        this._panelIcon.visible = false;

        this._toggle = new RatClickToggle(client);
        this.quickSettingsItems.push(this._toggle);

        this._stateChangedId = client.connect('state-changed', () => this._sync());
        this._availabilityId = client.connect('availability-changed', () => this._sync());

        this.connect('destroy', () => this._onDestroy());

        this._sync();
    }

    _sync() {
        if (!this._client || !this._toggle)
            return;

        this._panelIcon.visible = this._client.available && this._client.running;
        this._toggle.sync();
        this._updateTimer();
    }

    /** Run a 1 Hz source only while a timed run is counting down. */
    _updateTimer() {
        const wanted = this._client.available &&
            this._client.running &&
            this._client.mode === 'timed' &&
            this._client.remainingSeconds > 0;

        if (wanted && !this._timerId) {
            this._timerId = GLib.timeout_add_seconds(
                GLib.PRIORITY_DEFAULT, 1, () => this._onTick());
        } else if (!wanted && this._timerId) {
            GLib.Source.remove(this._timerId);
            this._timerId = 0;
        }
    }

    _onTick() {
        if (!this._client || !this._toggle) {
            this._timerId = 0;
            return GLib.SOURCE_REMOVE;
        }

        this._client.tick();
        this._toggle.sync();

        if (this._client.remainingSeconds <= 0) {
            this._timerId = 0;
            /* The daemon will confirm the stop via StateChanged. */
            return GLib.SOURCE_REMOVE;
        }

        return GLib.SOURCE_CONTINUE;
    }

    _onDestroy() {
        if (this._timerId) {
            GLib.Source.remove(this._timerId);
            this._timerId = 0;
        }
        if (this._client) {
            if (this._stateChangedId) {
                this._client.disconnect(this._stateChangedId);
                this._stateChangedId = 0;
            }
            if (this._availabilityId) {
                this._client.disconnect(this._availabilityId);
                this._availabilityId = 0;
            }
            this._client = null;
        }
        this._toggle = null;
        this._panelIcon = null;
    }
});

export default class RatClickExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._client = new RatClickClient();
        this._indicator = new RatClickIndicator(this._client);

        Main.panel.statusArea.quickSettings.addExternalIndicator(this._indicator);

        Main.wm.addKeybinding(
            TOGGLE_KEY,
            this._settings,
            Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
            Shell.ActionMode.ALL,
            () => this._client?.toggle().catch(() => {}));
        this._keybindingAdded = true;

        /* Fire and forget: the shell must not wait on the bus. */
        this._client.open().catch(
            error => console.warn(`RatClick: ${error.message}`));
    }

    disable() {
        if (this._keybindingAdded) {
            Main.wm.removeKeybinding(TOGGLE_KEY);
            this._keybindingAdded = false;
        }

        if (this._indicator) {
            this._indicator.quickSettingsItems.forEach(item => item.destroy());
            this._indicator.quickSettingsItems.length = 0;
            this._indicator.destroy();
            this._indicator = null;
        }

        if (this._client) {
            this._client.close();
            this._client = null;
        }

        this._settings = null;
    }
}
