#!/usr/bin/env bash
#
# Print the workspace version. One implementation, used by the build script and
# by the release workflow, so the tag can never disagree with the package.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

version="$(
    awk '
        /^\[workspace\.package\]/ { in_section = 1; next }
        /^\[/                     { in_section = 0 }
        in_section && /^version[[:space:]]*=/ {
            gsub(/^[^=]*=[[:space:]]*"?|"[[:space:]]*$/, "")
            print
            exit
        }
    ' "$ROOT/Cargo.toml"
)"

if [ -z "$version" ]; then
    echo "version.sh: no [workspace.package] version in $ROOT/Cargo.toml" >&2
    exit 1
fi

printf '%s\n' "$version"
