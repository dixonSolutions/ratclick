# Installing RatClick

Signed apt and dnf repositories are published at
<https://dixonsolutions.github.io/ratclick>. Both are rebuilt by the release
workflow, and both keep every version that has ever been released, so
downgrading works too.

- [Debian / Ubuntu (apt)](#debian--ubuntu-apt)
- [Fedora / RHEL (dnf)](#fedora--rhel-dnf)
- [A single .deb or .rpm](#a-single-deb-or-rpm)
- [The GNOME Shell extension](#the-gnome-shell-extension)
- [After installing](#after-installing)
- [Uninstalling](#uninstalling)
- [What gets installed where](#what-gets-installed-where)

## Debian / Ubuntu (apt)

```bash
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://dixonsolutions.github.io/ratclick/ratclick.asc | sudo tee /etc/apt/keyrings/ratclick.asc >/dev/null
echo "deb [signed-by=/etc/apt/keyrings/ratclick.asc] https://dixonsolutions.github.io/ratclick/apt stable main" | sudo tee /etc/apt/sources.list.d/ratclick.list
sudo apt update && sudo apt install ratclick
```

The repository is signed with both `InRelease` and a detached `Release.gpg`, so
any apt from Debian 10 / Ubuntu 20.04 onwards is happy with it.

If you would rather use the dearmoured key — some tooling insists on a keyring
in binary form — it is published alongside the armoured one:

```bash
sudo curl -fsSL -o /etc/apt/keyrings/ratclick.gpg https://dixonsolutions.github.io/ratclick/ratclick.gpg
echo "deb [signed-by=/etc/apt/keyrings/ratclick.gpg] https://dixonsolutions.github.io/ratclick/apt stable main" | sudo tee /etc/apt/sources.list.d/ratclick.list
```

## Fedora / RHEL (dnf)

```bash
sudo curl -fsSL -o /etc/yum.repos.d/ratclick.repo https://dixonsolutions.github.io/ratclick/ratclick.repo
sudo rpm --import https://dixonsolutions.github.io/ratclick/ratclick.asc
sudo dnf install ratclick
```

That `.repo` file is:

```ini
[ratclick]
name=RatClick
baseurl=https://dixonsolutions.github.io/ratclick/dnf
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=https://dixonsolutions.github.io/ratclick/ratclick.asc
metadata_expire=6h
```

Both checks are on: `gpgcheck` verifies each package, `repo_gpgcheck` verifies
the repository metadata against `repodata/repomd.xml.asc`. Write it by hand
instead of downloading it if you prefer.

## A single .deb or .rpm

Every [release](https://github.com/dixonSolutions/ratclick/releases) attaches
the packages directly, along with a `SHA256SUMS` file covering all of them.

```bash
# Debian / Ubuntu
sudo apt install ./ratclick_<version>-1_amd64.deb

# Fedora
sudo dnf install ./ratclick-<version>-1.x86_64.rpm
```

Installing this way means no automatic updates — the repositories above are the
better option unless you are deliberately pinning a version.

## The GNOME Shell extension

The packages already install the extension to
`/usr/share/gnome-shell/extensions/ratclick@dixonsolutions.github.io/`. Log out
and back in so the Shell notices it, then turn it on:

```bash
gnome-extensions enable ratclick@dixonsolutions.github.io
```

Releases also attach `ratclick@dixonsolutions.github.io.shell-extension.zip`
for a per-user install without the rest of the package:

```bash
gnome-extensions install --force ratclick@dixonsolutions.github.io.shell-extension.zip
```

## After installing

```bash
ratclick gui        # open the window
ratclick doctor     # check the install and report anything broken
man 1 ratclick      # every subcommand
```

Clicking works by creating a virtual pointer through `/dev/uinput`, which is
root-only by default. The packaged udev rule
(`/usr/lib/udev/rules.d/60-ratclick-uinput.rules`) gives the `input` group write
access to it. Add yourself to that group and log in again:

```bash
sudo usermod -aG input $USER
```

`ratclick doctor` reports whether this is already in place, so run that first if
you are not sure.

## Uninstalling

```bash
sudo apt remove ratclick        # or: sudo apt purge ratclick
sudo dnf remove ratclick
```

To also drop the repository:

```bash
# Debian / Ubuntu
sudo rm /etc/apt/sources.list.d/ratclick.list /etc/apt/keyrings/ratclick.asc

# Fedora
sudo rm /etc/yum.repos.d/ratclick.repo
```

Your settings live in `~/.config/ratclick/config.toml` and are left alone;
`ratclick config reset` removes them before you uninstall if you want a clean
sweep.

## What gets installed where

| Path | What it is |
| --- | --- |
| `/usr/bin/ratclick` | the command line |
| `/usr/bin/ratclick-gui` | the libadwaita window |
| `/usr/libexec/ratclick/ratclickd` | the background service |
| `/usr/libexec/ratclick/ratclick-keyd-dispatch` | bridges a root keyd binding into your session |
| `/usr/lib/systemd/user/ratclick.service` | systemd user unit |
| `/usr/share/dbus-1/services/io.github.dixonsolutions.RatClick.service` | D-Bus session activation |
| `/usr/lib/udev/rules.d/60-ratclick-uinput.rules` | `input` group access to `/dev/uinput` |
| `/usr/share/applications/io.github.dixonsolutions.RatClick.desktop` | app launcher entry |
| `/usr/share/icons/hicolor/**/apps/io.github.dixonsolutions.RatClick.*` | icons |
| `/usr/share/metainfo/io.github.dixonsolutions.RatClick.metainfo.xml` | AppStream data for software centres |
| `/usr/share/gnome-shell/extensions/ratclick@dixonsolutions.github.io/` | the Shell extension |
| `/usr/share/man/man1/ratclick.1.gz` | the man page |
| `/usr/share/doc/ratclick/README.md` | this project's README |

## Building the packages yourself

```bash
# Debian / Ubuntu
sudo apt install libadwaita-1-dev libgtk-4-dev libudev-dev scdoc
cargo install cargo-deb cargo-generate-rpm

scripts/build-packages.sh          # writes dist/*.deb and dist/*.rpm
```

`scripts/build-pages.sh <site-dir> dist` builds the signed apt and dnf
repositories out of those packages, exactly as the release workflow does. It
signs with whichever secret key `GNUPGHOME` holds; set `GPG_KEY_ID` to choose
one and `GPG_PASSPHRASE` if it has a passphrase.
