#!/usr/bin/env bash
#
# Build the GNOME Shell extension zip that the release attaches, and that
# extensions.gnome.org would accept.
#
#   scripts/pack-extension.sh <out-dir>
#
# Exits 0 with a message and no output file when there is no extension in the
# tree yet — it is produced separately, and its absence must not fail a release.

set -euo pipefail

export PATH="/usr/bin:$PATH"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/dist}"

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
skip() { printf '\033[1;33m--\033[0m %s\n' "$*"; }
die() {
    printf '\033[1;31mpack-extension.sh:\033[0m %s\n' "$*" >&2
    exit 1
}

if [ ! -f "$ROOT/extension/metadata.json" ]; then
    skip "extension/metadata.json is missing — no extension zip to build"
    exit 0
fi

command -v gnome-extensions >/dev/null 2>&1 ||
    die "gnome-extensions is missing. On Debian/Ubuntu it ships in the gnome-shell package."

mkdir -p "$OUT"

# `gnome-extensions pack` picks up extension.js, prefs.js, metadata.json,
# stylesheet.css, schemas/ and locale/ by itself. Anything else the extension
# imports — dbus.js today, whatever gets added later — has to be named.
extra=()
while IFS= read -r -d '' f; do
    base="$(basename "$f")"
    case "$base" in
    extension.js | prefs.js | stylesheet.css | metadata.json) ;;
    *) extra+=("--extra-source=$base") ;;
    esac
done < <(find "$ROOT/extension" -maxdepth 1 -type f \( -name '*.js' -o -name '*.css' -o -name '*.json' \) -print0)

if [ "${#extra[@]}" -gt 0 ]; then
    say "extra sources: ${extra[*]#--extra-source=}"
fi

say "gnome-extensions pack"
gnome-extensions pack "$ROOT/extension" --force --out-dir "$OUT" "${extra[@]}"

zip="$OUT/ratclick@dixonsolutions.github.io.shell-extension.zip"
[ -f "$zip" ] || die "pack produced no $zip"
say "built $(basename "$zip")"
