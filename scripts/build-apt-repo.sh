#!/usr/bin/env bash
#
# Build (or extend) the signed apt repository that RatClick publishes on GitHub
# Pages.
#
#   scripts/build-apt-repo.sh <site-dir> <deb> [<deb> ...]
#
# <site-dir> is the root of the published site. It may already contain an apt/
# tree from a previous release: this script *adds* to the pool and regenerates
# the metadata, so old versions stay installable.
#
# Layout produced, matching the sources.list line in docs/install.md
# (`deb <url>/apt stable main`):
#
#   <site-dir>/apt/pool/main/r/ratclick/ratclick_<ver>-1_<arch>.deb
#   <site-dir>/apt/dists/stable/main/binary-<arch>/{Packages,Packages.gz,Release}
#   <site-dir>/apt/dists/stable/{Release,Release.gpg,InRelease}
#
# Signing uses whichever secret key GNUPGHOME holds; set GPG_KEY_ID to pick one
# when there is more than one, and GPG_PASSPHRASE when the key has one.

set -euo pipefail

export PATH="/usr/bin:$PATH"

ORIGIN="RatClick"
LABEL="RatClick"
SUITE="stable"
CODENAME="stable"
COMPONENT="main"
DESCRIPTION="RatClick packages for Debian and Ubuntu"

die() {
    printf '\033[1;31mbuild-apt-repo.sh:\033[0m %s\n' "$*" >&2
    exit 1
}
say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

[ $# -ge 2 ] || die "usage: build-apt-repo.sh <site-dir> <deb> [<deb> ...]"
command -v apt-ftparchive >/dev/null 2>&1 ||
    die "apt-ftparchive is missing. On Debian/Ubuntu: sudo apt-get install apt-utils"
command -v gpg >/dev/null 2>&1 || die "gpg is missing"

SITE="$1"
shift

REPO="$SITE/apt"
POOL="$REPO/pool/$COMPONENT/r/ratclick"
mkdir -p "$POOL"

for deb in "$@"; do
    [ -f "$deb" ] || die "no such .deb: $deb"
    say "pool <- $(basename "$deb")"
    install -m 0644 "$deb" "$POOL/$(basename "$deb")"
done

# Which architectures does the pool actually hold? Deriving this from the pool
# rather than hard-coding amd64 means an arm64 build later needs no edit here.
mapfile -t ARCHES < <(
    for f in "$POOL"/*.deb; do
        [ -e "$f" ] || continue
        dpkg-deb -f "$f" Architecture
    done | sort -u
)
[ "${#ARCHES[@]}" -gt 0 ] || die "the pool is empty — nothing to index"
say "architectures: ${ARCHES[*]}"

# apt-ftparchive writes the pool paths it is given verbatim into Filename:, and
# those must be relative to the repository root, so run it from there.
cd "$REPO"

for arch in "${ARCHES[@]}"; do
    dir="dists/$SUITE/$COMPONENT/binary-$arch"
    mkdir -p "$dir"
    apt-ftparchive --arch "$arch" packages "pool/$COMPONENT" >"$dir/Packages"
    gzip -9nc "$dir/Packages" >"$dir/Packages.gz"

    # A per-binary Release file. apt does not strictly need it, but apt-secure
    # warns without it and it costs nothing.
    cat >"$dir/Release" <<EOF
Archive: $SUITE
Component: $COMPONENT
Origin: $ORIGIN
Label: $LABEL
Architecture: $arch
EOF
    say "indexed $arch: $(grep -c '^Package: ' "$dir/Packages") package(s)"
done

# Drop indices for architectures that are no longer in the pool, otherwise the
# stale Packages file stays listed in Release and apt fetches a phantom.
for dir in "dists/$SUITE/$COMPONENT"/binary-*; do
    [ -d "$dir" ] || continue
    arch="${dir##*binary-}"
    keep=0
    for a in "${ARCHES[@]}"; do [ "$a" = "$arch" ] && keep=1; done
    [ "$keep" = 1 ] || { say "dropping stale index for $arch"; rm -rf "$dir"; }
done

say "writing dists/$SUITE/Release"
rm -f "dists/$SUITE/Release" "dists/$SUITE/Release.gpg" "dists/$SUITE/InRelease"
# Written outside the tree first. apt-ftparchive hashes every file it finds
# under dists/$SUITE, so redirecting straight into dists/$SUITE/Release makes
# the file list itself — apt then chases a checksum that cannot ever match.
release_tmp="$(mktemp)"
trap 'rm -f "$release_tmp"' EXIT
apt-ftparchive \
    -o "APT::FTPArchive::Release::Origin=$ORIGIN" \
    -o "APT::FTPArchive::Release::Label=$LABEL" \
    -o "APT::FTPArchive::Release::Suite=$SUITE" \
    -o "APT::FTPArchive::Release::Codename=$CODENAME" \
    -o "APT::FTPArchive::Release::Components=$COMPONENT" \
    -o "APT::FTPArchive::Release::Architectures=${ARCHES[*]}" \
    -o "APT::FTPArchive::Release::Description=$DESCRIPTION" \
    release "dists/$SUITE" >"$release_tmp"
install -m 0644 "$release_tmp" "dists/$SUITE/Release"

# ---------------------------------------------------------------------------
# Signatures
# ---------------------------------------------------------------------------
gpg_args=(--batch --yes --pinentry-mode loopback)
if [ -n "${GPG_PASSPHRASE:-}" ]; then
    gpg_args+=(--passphrase "$GPG_PASSPHRASE")
fi
if [ -n "${GPG_KEY_ID:-}" ]; then
    gpg_args+=(--local-user "$GPG_KEY_ID")
fi

gpg --list-secret-keys >/dev/null 2>&1 ||
    die "no secret key available to sign with (check GNUPGHOME)"

say "signing Release"
gpg "${gpg_args[@]}" --armor --detach-sign \
    --output "dists/$SUITE/Release.gpg" "dists/$SUITE/Release"
gpg "${gpg_args[@]}" --clearsign \
    --output "dists/$SUITE/InRelease" "dists/$SUITE/Release"

# ---------------------------------------------------------------------------
# Public key, in both shapes apt accepts for signed-by=
# ---------------------------------------------------------------------------
key_selector=("${GPG_KEY_ID:-}")
[ -n "${key_selector[0]}" ] || key_selector=()

say "exporting the public key to ratclick.asc and ratclick.gpg"
gpg --batch --yes --armor --export "${key_selector[@]}" >"$SITE/ratclick.asc"
gpg --batch --yes --export "${key_selector[@]}" >"$SITE/ratclick.gpg"
[ -s "$SITE/ratclick.asc" ] || die "exporting the public key produced nothing"

say "apt repository ready under $REPO"
