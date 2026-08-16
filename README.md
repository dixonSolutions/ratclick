# RatClick

An auto-clicker for GNOME. Pick a click rate, a mouse button and how long the
run should last, then start it from the window, from a global keyboard
shortcut, or from the command line.

The clicks come from a virtual pointer created through the kernel `uinput`
device, so they behave the same under Wayland and X11.

## Install

Packages for Debian/Ubuntu and Fedora, and the apt/dnf repositories that carry
them, are described in [docs/install.md](docs/install.md).

## What you get

| Path | What it is |
| --- | --- |
| `/usr/bin/ratclick` | the command line |
| `/usr/bin/ratclick-gui` | the libadwaita window |
| `/usr/libexec/ratclick/ratclickd` | the background service |
| `/usr/lib/systemd/user/ratclick.service` | systemd user unit for the service |
| `/usr/share/gnome-shell/extensions/ratclick@dixonsolutions.github.io/` | Quick Settings toggle and panel indicator |

## Using it

```bash
ratclick gui        # open the window
ratclick start      # start clicking
ratclick stop       # stop clicking
ratclick toggle     # what the global shortcut runs
ratclick status     # what is it doing right now
ratclick doctor     # check the install and report anything broken
```

`ratclick config set cpm 900`, `ratclick config set button right` and
`ratclick config set duration 1h30m` change the settings without opening the
window. `man 1 ratclick` documents every subcommand.

## Permissions

The daemon needs write access to `/dev/uinput`. The packaged udev rule grants
that to the `input` group; add yourself and log in again:

```bash
sudo usermod -aG input $USER
```

`ratclick doctor` reports whether this is already in place.

## Building from source

Needs Rust 1.82 or newer plus the GTK 4 and libadwaita development packages.

```bash
# Debian/Ubuntu
sudo apt install libadwaita-1-dev libgtk-4-dev libudev-dev

cargo build --release --workspace
```

`scripts/build-packages.sh` turns that into a `.deb` and an `.rpm` under
`dist/`.

## Layout

| Directory | Contents |
| --- | --- |
| `crates/ratclick-core` | configuration, accelerators, shortcut backends |
| `crates/ratclick-daemon` | click engine and the session D-Bus service |
| `crates/ratclick-cli` | the `ratclick` command |
| `crates/ratclick-gui` | the libadwaita front end |
| `extension/` | GNOME Shell extension |
| `data/` | units, D-Bus and desktop files, udev rule, man page, AppStream |
| `packaging/` | `.deb` and `.rpm` metadata |
| `scripts/` | package and repository builders |

## Licence

GPL-3.0-or-later.
