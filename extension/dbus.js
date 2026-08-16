/* dbus.js
 *
 * Thin async wrapper around the RatClick daemon's session-bus interface.
 *
 * The daemon is D-Bus activatable, so calling a method on the well-known name
 * starts it. It may however be legitimately absent (application not installed),
 * so every call is defensive: failures are swallowed and reported through the
 * `availability-changed` signal rather than thrown at the shell.
 */

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';

import {EFFECT_NAMES} from './effects.js';

// Not the app ID: that name belongs to the GUI's GtkApplication.
export const BUS_NAME = 'io.github.dixonsolutions.RatClick.Daemon';
export const OBJECT_PATH = '/io/github/dixonsolutions/RatClick/Daemon';
export const INTERFACE_NAME = 'io.github.dixonsolutions.RatClick1';

const RATCLICK_IFACE = `
<node>
  <interface name="io.github.dixonsolutions.RatClick1">
    <method name="Start"/>
    <method name="Stop"/>
    <method name="Toggle">
      <arg type="b" direction="out" name="running"/>
    </method>
    <method name="Status">
      <arg type="b" direction="out" name="running"/>
      <arg type="u" direction="out" name="cpm"/>
      <arg type="s" direction="out" name="button"/>
      <arg type="s" direction="out" name="mode"/>
      <arg type="u" direction="out" name="remaining_seconds"/>
      <arg type="t" direction="out" name="clicks"/>
    </method>
    <method name="ReloadConfig"/>
    <method name="Quit"/>
    <property name="Running" type="b" access="read"/>
    <property name="Cpm" type="u" access="read"/>
    <property name="Mode" type="s" access="read"/>
    <property name="RemainingSeconds" type="u" access="read"/>
    <property name="EffectsEnabled" type="b" access="read"/>
    <property name="EffectOn" type="s" access="read"/>
    <property name="EffectOff" type="s" access="read"/>
    <signal name="StateChanged">
      <arg type="b" name="running"/>
      <arg type="u" name="remaining_seconds"/>
    </signal>
  </interface>
</node>`;

const RatClickProxy = Gio.DBusProxy.makeProxyWrapper(RATCLICK_IFACE);

const PROPERTIES_IFACE = 'org.freedesktop.DBus.Properties';

/* The visual-effect half of the interface. Daemons older than the effects
 * feature simply do not export these, which must read as "effects off". */
const EFFECT_PROPERTIES = ['EffectsEnabled', 'EffectOn', 'EffectOff'];

const NO_EFFECTS = Object.freeze({
    effectsEnabled: false,
    effectOn: 'none',
    effectOff: 'none',
});

/**
 * True for errors that just mean "the daemon isn't installed / isn't running".
 * Those are expected and must not produce log noise.
 *
 * @param {Error} error - the error to classify
 * @returns {boolean} whether the error is an expected absence
 */
function _isExpectedAbsence(error) {
    return error.matches?.(Gio.DBusError, Gio.DBusError.SERVICE_UNKNOWN) ||
        error.matches?.(Gio.DBusError, Gio.DBusError.NAME_HAS_NO_OWNER) ||
        error.matches?.(Gio.DBusError, Gio.DBusError.NO_REPLY) ||
        error.matches?.(Gio.DBusError, Gio.DBusError.TIMED_OUT) ||
        error.matches?.(Gio.DBusError, Gio.DBusError.SPAWN_SERVICE_NOT_FOUND) ||
        error.matches?.(Gio.DBusError, Gio.DBusError.SPAWN_FAILED) ||
        error.matches?.(Gio.DBusError, Gio.DBusError.SPAWN_EXEC_FAILED) ||
        error.matches?.(Gio.DBusError, Gio.DBusError.SPAWN_CHILD_EXITED) ||
        error.matches?.(Gio.DBusError, Gio.DBusError.SPAWN_FILE_INVALID);
}

/**
 * True if the operation was cancelled during disable(); always silent.
 *
 * @param {Error} error - the error to classify
 * @returns {boolean} whether the error is a cancellation
 */
function _isCancelled(error) {
    return error.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED);
}

export const RatClickClient = GObject.registerClass({
    Signals: {
        /* The cached daemon state changed. */
        'state-changed': {},
        /* The daemon appeared on, or vanished from, the bus. */
        'availability-changed': {},
        /* The daemon announced that clicking actually started or stopped.
         *
         * Emitted only for a real edge on StateChanged, never for the
         * repeated identical states a timed run emits while counting down,
         * and never for state merely discovered by polling Status(). It is
         * what drives the visual effects, which must fire once per toggle. */
        'run-transition': {param_types: [GObject.TYPE_BOOLEAN]},
    },
}, class RatClickClient extends GObject.Object {
    _init() {
        super._init();

        this._proxy = null;
        this._cancellable = new Gio.Cancellable();
        this._nameOwnerId = 0;
        this._stateChangedId = 0;
        this._propertiesChangedId = 0;
        this._closed = false;

        /* Cached daemon state. Valid only while `available` is true. */
        this.running = false;
        this.cpm = 0;
        this.button = '';
        this.mode = 'endless';
        this.remainingSeconds = 0;
        this.clicks = 0;

        /* Effect configuration, owned by the daemon's config file. */
        Object.assign(this, NO_EFFECTS);
        this._effectsKnown = false;

        /* Mirror of `running` as last *announced*, so that state we learned
         * some other way cannot be mistaken for a transition later. */
        this._lastAnnouncedRunning = false;
    }

    /** @returns {boolean} whether the daemon currently owns the bus name */
    get available() {
        return !!this._proxy?.g_name_owner;
    }

    /**
     * Build the proxy and take an initial state reading. Never rejects.
     *
     * @returns {Promise<void>} resolves once the proxy is up (or failed)
     */
    async open() {
        if (this._closed || this._proxy)
            return;

        let proxy;
        try {
            proxy = await RatClickProxy.newAsync(
                Gio.DBus.session, BUS_NAME, OBJECT_PATH, this._cancellable);
        } catch (error) {
            if (!this._isSilent(error))
                console.warn(`RatClick: could not create D-Bus proxy: ${error.message}`);
            return;
        }

        /* disable() may have run while we were waiting. */
        if (this._closed) {
            proxy.run_dispose();
            return;
        }

        this._proxy = proxy;

        this._stateChangedId = proxy.connectSignal('StateChanged',
            (_proxy, _sender, [running, remainingSeconds]) => {
                /* A timed run re-announces the same `running` about once a
                 * second while it counts down. Only the edges are interesting
                 * to anything with a side effect. */
                const isTransition = running !== this._lastAnnouncedRunning;

                this.running = running;
                this._lastAnnouncedRunning = running;
                this.remainingSeconds = remainingSeconds;
                this.emit('state-changed');
                if (isTransition)
                    this._announceTransition(running);
                /* A state change may also mean a new mode/cpm; refresh. */
                this.refresh().catch(() => {});
            });

        /* The effect properties change whenever the user edits them in the
         * RatClick app and the daemon reloads its config, so they are re-read
         * on every announcement rather than cached once here. */
        this._propertiesChangedId = proxy.connect('g-properties-changed',
            (_proxy, changed, invalidated) => {
                const touched = [
                    ...Object.keys(changed.deepUnpack()),
                    ...invalidated,
                ];
                if (touched.some(name => EFFECT_PROPERTIES.includes(name)))
                    this._refreshEffects();
            });

        this._nameOwnerId = proxy.connect('notify::g-name-owner', () => {
            if (!this.available)
                this._resetState();
            this.emit('availability-changed');
            if (this.available) {
                this.refresh().catch(() => {});
                this._refreshEffects();
            }
        });

        this.emit('availability-changed');

        /* Only poll if somebody already owns the name: merely enabling the
         * extension must not D-Bus-activate the daemon behind the user's back. */
        if (this.available) {
            this._refreshEffects();
            await this.refresh();
        }
    }

    /**
     * Emit `run-transition`, first making sure the effect configuration is
     * actually known.
     *
     * The very first toggle after the extension starts is also what D-Bus-
     * activates the daemon, so the name owner, the effect properties and the
     * StateChanged signal all land at once. Without this the first start of a
     * session would silently draw nothing.
     *
     * @param {boolean} running - the state just announced
     */
    _announceTransition(running) {
        if (this._effectsKnown)
            this.emit('run-transition', running);
        else
            this._refreshEffects(() => this.emit('run-transition', running));
    }

    /**
     * Re-read the three effect properties straight from the daemon.
     *
     * Deliberately a fresh Properties.GetAll rather than the proxy's cache: a
     * daemon that predates the effects feature does not export them at all,
     * and one that gained them at runtime would otherwise never be noticed.
     * Anything unreadable or unrecognised degrades to "no effects".
     *
     * @param {?Function} [onDone] - called once the cache has been updated,
     *   whether the read succeeded or not
     */
    _refreshEffects(onDone = null) {
        if (!this._proxy || this._closed || !this.available) {
            this._applyEffects(null);
            onDone?.();
            return;
        }

        Gio.DBus.session.call(
            BUS_NAME,
            OBJECT_PATH,
            PROPERTIES_IFACE,
            'GetAll',
            new GLib.Variant('(s)', [INTERFACE_NAME]),
            new GLib.VariantType('(a{sv})'),
            Gio.DBusCallFlags.NONE,
            -1,
            this._cancellable,
            (connection, result) => {
                if (this._closed)
                    return;
                try {
                    const [props] = connection.call_finish(result).recursiveUnpack();
                    this._applyEffects(props);
                } catch (error) {
                    if (!this._isSilent(error))
                        console.debug(`RatClick: GetAll() failed: ${error.message}`);
                    this._applyEffects(null);
                }
                onDone?.();
            });
    }

    /**
     * @param {?object} props - the unpacked Properties.GetAll dictionary, or
     *   null when it could not be read
     */
    _applyEffects(props) {
        /* A null dictionary means the read failed, not that the daemon said
         * "no effects" — do not treat it as authoritative. */
        this._effectsKnown = props !== null;

        const name = value =>
            (typeof value === 'string' && EFFECT_NAMES.includes(value))
                ? value
                : 'none';

        const next = props?.EffectsEnabled === true
            ? {
                effectsEnabled: true,
                effectOn: name(props.EffectOn),
                effectOff: name(props.EffectOff),
            }
            : NO_EFFECTS;

        if (this.effectsEnabled === next.effectsEnabled &&
            this.effectOn === next.effectOn &&
            this.effectOff === next.effectOff)
            return;

        Object.assign(this, next);
        this.emit('state-changed');
    }

    /**
     * Re-read the full daemon state via Status(). Never rejects.
     *
     * @returns {Promise<void>} resolves when the cache is updated
     */
    async refresh() {
        if (!this._proxy || this._closed || !this.available)
            return;

        try {
            const [running, cpm, button, mode, remainingSeconds, clicks] =
                await this._proxy.StatusAsync();

            if (this._closed)
                return;

            this.running = running;
            /* Polled state is not an announcement: adopt it silently, so that
             * the daemon's next StateChanged is judged against what is really
             * happening rather than against a stale `false`. */
            this._lastAnnouncedRunning = running;
            this.cpm = cpm;
            this.button = button;
            this.mode = mode;
            this.remainingSeconds = remainingSeconds;
            this.clicks = clicks;
            this.emit('state-changed');
        } catch (error) {
            this._report(error, 'Status');
        }
    }

    /**
     * Flip the daemon's running state. Activates the daemon if needed.
     *
     * @returns {Promise<void>} resolves once the call completed
     */
    async toggle() {
        await this._call('Toggle');
    }

    /**
     * Start clicking. Activates the daemon if needed.
     *
     * @returns {Promise<void>} resolves once the call completed
     */
    async start() {
        await this._call('Start');
    }

    /**
     * Stop clicking.
     *
     * @returns {Promise<void>} resolves once the call completed
     */
    async stop() {
        await this._call('Stop');
    }

    /**
     * Ask the daemon to re-read its configuration file.
     *
     * @returns {Promise<void>} resolves once the call completed
     */
    async reloadConfig() {
        await this._call('ReloadConfig');
    }

    /**
     * Invoke a no-argument method and refresh the cached state afterwards.
     *
     * @param {string} name - bare method name, e.g. 'Toggle'
     * @returns {Promise<void>} resolves once the call completed
     */
    async _call(name) {
        if (!this._proxy || this._closed)
            return;

        try {
            await this._proxy[`${name}Async`]();
        } catch (error) {
            this._report(error, name);
            return;
        }

        if (!this._closed)
            await this.refresh();
    }

    /** Locally count down one second between StateChanged updates. */
    tick() {
        if (this.remainingSeconds > 0)
            this.remainingSeconds--;
    }

    _resetState() {
        this.running = false;
        /* A daemon that vanished did not announce a stop, and a crash must not
         * fire the stop effect. */
        this._lastAnnouncedRunning = false;
        this.cpm = 0;
        this.button = '';
        this.mode = 'endless';
        this.remainingSeconds = 0;
        this.clicks = 0;
        Object.assign(this, NO_EFFECTS);
        this._effectsKnown = false;
    }

    _isSilent(error) {
        return _isCancelled(error) || _isExpectedAbsence(error);
    }

    _report(error, what) {
        if (this._isSilent(error))
            console.debug(`RatClick: ${what}() unavailable: ${error.message}`);
        else
            console.warn(`RatClick: ${what}() failed: ${error.message}`);
    }

    /** Tear everything down. Safe to call more than once. */
    close() {
        if (this._closed)
            return;
        this._closed = true;

        this._cancellable.cancel();

        if (this._proxy) {
            if (this._stateChangedId) {
                this._proxy.disconnectSignal(this._stateChangedId);
                this._stateChangedId = 0;
            }
            if (this._propertiesChangedId) {
                this._proxy.disconnect(this._propertiesChangedId);
                this._propertiesChangedId = 0;
            }
            if (this._nameOwnerId) {
                this._proxy.disconnect(this._nameOwnerId);
                this._nameOwnerId = 0;
            }
            this._proxy.run_dispose();
            this._proxy = null;
        }

        this._cancellable = null;
    }
});
