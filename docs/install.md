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
curl -fsSL -o /tmp/ratclick.asc https://dixonsolutions.github.io/ratclick/ratclick.asc \
  && grep -q 'BEGIN PGP PUBLIC KEY' /tmp/ratclick.asc \
  && sudo install -m 0644 /tmp/ratclick.asc /etc/apt/keyrings/ratclick.asc \
  && echo "deb [signed-by=/etc/apt/keyrings/ratclick.asc] https://dixonsolutions.github.io/ratclick/apt stable main" \
     | sudo tee /etc/apt/sources.list.d/ratclick.list >/dev/null \
  && sudo apt update \
  && sudo apt install ratclick
```

Every step is chained with `&&` deliberately. The key is downloaded to a
temporary file first and the sources file is written only after that key is in
place, so a download that fails — Pages unavailable, a proxy in the way, no
`/etc/apt/keyrings` on an older release — leaves the system exactly as it was.
The obvious-looking `curl … | sudo tee` version cannot do that: a pipeline
reports `tee`'s exit status, not `curl`'s, so a 404 silently installs an empty
keyring and the next `sudo apt update` fails on a repository it cannot verify.

Confirm it took, before installing anything:

```bash
apt policy ratclick
```

It should list `https://dixonsolutions.github.io/ratclick/apt stable/main` as a
source and show a candidate version. To check that the key the site served is
the expected one, its fingerprint is in [keys/README.md](../keys/README.md):

```bash
gpg --show-keys --with-fingerprint /etc/apt/keyrings/ratclick.asc
```

The repository is signed with both `InRelease` and a detached `Release.gpg`, so
any apt from Debian 10 / Ubuntu 20.04 onwards is happy with it.

If you would rather use the dearmoured key — some tooling insists on a keyring
in binary form — it is published alongside the armoured one:

```bash
curl -fsSL -o /tmp/ratclick.gpg https://dixonsolutions.github.io/ratclick/ratclick.gpg \
  && sudo install -d -m 0755 /etc/apt/keyrings \
  && sudo install -m 0644 /tmp/ratclick.gpg /etc/apt/keyrings/ratclick.gpg \
  && echo "deb [signed-by=/etc/apt/keyrings/ratclick.gpg] https://dixonsolutions.github.io/ratclick/apt stable main" \
     | sudo tee /etc/apt/sources.list.d/ratclick.list >/dev/null
```

### If `apt update` is failing on ratclick

Both of these mean the sources file exists but the keyring next to it does not,
so apt has no way to verify the repository:

```
E: The repository 'https://dixonsolutions.github.io/ratclick/apt stable InRelease' is not signed.
W: GPG error: … NO_PUBKEY 0C2EA70964F7D273                       ← keyring file missing
W: GPG error: … GPG: add_keyblock_resource 33587281 …            ← keyring file empty
```

This is what an interrupted or 404'd key download used to leave behind, and it
keeps breaking every later `sudo apt update`, not just RatClick's. Remove both
files, check apt is healthy again, then re-run the block above and watch for a
`curl:` error on the first line:

```bash
sudo rm -f /etc/apt/sources.list.d/ratclick.list /etc/apt/keyrings/ratclick.asc
sudo apt update
```

`404 Not Found` on the key or on `dists/stable/InRelease` means the published
site is not answering; the packages are also attached to every
[release](https://github.com/dixonSolutions/ratclick/releases) if you need one
right now.

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
