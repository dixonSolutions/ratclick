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
import GObject from 'gi://GObject';

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
    <signal name="StateChanged">
      <arg type="b" name="running"/>
      <arg type="u" name="remaining_seconds"/>
    </signal>
  </interface>
</node>`;

const RatClickProxy = Gio.DBusProxy.makeProxyWrapper(RATCLICK_IFACE);

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
    },
}, class RatClickClient extends GObject.Object {
    _init() {
        super._init();

        this._proxy = null;
        this._cancellable = new Gio.Cancellable();
        this._nameOwnerId = 0;
        this._stateChangedId = 0;
        this._closed = false;

        /* Cached daemon state. Valid only while `available` is true. */
        this.running = false;
        this.cpm = 0;
        this.button = '';
        this.mode = 'endless';
        this.remainingSeconds = 0;
        this.clicks = 0;
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
                this.running = running;
                this.remainingSeconds = remainingSeconds;
                this.emit('state-changed');
                /* A state change may also mean a new mode/cpm; refresh. */
                this.refresh().catch(() => {});
            });

        this._nameOwnerId = proxy.connect('notify::g-name-owner', () => {
            if (!this.available)
                this._resetState();
            this.emit('availability-changed');
            if (this.available)
                this.refresh().catch(() => {});
        });

        this.emit('availability-changed');

        /* Only poll if somebody already owns the name: merely enabling the
         * extension must not D-Bus-activate the daemon behind the user's back. */
        if (this.available)
            await this.refresh();
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
        this.cpm = 0;
        this.button = '';
        this.mode = 'endless';
        this.remainingSeconds = 0;
        this.clicks = 0;
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
