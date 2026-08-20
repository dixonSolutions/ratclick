#!/usr/bin/env bash
#
# Assemble the whole GitHub Pages site: the apt repo, the dnf repo, the public
# signing key, a ready-made dnf .repo file and a landing page.
#
#   scripts/build-pages.sh <site-dir> <dist-dir>
#
# <site-dir> should already contain whatever the previous release published
# (the workflow checks out gh-pages into it first) so that the package pools
# accumulate instead of being replaced.
#
# Environment:
#   SITE_URL         public base URL, default https://dixonsolutions.github.io/ratclick
#   GPG_KEY_ID       which secret key to sign with
#   GPG_PASSPHRASE   its passphrase, if it has one

set -euo pipefail

export PATH="/usr/bin:$PATH"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SITE_URL="${SITE_URL:-https://dixonsolutions.github.io/ratclick}"

die() {
    printf '\033[1;31mbuild-pages.sh:\033[0m %s\n' "$*" >&2
    exit 1
}
say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

[ $# -eq 2 ] || die "usage: build-pages.sh <site-dir> <dist-dir>"
SITE="$1"
DIST="$2"

mkdir -p "$SITE"
[ -d "$DIST" ] || die "no such dist directory: $DIST"

mapfile -t DEBS < <(find "$DIST" -maxdepth 1 -name '*.deb' | sort)
mapfile -t RPMS < <(find "$DIST" -maxdepth 1 -name '*.rpm' | sort)
[ "${#DEBS[@]}" -gt 0 ] || die "no .deb in $DIST"
[ "${#RPMS[@]}" -gt 0 ] || die "no .rpm in $DIST"

"$HERE/build-apt-repo.sh" "$SITE" "${DEBS[@]}"
"$HERE/build-dnf-repo.sh" "$SITE" "${RPMS[@]}"

# GitHub Pages runs the published tree through Jekyll unless told not to, and
# Jekyll silently drops directories beginning with an underscore and rewrites
# some files. .nojekyll turns all of that off.
touch "$SITE/.nojekyll"

say "writing ratclick.repo and ratclick.list"
cat >"$SITE/ratclick.repo" <<EOF
[ratclick]
name=RatClick
baseurl=$SITE_URL/dnf
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=$SITE_URL/ratclick.asc
metadata_expire=6h
EOF

cat >"$SITE/ratclick.list" <<EOF
deb [signed-by=/etc/apt/keyrings/ratclick.asc] $SITE_URL/apt stable main
EOF

say "writing index.html"
cat >"$SITE/index.html" <<EOF
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>RatClick package repositories</title>
<style>
  :root {
    color-scheme: light dark;
    --bg: #ffffff; --fg: #1c1b1f; --muted: #5d5b62;
    --card: #f5f4f7; --border: #d8d6dd; --accent: #7a4fd6;
  }
  @media (prefers-color-scheme: dark) {
    :root { --bg: #17161a; --fg: #eceaf0; --muted: #a4a1ac;
            --card: #232128; --border: #35323c; --accent: #b79bf0; }
  }
  * { box-sizing: border-box; }
  body {
    margin: 0 auto; padding: 3rem 1.25rem 5rem; max-width: 46rem;
    background: var(--bg); color: var(--fg);
    font: 16px/1.65 system-ui, -apple-system, "Segoe UI", sans-serif;
  }
  h1 { font-size: 2rem; margin: 0 0 .25rem; letter-spacing: -.02em; }
  h2 { font-size: 1.2rem; margin: 2.5rem 0 .75rem; }
  p.lede { color: var(--muted); margin: 0 0 2rem; }
  pre {
    background: var(--card); border: 1px solid var(--border);
    border-radius: 8px; padding: .9rem 1rem; overflow-x: auto;
    font: 13.5px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }
  code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
  a { color: var(--accent); }
  footer { margin-top: 3rem; color: var(--muted); font-size: .9rem; }
</style>
</head>
<body>
<h1>RatClick</h1>
<p class="lede">A desktop-independent auto-clicker for Linux. These are the
signed apt and dnf repositories; both are refreshed by the release
workflow.</p>

<h2>Debian / Ubuntu</h2>
<pre><code>sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL -o /tmp/ratclick.asc $SITE_URL/ratclick.asc \\
  &amp;&amp; grep -q 'BEGIN PGP PUBLIC KEY' /tmp/ratclick.asc \\
  &amp;&amp; sudo install -m 0644 /tmp/ratclick.asc /etc/apt/keyrings/ratclick.asc \\
  &amp;&amp; echo "deb [signed-by=/etc/apt/keyrings/ratclick.asc] $SITE_URL/apt stable main" \\
     | sudo tee /etc/apt/sources.list.d/ratclick.list &gt;/dev/null \\
  &amp;&amp; sudo apt update \\
  &amp;&amp; sudo apt install ratclick</code></pre>
<p>The chain is deliberate: the sources file is written only once the signing
key is really in place, so a failed download leaves nothing for
<code>apt update</code> to choke on.</p>

<h2>Fedora / RHEL</h2>
<pre><code>sudo curl -fsSL -o /etc/yum.repos.d/ratclick.repo $SITE_URL/ratclick.repo
sudo rpm --import $SITE_URL/ratclick.asc
sudo dnf install ratclick</code></pre>

<h2>After installing</h2>
<pre><code>ratclick gui        # open the window
ratclick doctor     # check the install

# clicking needs write access to /dev/uinput:
sudo usermod -aG input \$USER   # then log out and back in</code></pre>

<h2>What is here</h2>
<pre><code>$SITE_URL/apt/            apt repository (suite "stable", component "main")
$SITE_URL/dnf/            dnf repository
$SITE_URL/ratclick.asc    signing key, ASCII-armoured
$SITE_URL/ratclick.gpg    signing key, dearmoured, for signed-by=
$SITE_URL/ratclick.repo   ready-made dnf repository file
$SITE_URL/ratclick.list   ready-made apt sources line</code></pre>

<footer>
Source and issue tracker:
<a href="https://github.com/dixonSolutions/ratclick">github.com/dixonSolutions/ratclick</a>
</footer>
</body>
</html>
EOF

say "site ready under $SITE"
find "$SITE" -maxdepth 2 -mindepth 1 -not -path '*/repodata/*' | sort | sed 's/^/    /'
