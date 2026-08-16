#!/usr/bin/env bash
#
# Regenerate the RatClick hicolor icon theme from the source SVGs.
#
# Requires: rsvg-convert  (Debian/Ubuntu: apt-get install librsvg2-bin)
#
# Output layout (freedesktop hicolor theme):
#   icons/hicolor/<size>x<size>/apps/io.github.dixonsolutions.RatClick.png
#   icons/hicolor/scalable/apps/io.github.dixonsolutions.RatClick.svg
#   icons/hicolor/symbolic/apps/io.github.dixonsolutions.RatClick-symbolic.svg

set -euo pipefail

APP_ID="io.github.dixonsolutions.RatClick"
SIZES=(16 24 32 48 64 128 256 512)

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_LOGO="${HERE}/logo.svg"
SRC_SYMBOLIC="${HERE}/logo-symbolic.svg"
OUT="${HERE}/icons/hicolor"

if ! command -v rsvg-convert >/dev/null 2>&1; then
    echo "error: rsvg-convert not found (install librsvg2-bin)" >&2
    exit 1
fi

for f in "${SRC_LOGO}" "${SRC_SYMBOLIC}"; do
    [ -f "${f}" ] || { echo "error: missing source ${f}" >&2; exit 1; }
done

# Fail fast on malformed SVG rather than emitting a blank PNG.
if command -v python3 >/dev/null 2>&1; then
    python3 - "${SRC_LOGO}" "${SRC_SYMBOLIC}" "${HERE}/banner.svg" <<'EOF'
import sys, xml.dom.minidom
for path in sys.argv[1:]:
    xml.dom.minidom.parse(path)
    print(f"validated {path}")
EOF
fi

echo "generating raster icons..."
for size in "${SIZES[@]}"; do
    dir="${OUT}/${size}x${size}/apps"
    mkdir -p "${dir}"
    rsvg-convert -w "${size}" -h "${size}" "${SRC_LOGO}" -o "${dir}/${APP_ID}.png"
    echo "  ${size}x${size}"
done

echo "installing scalable + symbolic svg..."
mkdir -p "${OUT}/scalable/apps" "${OUT}/symbolic/apps"
cp -f "${SRC_LOGO}"     "${OUT}/scalable/apps/${APP_ID}.svg"
cp -f "${SRC_SYMBOLIC}" "${OUT}/symbolic/apps/${APP_ID}-symbolic.svg"

echo "done -> ${OUT}"
