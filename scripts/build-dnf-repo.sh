#!/usr/bin/env bash
#
# Build (or extend) the signed dnf repository that RatClick publishes on GitHub
# Pages.
#
#   scripts/build-dnf-repo.sh <site-dir> <rpm> [<rpm> ...]
#
# <site-dir> is the root of the published site. It may already contain a dnf/
# tree from a previous release: this script *adds* the new packages and
# regenerates the metadata, so old versions stay installable.
#
# Layout produced, matching the baseurl in docs/install.md:
#
#   <site-dir>/dnf/packages/ratclick-<ver>-1.<arch>.rpm
#   <site-dir>/dnf/repodata/{repomd.xml,repomd.xml.asc,...}
#
# The .rpm files themselves are signed by the release workflow before they get
# here (see .github/workflows/release.yml), which is what lets the published
# .repo file keep gpgcheck=1. This script signs the repository metadata.

set -euo pipefail

export PATH="/usr/bin:$PATH"

die() {
    printf '\033[1;31mbuild-dnf-repo.sh:\033[0m %s\n' "$*" >&2
    exit 1
}
say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

[ $# -ge 2 ] || die "usage: build-dnf-repo.sh <site-dir> <rpm> [<rpm> ...]"
command -v gpg >/dev/null 2>&1 || die "gpg is missing"

CREATEREPO=""
for c in createrepo_c createrepo; do
    command -v "$c" >/dev/null 2>&1 && { CREATEREPO="$c"; break; }
done
[ -n "$CREATEREPO" ] ||
    die "createrepo_c is missing. On Debian/Ubuntu: sudo apt-get install createrepo-c; on Fedora: sudo dnf install createrepo_c"

SITE="$1"
shift

REPO="$SITE/dnf"
PKGS="$REPO/packages"
mkdir -p "$PKGS"

for rpm in "$@"; do
    [ -f "$rpm" ] || die "no such .rpm: $rpm"
    say "packages <- $(basename "$rpm")"
    install -m 0644 "$rpm" "$PKGS/$(basename "$rpm")"
done

ls "$PKGS"/*.rpm >/dev/null 2>&1 || die "no packages to index"

say "$CREATEREPO $REPO"
# --update reuses the existing xml where it can, which keeps the run cheap as
# the pool grows across releases. gzip rather than createrepo_c's default zstd:
# it costs a few kB and every dnf and yum ever shipped can read it.
"$CREATEREPO" --update --no-database --general-compress-type=gz "$REPO"

# ---------------------------------------------------------------------------
# Signature over repomd.xml (this is what repo_gpgcheck=1 verifies)
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

say "signing repodata/repomd.xml"
rm -f "$REPO/repodata/repomd.xml.asc"
gpg "${gpg_args[@]}" --detach-sign --armor \
    --output "$REPO/repodata/repomd.xml.asc" "$REPO/repodata/repomd.xml"

# The public key: harmless to repeat when the apt builder already wrote it, and
# it means this script is usable on its own.
key_selector=("${GPG_KEY_ID:-}")
[ -n "${key_selector[0]}" ] || key_selector=()
gpg --batch --yes --armor --export "${key_selector[@]}" >"$SITE/ratclick.asc"
gpg --batch --yes --export "${key_selector[@]}" >"$SITE/ratclick.gpg"
[ -s "$SITE/ratclick.asc" ] || die "exporting the public key produced nothing"

say "dnf repository ready under $REPO"
