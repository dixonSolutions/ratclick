#!/usr/bin/env bash
#
# GPG-sign built .rpm files in place, so the published dnf repository can keep
# gpgcheck=1 rather than only verifying the repository metadata.
#
#   scripts/sign-rpm.sh <rpm> [<rpm> ...]
#
# cargo-generate-rpm's own --signing-key cannot take a passphrase, and the
# release key has one, so this drives rpmsign instead.
#
# Environment:
#   GPG_KEY_ID       fingerprint or uid of the secret key (required)
#   GPG_PASSPHRASE   its passphrase, if it has one
#   GNUPGHOME        where that key lives

set -euo pipefail

export PATH="/usr/bin:$PATH"

die() {
    printf '\033[1;31msign-rpm.sh:\033[0m %s\n' "$*" >&2
    exit 1
}
say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

[ $# -ge 1 ] || die "usage: sign-rpm.sh <rpm> [<rpm> ...]"
command -v rpmsign >/dev/null 2>&1 ||
    die "rpmsign is missing. On Fedora: dnf install rpm-sign; on Debian/Ubuntu it is in the rpm package."
[ -n "${GPG_KEY_ID:-}" ] || die "GPG_KEY_ID is not set"

passfile="$(mktemp)"
trap 'rm -f "$passfile"' EXIT
chmod 600 "$passfile"
printf '%s' "${GPG_PASSPHRASE:-}" >"$passfile"

# rpm's stock signing macro expects an interactive pinentry. `%_gpg_sign_cmd_extra_args`
# is the supported hook for adding flags to it, and it exists in both rpm 4 and
# rpm 6 — which matters because the release builds in a Fedora container whose
# rpm major version is not ours.
rpm_defines=(
    -D "_gpg_name $GPG_KEY_ID"
    -D "_gpg_sign_cmd_extra_args --pinentry-mode loopback --passphrase-file $passfile"
)

for rpm in "$@"; do
    [ -f "$rpm" ] || die "no such .rpm: $rpm"
    say "signing $(basename "$rpm")"
    rpmsign "${rpm_defines[@]}" --addsign "$rpm"
done

# Verify by reading the signature back. `NOKEY` is expected and fine here: the
# public key is not in this machine's rpmdb, but the signature block is there,
# which is all this step is claiming.
for rpm in "$@"; do
    out="$(rpm -Kv "$rpm" 2>&1 || true)"
    printf '%s\n' "$out" | grep -qi 'signature' ||
        die "$(basename "$rpm") still has no signature:"$'\n'"$out"
    say "$(basename "$rpm"): $(printf '%s\n' "$out" | grep -i 'signature' | head -1 | sed 's/^ *//')"
done
