#!/bin/sh
# RPM %post for ratclick. $1 is 1 on a first install, 2 or more on an upgrade.
#
# As with the Debian postinst, nothing here may fail the transaction: these are
# all best-effort cache refreshes, and a container image has neither a running
# udev nor half of these tools.

if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules >/dev/null 2>&1 || :
    udevadm trigger --subsystem-match=misc --sysname-match=uinput >/dev/null 2>&1 || :
fi

if command -v glib-compile-schemas >/dev/null 2>&1; then
    if [ -d /usr/share/glib-2.0/schemas ]; then
        glib-compile-schemas /usr/share/glib-2.0/schemas >/dev/null 2>&1 || :
    fi
    ext_schemas=/usr/share/gnome-shell/extensions/ratclick@dixonsolutions.github.io/schemas
    if [ -d "$ext_schemas" ]; then
        glib-compile-schemas "$ext_schemas" >/dev/null 2>&1 || :
    fi
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -d /usr/share/icons/hicolor ]; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor >/dev/null 2>&1 || :
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || :
fi

if [ "$1" = "1" ]; then
    cat <<'EOF'

RatClick is installed. Open it with:

    ratclick gui

Clicking needs write access to /dev/uinput, which the installed udev rule
grants to the `input` group. If you are not in it yet:

    sudo usermod -aG input $USER

then log out and back in. `ratclick doctor` checks all of this for you.

EOF
fi

exit 0
