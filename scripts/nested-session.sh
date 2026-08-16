#!/usr/bin/env bash
# Run a throwaway nested GNOME Shell session for testing RatClick.
#
# Why this exists
# ---------------
# RatClick installs global keyboard shortcuts through gsettings, and gsettings
# is per-*user*, not per-session — so testing it in your own session would
# rewrite your real keybindings. This script points XDG_CONFIG_HOME at a scratch
# directory, which gives the nested shell (and RatClick) their own dconf
# database, their own config.toml and their own D-Bus session. Nothing it does
# touches your real desktop.
#
# Usage
# -----
#   scripts/nested-session.sh start     # launch the headless shell
#   scripts/nested-session.sh run CMD…  # run a command inside the session
#   scripts/nested-session.sh shell     # interactive shell inside the session
#   scripts/nested-session.sh env       # print the env vars to export
#   scripts/nested-session.sh screenshot OUT.png   # capture the virtual monitor
#   scripts/nested-session.sh stop      # tear everything down
#   scripts/nested-session.sh reset     # stop and delete the scratch state
#
# The scratch root can be overridden with RATCLICK_NESTED_ROOT.

set -euo pipefail

ROOT="${RATCLICK_NESTED_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/.nested}"
DISPLAY_NAME="${RATCLICK_NESTED_DISPLAY:-ratclick-nested}"
MONITOR="${RATCLICK_NESTED_MONITOR:-1400x1000}"
STATE="$ROOT/state"
ENV_FILE="$STATE/env"
SHELL_PID_FILE="$STATE/shell.pid"
BUS_PID_FILE="$STATE/bus.pid"

mkdir -p "$STATE"

log() { printf '\033[1m›\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }

# --- env -------------------------------------------------------------------

write_env() {
    cat >"$ENV_FILE" <<EOF
export XDG_CONFIG_HOME='$ROOT/config'
export XDG_DATA_HOME='$ROOT/data'
export XDG_CACHE_HOME='$ROOT/cache'
export XDG_STATE_HOME='$ROOT/state-home'
export DBUS_SESSION_BUS_ADDRESS='$1'
export WAYLAND_DISPLAY='$DISPLAY_NAME'
export XDG_CURRENT_DESKTOP='GNOME'
export XDG_SESSION_TYPE='wayland'
export GNOME_SHELL_SESSION_MODE='user'
EOF
}

require_env() {
    [ -f "$ENV_FILE" ] || die "no nested session — run: $0 start"
    # shellcheck disable=SC1090
    . "$ENV_FILE"
}

# --- safety ----------------------------------------------------------------

# Prove that settings written inside the session cannot reach the real desktop.
#
# This is a hard gate rather than a warning. The failure it guards against is
# silent and destructive: if dconf-service was activated with the real config
# path, RatClick's shortcut installer reads an empty list from the scratch
# database and writes it back over your actual custom keybindings, deleting
# them. Refusing to run is the only safe response.
verify_isolation() {
    local probe='/ratclick/isolation-probe'

    # shellcheck disable=SC1090
    ( . "$ENV_FILE"; dconf write "$probe" 'true' ) 2>/dev/null \
        || die "cannot write to the scratch dconf — refusing to run"

    local inside outside
    # shellcheck disable=SC1090
    inside=$( . "$ENV_FILE"; dconf read "$probe" 2>/dev/null )
    outside=$(env -u DBUS_SESSION_BUS_ADDRESS \
        XDG_CONFIG_HOME="${REAL_XDG_CONFIG_HOME:-$HOME/.config}" \
        dconf read "$probe" 2>/dev/null)

    # shellcheck disable=SC1090
    ( . "$ENV_FILE"; dconf reset "$probe" ) 2>/dev/null || true

    if [ "$inside" != "true" ]; then
        stop
        die "the scratch session cannot persist settings (got '$inside') — refusing to run"
    fi
    if [ -n "$outside" ]; then
        stop
        die "SETTINGS ARE LEAKING INTO YOUR REAL DESKTOP (probe visible outside) — refusing to run"
    fi
    [ -f "$ROOT/config/dconf/user" ] \
        || { stop; die "no scratch dconf database was created — refusing to run"; }

    log "isolation verified: settings stay inside $ROOT"
}

# --- lifecycle -------------------------------------------------------------

start() {
    if [ -f "$SHELL_PID_FILE" ] && kill -0 "$(cat "$SHELL_PID_FILE")" 2>/dev/null; then
        log "already running (pid $(cat "$SHELL_PID_FILE"))"
        return 0
    fi

    command -v gnome-shell >/dev/null || die "gnome-shell is not installed"
    command -v dbus-daemon >/dev/null || die "dbus-daemon is not installed"

    mkdir -p "$ROOT"/{config,data,cache,state-home}

    log "starting a private session bus"
    # The bus must be started *inside* the scratch environment, not just the
    # shell. Everything D-Bus activates inherits the bus daemon's environment,
    # and the service that matters is dconf-service: activated with your real
    # XDG_CONFIG_HOME it writes settings straight into your live desktop, while
    # gsettings *reads* come from the scratch database. That split brain is
    # worse than no isolation at all — writes land on the real system and the
    # test cannot see them, so nothing looks wrong until your keybindings are
    # already gone.
    local addr
    addr=$(env \
        XDG_CONFIG_HOME="$ROOT/config" \
        XDG_DATA_HOME="$ROOT/data" \
        XDG_CACHE_HOME="$ROOT/cache" \
        XDG_STATE_HOME="$ROOT/state-home" \
        dbus-daemon --session --print-address --fork --print-pid="3" 3>"$BUS_PID_FILE")
    write_env "$addr"

    log "starting headless gnome-shell on wayland display '$DISPLAY_NAME'"
    # GNOME 50 has no nested backend. MetaBackendX11Nested went away with X11
    # session support in GNOME 49, so `gnome-shell --wayland` without
    # `--display-server` no longer opens a window inside your session — it tries
    # to become the real display server and dies with
    #   Failed to take control of the session: ... EBUSY
    # because logind has already handed control to your actual session.
    #
    # `--headless --virtual-monitor` is the GNOME 50 replacement: a complete,
    # fully functional shell rendering to an offscreen monitor, which is exactly
    # what we want for testing and is strictly safer than a nested window. Use
    # `screenshot` below to see what it looks like.
    env \
        XDG_CONFIG_HOME="$ROOT/config" \
        XDG_DATA_HOME="$ROOT/data" \
        XDG_CACHE_HOME="$ROOT/cache" \
        XDG_STATE_HOME="$ROOT/state-home" \
        DBUS_SESSION_BUS_ADDRESS="$addr" \
        GNOME_SHELL_SESSION_MODE=user \
        gnome-shell --headless --virtual-monitor "$MONITOR" \
        --wayland-display="$DISPLAY_NAME" \
        >"$STATE/shell.log" 2>&1 &
    echo $! >"$SHELL_PID_FILE"

    # shellcheck disable=SC1090
    . "$ENV_FILE"

    log "waiting for the shell to come up"
    local i
    for i in $(seq 1 60); do
        if gdbus call --session --dest org.gnome.Shell \
                --object-path /org/gnome/Shell \
                --method org.freedesktop.DBus.Peer.Ping >/dev/null 2>&1; then
            verify_isolation
            log "nested GNOME Shell is ready"
            log "  config:  $ROOT/config"
            log "  log:     $STATE/shell.log"
            return 0
        fi
        if ! kill -0 "$(cat "$SHELL_PID_FILE")" 2>/dev/null; then
            tail -30 "$STATE/shell.log" >&2
            die "gnome-shell exited during startup"
        fi
        sleep 0.5
    done
    tail -30 "$STATE/shell.log" >&2
    die "timed out waiting for the nested shell"
}

stop() {
    if [ -f "$SHELL_PID_FILE" ]; then
        local pid
        pid=$(cat "$SHELL_PID_FILE")
        if kill -0 "$pid" 2>/dev/null; then
            log "stopping nested shell (pid $pid)"
            kill "$pid" 2>/dev/null || true
            # Give it a moment to close cleanly before insisting.
            for _ in $(seq 1 20); do
                kill -0 "$pid" 2>/dev/null || break
                sleep 0.25
            done
            kill -9 "$pid" 2>/dev/null || true
        fi
        rm -f "$SHELL_PID_FILE"
    fi

    if [ -f "$BUS_PID_FILE" ]; then
        local bpid
        bpid=$(cat "$BUS_PID_FILE")
        kill "$bpid" 2>/dev/null || true
        rm -f "$BUS_PID_FILE"
    fi
    rm -f "$ENV_FILE"
    log "stopped"
}

case "${1:-}" in
    start) start ;;
    stop)  stop ;;
    reset) stop; rm -rf "$ROOT"; log "scratch state deleted" ;;
    env)   require_env; cat "$ENV_FILE" ;;
    shell) require_env; exec "${SHELL:-bash}" ;;
    run)
        shift
        [ $# -gt 0 ] || die "run needs a command"
        require_env
        exec "$@"
        ;;
    log)   tail -f "$STATE/shell.log" ;;
    screenshot)
        require_env
        out="${2:-$STATE/screenshot.png}"
        # The Shell's own screenshot API is the only thing that can see a
        # headless virtual monitor; no external tool has a surface to grab.
        gdbus call --session --dest org.gnome.Shell.Screenshot \
            --object-path /org/gnome/Shell/Screenshot \
            --method org.gnome.Shell.Screenshot.Screenshot \
            false false "$out" >/dev/null || die "screenshot failed"
        log "wrote $out"
        ;;
    *)
        sed -n '2,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
        exit 1
        ;;
esac
