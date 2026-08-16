#!/usr/bin/env bash
#
# Build the RatClick release binaries, then a .deb and an .rpm, into dist/.
#
# Runs the same way locally and in CI. Every real failure is fatal; the only
# things that are allowed to be missing are the icon set and the GNOME Shell
# extension, which are produced separately — those are reported as skips.
#
# Usage:
#   scripts/build-packages.sh              build everything
#   scripts/build-packages.sh --no-build   package binaries that are already built
#   scripts/build-packages.sh --deb-only
#   scripts/build-packages.sh --rpm-only

set -euo pipefail

# Homebrew's pkg-config shadows the system one on some developer machines and
# cannot see /usr/lib/x86_64-linux-gnu/pkgconfig, so gtk4-sys fails to
# configure. Putting /usr/bin first fixes it and is harmless everywhere else:
# cargo and the rust toolchain live in ~/.cargo/bin or /usr/bin, both of which
# stay on PATH.
export PATH="/usr/bin:$PATH"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STAGE="$ROOT/packaging/generated"
EXEC="$STAGE/exec"   # installed 0755
DATA="$STAGE/data"   # installed 0644
DIST="$ROOT/dist"
EXT_UUID="ratclick@dixonsolutions.github.io"

DO_BUILD=1
DO_DEB=1
DO_RPM=1
for arg in "$@"; do
    case "$arg" in
    --no-build) DO_BUILD=0 ;;
    --deb-only) DO_RPM=0 ;;
    --rpm-only) DO_DEB=0 ;;
    -h | --help)
        sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    *)
        echo "build-packages.sh: unknown argument '$arg'" >&2
        exit 2
        ;;
    esac
done

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
skip() { printf '\033[1;33m--\033[0m %s\n' "$*"; }
die() {
    printf '\033[1;31mbuild-packages.sh:\033[0m %s\n' "$*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is not installed. $2"
}

VERSION="$(awk '/^\[workspace\.package\]/{f=1} f && /^version *=/{gsub(/[" ]/,"",$3); print $3; exit}' Cargo.toml)"
[ -n "$VERSION" ] || die "could not read the version out of Cargo.toml"
say "RatClick $VERSION"

# ---------------------------------------------------------------------------
# 1. Binaries
# ---------------------------------------------------------------------------
if [ "$DO_BUILD" = 1 ]; then
    need cargo "Install Rust from https://rustup.rs"
    say "cargo build --release --workspace"
    cargo build --release --workspace
fi

for bin in ratclick ratclick-gui ratclickd; do
    [ -x "target/release/$bin" ] ||
        die "target/release/$bin is missing — run without --no-build"
done

# ---------------------------------------------------------------------------
# 2. Man page
# ---------------------------------------------------------------------------
need scdoc "On Debian/Ubuntu: sudo apt install scdoc; on Fedora: sudo dnf install scdoc"
need gzip ""

# ---------------------------------------------------------------------------
# 3. Stage the install tree
# ---------------------------------------------------------------------------
say "staging into packaging/generated"
rm -rf "$STAGE"
mkdir -p \
    "$EXEC/bin" \
    "$EXEC/libexec/ratclick" \
    "$DATA/lib/systemd/user" \
    "$DATA/lib/udev/rules.d" \
    "$DATA/share/dbus-1/services" \
    "$DATA/share/applications" \
    "$DATA/share/metainfo" \
    "$DATA/share/man/man1" \
    "$DATA/share/doc/ratclick"

install -m 0755 target/release/ratclick "$EXEC/bin/ratclick"
install -m 0755 target/release/ratclick-gui "$EXEC/bin/ratclick-gui"
install -m 0755 target/release/ratclickd "$EXEC/libexec/ratclick/ratclickd"
install -m 0755 data/ratclick-keyd-dispatch "$EXEC/libexec/ratclick/ratclick-keyd-dispatch"

install -m 0644 data/ratclick.service "$DATA/lib/systemd/user/ratclick.service"
install -m 0644 data/60-ratclick-uinput.rules "$DATA/lib/udev/rules.d/60-ratclick-uinput.rules"
install -m 0644 data/io.github.dixonsolutions.RatClick.Daemon.service \
    "$DATA/share/dbus-1/services/io.github.dixonsolutions.RatClick.Daemon.service"
install -m 0644 data/io.github.dixonsolutions.RatClick.desktop \
    "$DATA/share/applications/io.github.dixonsolutions.RatClick.desktop"
install -m 0644 data/io.github.dixonsolutions.RatClick.metainfo.xml \
    "$DATA/share/metainfo/io.github.dixonsolutions.RatClick.metainfo.xml"

scdoc <data/ratclick.1.scd >"$DATA/share/man/man1/ratclick.1"
gzip -9n "$DATA/share/man/man1/ratclick.1"

if [ -f README.md ]; then
    install -m 0644 README.md "$DATA/share/doc/ratclick/README.md"
else
    skip "no README.md — /usr/share/doc/ratclick/README.md will not be shipped"
fi

# --- icons (produced separately; optional) ---------------------------------
if [ -d assets/icons/hicolor ]; then
    say "staging icons from assets/icons/hicolor"
    mkdir -p "$DATA/share/icons"
    cp -r assets/icons/hicolor "$DATA/share/icons/hicolor"
    find "$DATA/share/icons" -type f -exec chmod 0644 {} +
    find "$DATA/share/icons" -type d -exec chmod 0755 {} +
elif [ -f assets/logo.svg ]; then
    # The full hicolor set is not in the tree yet. A scalable icon alone is
    # enough for the .desktop entry to render, so ship that rather than an
    # application with no icon at all.
    skip "assets/icons/hicolor is missing — falling back to assets/logo.svg for the scalable icon"
    mkdir -p "$DATA/share/icons/hicolor/scalable/apps"
    install -m 0644 assets/logo.svg \
        "$DATA/share/icons/hicolor/scalable/apps/io.github.dixonsolutions.RatClick.svg"
    if [ -f assets/logo-symbolic.svg ]; then
        mkdir -p "$DATA/share/icons/hicolor/symbolic/apps"
        install -m 0644 assets/logo-symbolic.svg \
            "$DATA/share/icons/hicolor/symbolic/apps/io.github.dixonsolutions.RatClick-symbolic.svg"
    fi
else
    skip "no icons found under assets/ — the package will ship without any"
fi

# --- GNOME Shell extension (produced separately; optional) -----------------
if [ -f "extension/metadata.json" ]; then
    say "staging the GNOME Shell extension"
    EXT_DEST="$DATA/share/gnome-shell/extensions/$EXT_UUID"
    mkdir -p "$EXT_DEST"
    # Everything the Shell loads, and nothing else: the test harness and any
    # Python bytecode that ran alongside it stay out of the package.
    (cd extension && find . \
        -path ./tests -prune -o \
        -name '__pycache__' -prune -o \
        -name '*.pyc' -prune -o \
        -type f -print) |
        while read -r rel; do
            mkdir -p "$EXT_DEST/$(dirname "$rel")"
            install -m 0644 "extension/$rel" "$EXT_DEST/$rel"
        done
    [ -f "$EXT_DEST/metadata.json" ] || die "staging the extension produced no metadata.json"
else
    skip "extension/metadata.json is missing — the package will ship without the Shell extension"
fi

# ---------------------------------------------------------------------------
# 4. Packages
# ---------------------------------------------------------------------------
mkdir -p "$DIST"
rm -f "$DIST"/*.deb "$DIST"/*.rpm

# Both packagers resolve the asset globs in packaging/Cargo.toml against the
# working directory, so both are run from packaging/.
if [ "$DO_DEB" = 1 ]; then
    need cargo-deb "cargo install cargo-deb"
    say "cargo deb"
    (cd packaging && cargo deb --no-build --no-strip -p ratclick -o "$DIST")
fi

if [ "$DO_RPM" = 1 ]; then
    need cargo-generate-rpm "cargo install cargo-generate-rpm"
    say "cargo generate-rpm"
    # No -p here: cargo-generate-rpm's --package takes a *path* relative to the
    # working directory, and the working directory already is the crate.
    (cd packaging && cargo generate-rpm --target-dir "$ROOT/target" -o "$DIST")
fi

say "built:"
find "$DIST" -maxdepth 1 -type f -printf '    %f\n' | sort
