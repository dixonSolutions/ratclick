#!/bin/sh
# RPM %postun for ratclick. $1 is 0 on the final removal, 1 or more mid-upgrade.
#
# Refresh the same caches the install touched. Never fail the transaction.

if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules >/dev/null 2>&1 || :
    udevadm trigger --subsystem-match=misc --sysname-match=uinput >/dev/null 2>&1 || :
fi

if command -v glib-compile-schemas >/dev/null 2>&1 && [ -d /usr/share/glib-2.0/schemas ]; then
    glib-compile-schemas /usr/share/glib-2.0/schemas >/dev/null 2>&1 || :
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -d /usr/share/icons/hicolor ]; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor >/dev/null 2>&1 || :
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || :
fi

exit 0
