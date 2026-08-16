# RatClick GNOME Shell extension

The extension is the desktop front-end for the RatClick auto-clicker. It adds:

- a **Quick Settings toggle** ("RatClick") that starts and stops clicking and
  shows the current rate, mode and remaining time;
- a **panel indicator** that appears *only while the clicker is armed*, so you
  can always tell at a glance that RatClick is running;
- a **global keyboard shortcut** to toggle clicking from anywhere.

It does not click anything by itself. All of the actual work happens in the
`ratclick` daemon, which the extension talks to over the session bus. Click
rate, mouse button and run duration are configured in the RatClick application
(`ratclick gui`), not in the extension.

- **UUID:** `ratclick@dixonsolutions.github.io`
- **Supported shells:** GNOME 49 and 50
- **Not published on extensions.gnome.org** — install it from the release zip.

---

## Install

### 1. Download the release zip

Grab `ratclick@dixonsolutions.github.io.shell-extension.zip` from the
[Releases page](https://github.com/dixonSolutions/ratclick/releases).

### 2. Install it

```bash
gnome-extensions install --force ratclick@dixonsolutions.github.io.shell-extension.zip
```

This unpacks the extension into
`~/.local/share/gnome-shell/extensions/ratclick@dixonsolutions.github.io/` and
compiles its GSettings schema for you.

<details>
<summary>Installing by hand instead</summary>

```bash
UUID=ratclick@dixonsolutions.github.io
mkdir -p ~/.local/share/gnome-shell/extensions/$UUID
unzip -o $UUID.shell-extension.zip -d ~/.local/share/gnome-shell/extensions/$UUID
glib-compile-schemas ~/.local/share/gnome-shell/extensions/$UUID/schemas/
```

The `glib-compile-schemas` step is required — without it the extension cannot
read its own settings and will fail to enable.
</details>

### 3. Restart GNOME Shell

**On Wayland (the default on Ubuntu 26.04) you must log out and log back in.**
There is no way to restart the shell in place on Wayland; `Alt`+`F2` → `r` only
works on X11, which GNOME 49 and later no longer support for the session.

### 4. Enable it

```bash
gnome-extensions enable ratclick@dixonsolutions.github.io
```

or turn it on in the **Extensions** app. Confirm it came up cleanly:

```bash
gnome-extensions info ratclick@dixonsolutions.github.io
# ...
#   State: ACTIVE
```

### 5. Set the keyboard shortcut (optional)

```bash
gnome-extensions prefs ratclick@dixonsolutions.github.io
```

Click the **Toggle Clicking** row, press the combination you want, and it is
saved immediately. `Backspace` clears it, `Esc` cancels. No shortcut is bound
by default.

---

## Using it

Open Quick Settings (click the status area at the top right). The **RatClick**
tile shows the current state:

| Tile subtitle | Meaning |
| --- | --- |
| `Not running` | The daemon is not installed, or could not be started |
| `Idle` | Daemon is up, clicking is stopped |
| `Clicking` | Running in endless mode |
| `2:31` | Running in timed mode, counting down |

Click the tile to start or stop. Click the arrow to open the menu, which shows
`<rate> CPM · <mode>` and a **Settings** item that launches the RatClick app.

While clicking is active, a mouse icon appears in the top panel. It disappears
as soon as clicking stops.

The daemon is D-Bus activatable: using the toggle or the shortcut starts it on
demand. Simply enabling the extension does **not** start the daemon.

---

## Uninstall

```bash
gnome-extensions uninstall ratclick@dixonsolutions.github.io
```

Then log out and back in.

---

## Reporting problems

Please open an issue at
<https://github.com/dixonSolutions/ratclick/issues>.

Include the output of all of the following:

```bash
# 1. Versions
gnome-shell --version
gnome-extensions --version
echo "$XDG_SESSION_TYPE"

# 2. Extension state (look for State: and any error text)
gnome-extensions info ratclick@dixonsolutions.github.io

# 3. Shell log, filtered to this extension
journalctl --user -b --no-pager | grep -iE 'ratclick|JS ERROR' | tail -50

# 4. Is the daemon reachable?
gdbus call --session \
  -d io.github.dixonsolutions.RatClick \
  -o /io/github/dixonsolutions/RatClick \
  -m io.github.dixonsolutions.RatClick1.Status
```

### Common problems

**The tile says "Not running" and never changes.**
Step 4 above will tell you why. If it reports `ServiceUnknown` or
`Spawn.ChildExited`, the daemon is not installed correctly — reinstall the
RatClick package. The extension is designed to sit there harmlessly rather than
spam your log when the daemon is missing.

**The extension will not enable / `State: ERROR`.**
Almost always a missing compiled schema. Run the `glib-compile-schemas` command
from the manual-install section, then log out and back in.

**`State: OUT_OF_DATE`.**
The zip was built for a different GNOME version. Check `gnome-shell --version`
against the supported list at the top of this page.

**The keyboard shortcut does nothing.**
Another application or GNOME itself may already own that combination. Pick a
different one in the extension's preferences. Shortcuts are also inactive while
the screen is locked.

---

## For maintainers

### Building the release zip

```bash
gnome-extensions pack --force --extra-source=dbus.js extension/
```

**`--extra-source=dbus.js` is mandatory.** `gnome-extensions pack` only bundles
a fixed set of names (`extension.js`, `prefs.js`, `metadata.json`,
`stylesheet.css`, `schemas/`, `locale/`); without the flag the zip is missing
`dbus.js` and the extension fails to load with an import error.

Verify the bundle before publishing:

```bash
unzip -l ratclick@dixonsolutions.github.io.shell-extension.zip
# must list: metadata.json, extension.js, prefs.js, dbus.js,
#            schemas/org.gnome.shell.extensions.ratclick.gschema.xml
```

### Testing without the daemon

`extension/tests/mock-daemon.py` implements the full
`io.github.dixonsolutions.RatClick1` interface with fake state, so the
extension can be exercised without the Rust daemon. It needs PyGObject
(`apt install python3-gi`).

```bash
python3 extension/tests/mock-daemon.py --cpm 600 --mode timed --duration 30
```

It logs every incoming call as `CALL <Method>` and every transition as
`STATE running=... remaining=...`, which makes it easy to assert on the daemon
side of an interaction in CI. `--start` begins clicking immediately and
`--replace` takes the bus name from another instance.

### Running a throwaway shell

GNOME 50 has **no nested backend** — mutter dropped `MetaBackendX11Nested`
along with X11 session support in GNOME 49. `gnome-shell --wayland
--wayland-display=...` therefore tries to become a *native* display server and
dies with `Failed to take control of the session: EBUSY`. Use headless mode
instead:

```bash
dbus-run-session -- gnome-shell --headless --virtual-monitor 1280x800
```

Run it with isolated `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and
`XDG_STATE_HOME` so it cannot touch your real session, extension set or dconf
database.
