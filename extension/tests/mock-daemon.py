#!/usr/bin/env python3
"""Mock RatClick daemon.

Implements the io.github.dixonsolutions.RatClick1 interface on the session bus
with entirely fake state, so the GNOME Shell extension can be exercised without
the real Rust daemon. Intended for manual testing and CI.

Requires PyGObject (Debian/Ubuntu: python3-gi).

Usage:
    python3 mock-daemon.py [--cpm 600] [--mode endless|timed] [--duration 30]
                           [--button left] [--start] [--replace]

Every incoming method call is logged to stdout as a single line prefixed with
"CALL", and every state transition as "STATE", so a test harness can assert on
the daemon side of an interaction:

    CALL Toggle
    STATE running=True remaining=0
"""

import argparse
import sys

try:
    import gi
except ImportError:  # pragma: no cover
    sys.exit("mock-daemon.py needs PyGObject (apt install python3-gi)")

gi.require_version("GLib", "2.0")
gi.require_version("Gio", "2.0")
from gi.repository import GLib, Gio  # noqa: E402

BUS_NAME = "io.github.dixonsolutions.RatClick.Daemon"
OBJECT_PATH = "/io/github/dixonsolutions/RatClick/Daemon"
INTERFACE_NAME = "io.github.dixonsolutions.RatClick1"

INTROSPECTION_XML = """
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
</node>
"""


def log(*parts):
    print(*parts, flush=True)


class MockDaemon:
    def __init__(self, args):
        self.cpm = args.cpm
        self.button = args.button
        self.mode = args.mode
        self.duration = args.duration

        self.running = False
        self.remaining = 0
        self.clicks = 0

        self._connection = None
        self._registration_id = 0
        self._tick_id = 0
        self._loop = GLib.MainLoop()
        self._autostart = args.start
        self._replace = args.replace

    # ------------------------------------------------------------------ bus

    def run(self):
        flags = Gio.BusNameOwnerFlags.ALLOW_REPLACEMENT
        if self._replace:
            flags |= Gio.BusNameOwnerFlags.REPLACE

        owner_id = Gio.bus_own_name(
            Gio.BusType.SESSION,
            BUS_NAME,
            flags,
            self._on_bus_acquired,
            self._on_name_acquired,
            self._on_name_lost,
        )
        try:
            self._loop.run()
        finally:
            Gio.bus_unown_name(owner_id)

    def _on_bus_acquired(self, connection, _name):
        self._connection = connection
        node_info = Gio.DBusNodeInfo.new_for_xml(INTROSPECTION_XML)
        # register_object() is deprecated in PyGObject; prefer the closure
        # variants where available so the mock stays warning-free.
        register = (
            getattr(connection, "register_object_with_closures2", None)
            or getattr(connection, "register_object_with_closures", None)
            or connection.register_object
        )
        self._registration_id = register(
            OBJECT_PATH,
            node_info.interfaces[0],
            self._handle_method_call,
            self._handle_get_property,
            None,
        )

    def _on_name_acquired(self, _connection, name):
        log(f"READY name={name} cpm={self.cpm} mode={self.mode}")
        if self._autostart:
            self._start()

    def _on_name_lost(self, _connection, name):
        log(f"NAMELOST name={name}")
        self._loop.quit()

    # -------------------------------------------------------------- methods

    def _handle_method_call(
        self, _conn, _sender, _path, _iface, method, params, invocation
    ):
        log(f"CALL {method}")

        if method == "Start":
            self._start()
            invocation.return_value(None)
        elif method == "Stop":
            self._stop()
            invocation.return_value(None)
        elif method == "Toggle":
            self._stop() if self.running else self._start()
            invocation.return_value(GLib.Variant("(b)", (self.running,)))
        elif method == "Status":
            invocation.return_value(
                GLib.Variant(
                    "(bussut)",
                    (
                        self.running,
                        self.cpm,
                        self.button,
                        self.mode,
                        self.remaining,
                        self.clicks,
                    ),
                )
            )
        elif method == "ReloadConfig":
            invocation.return_value(None)
        elif method == "Quit":
            invocation.return_value(None)
            GLib.idle_add(self._loop.quit)
        else:
            invocation.return_error_literal(
                Gio.dbus_error_quark(),
                Gio.DBusError.UNKNOWN_METHOD,
                f"No such method {method}",
            )

    def _handle_get_property(self, _conn, _sender, _path, _iface, prop):
        return {
            "Running": lambda: GLib.Variant("b", self.running),
            "Cpm": lambda: GLib.Variant("u", self.cpm),
            "Mode": lambda: GLib.Variant("s", self.mode),
            "RemainingSeconds": lambda: GLib.Variant("u", self.remaining),
        }[prop]()

    # ---------------------------------------------------------------- state

    def _start(self):
        if self.running:
            return
        self.running = True
        self.remaining = self.duration if self.mode == "timed" else 0
        self._emit_state()
        if self.mode == "timed":
            self._tick_id = GLib.timeout_add_seconds(1, self._on_tick)

    def _stop(self):
        if not self.running:
            return
        self.running = False
        self.remaining = 0
        self._cancel_tick()
        self._emit_state()

    def _cancel_tick(self):
        if self._tick_id:
            GLib.source_remove(self._tick_id)
            self._tick_id = 0

    def _on_tick(self):
        self.clicks += max(1, self.cpm // 60)
        self.remaining = max(0, self.remaining - 1)
        if self.remaining == 0:
            self._tick_id = 0
            self.running = False
            self._emit_state()
            return GLib.SOURCE_REMOVE
        # Re-announce every 5s so the extension can resync its countdown.
        if self.remaining % 5 == 0:
            self._emit_state()
        return GLib.SOURCE_CONTINUE

    def _emit_state(self):
        log(f"STATE running={self.running} remaining={self.remaining}")
        if not self._connection:
            return

        self._connection.emit_signal(
            None,
            OBJECT_PATH,
            INTERFACE_NAME,
            "StateChanged",
            GLib.Variant("(bu)", (self.running, self.remaining)),
        )
        self._connection.emit_signal(
            None,
            OBJECT_PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            GLib.Variant(
                "(sa{sv}as)",
                (
                    INTERFACE_NAME,
                    {
                        "Running": GLib.Variant("b", self.running),
                        "RemainingSeconds": GLib.Variant("u", self.remaining),
                        "Cpm": GLib.Variant("u", self.cpm),
                        "Mode": GLib.Variant("s", self.mode),
                    },
                    [],
                ),
            ),
        )


def main():
    parser = argparse.ArgumentParser(description="Mock RatClick daemon")
    parser.add_argument("--cpm", type=int, default=600)
    parser.add_argument("--button", default="left")
    parser.add_argument("--mode", choices=("endless", "timed"), default="endless")
    parser.add_argument("--duration", type=int, default=30)
    parser.add_argument("--start", action="store_true", help="begin clicking at startup")
    parser.add_argument("--replace", action="store_true", help="take over the bus name")
    MockDaemon(parser.parse_args()).run()


if __name__ == "__main__":
    main()
