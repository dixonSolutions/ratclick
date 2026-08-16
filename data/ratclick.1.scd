ratclick(1)

# NAME

ratclick - a configurable auto-clicker for GNOME

# SYNOPSIS

*ratclick* [_COMMAND_]

# DESCRIPTION

RatClick clicks your mouse for you, at a rate and for a length of time you
choose. It is made of three parts:

- *ratclick*, this command line;
- *ratclick-gui*, a libadwaita window;
- *ratclickd*, a background service that owns the virtual pointer and the
  session D-Bus name _io.github.dixonsolutions.RatClick_.

Running *ratclick* with no command opens the window when a graphical session is
present, and prints this help when there is not.

The first command that needs a configuration runs a short setup wizard. A
non-interactive caller (a keyboard shortcut, a script) gets sensible defaults
written for it instead of a prompt it cannot answer.

# COMMANDS

*gui*
	Open the RatClick window. Runs _ratclick-gui_ from the same directory as
	this binary if there is one, otherwise _/usr/bin/ratclick-gui_.

*start*
	Start clicking. Starts the background service first if it is not up.

*stop*
	Stop clicking. Stopping something that is not running succeeds.

*toggle*
	Start clicking if stopped, stop it if running. This is what the global
	keyboard shortcut runs.

*status* [*--porcelain*]
	Show what RatClick is doing. With *--porcelain*, print machine-readable
	_key=value_ lines instead of prose: _daemon_, _clicking_, _cpm_,
	_button_, _mode_, _remaining_seconds_ and _clicks_.

*setup*
	Run the guided first-time setup again.

*doctor*
	Check the installation and report anything that would stop RatClick
	working - a missing _/dev/uinput_, missing _input_ group membership, a
	shortcut that is not installed, and so on.

## daemon

Start, stop and inspect the background service.

*daemon start*
	Launch the background service.

*daemon stop*
	Shut the background service down. Uses *systemctl --user stop
	ratclick.service* when the unit is installed, so systemd does not
	restart it a moment later.

*daemon restart*
	Restart the background service.

*daemon status*
	Report whether the background service is running, and its version.

*daemon run*
	Run the service in the foreground with verbose logging, for debugging.
	This replaces the current process with _ratclickd --verbose_.

## config

Read and change settings.

*config show*
	Print the current configuration.

*config path*
	Print the path of the configuration file.

*config set* _KEY_ _VALUE_
	Change a setting. _KEY_ is one of:

	- *cpm* - clicks per minute, a whole number.
	- *button* - _left_, _right_ or _middle_.
	- *mode* - _endless_ or _timed_.
	- *duration* - how long a timed run lasts. Accepts _90_, _1h30m_ or
	  _1:30_. Setting it switches *mode* to _timed_.
	- *hours* - the hours part of the duration. Switches to _timed_.
	- *minutes* - the minutes part of the duration, 0-59. Switches to
	  _timed_.
	- *autostart* - _true_ or _false_; start clicking as soon as the app
	  launches.

*config reset* [*--yes*]
	Delete the configuration, so the next run starts the wizard again. This
	also unbinds the shortcut. *--yes* skips the confirmation prompt.

## shortcut

Manage the global toggle shortcut. RatClick can register it through GNOME's own
custom-keybinding list, through the bundled GNOME Shell extension, or through
*keyd*(1) for a binding that works outside the desktop session.

*shortcut show*
	Show RatClick's shortcut and which backend holds it, and warn when the
	installed binding has drifted from the configured one.

*shortcut list* [_FILTER_]
	List every keyboard shortcut the desktop has bound, plus anything found
	in the keyd configuration. _FILTER_ keeps only lines containing that
	text, case-insensitively.

*shortcut check* _ACCEL_
	Report whether a key combination is free, and name whatever holds it.

*shortcut set* _ACCEL_... [*--backend* _BACKEND_] [*--force*]
	Bind one or more key combinations to the toggle. _ACCEL_ is in GTK
	form, for example _<Super><Shift>c_. *--backend* is one of _gnome_,
	_extension_, _keyd_ or _none_. *--force* takes the combination even if
	something else already holds it, unbinding the other holder where that
	is possible.

*shortcut clear*
	Remove RatClick's shortcut from every backend.

*shortcut apply*
	Re-install the configured shortcut. Run under *sudo* for the keyd
	backend, which writes to _/etc/keyd_.

# OPTIONS

*-h*, *--help*
	Print help. Every subcommand accepts it too.

*-V*, *--version*
	Print version.

# FILES

_~/.config/ratclick/config.toml_
	The configuration. *ratclick config path* prints the real location,
	which follows _XDG_CONFIG_HOME_.

_/usr/lib/systemd/user/ratclick.service_
	The systemd user unit for the background service.

_/usr/share/dbus-1/services/io.github.dixonsolutions.RatClick.service_
	D-Bus session activation, so the service starts on demand.

_/usr/lib/udev/rules.d/60-ratclick-uinput.rules_
	Gives the *input* group write access to _/dev/uinput_.

_/usr/libexec/ratclick/ratclick-keyd-dispatch_
	Helper that lets a root-owned keyd binding reach the logged-in user's
	session bus.

# PERMISSIONS

The daemon creates a virtual pointer through _/dev/uinput_, which is root-only
by default. The shipped udev rule grants the *input* group write access. Add
yourself to it and log in again:

```
sudo usermod -aG input $USER
```

*ratclick doctor* tells you whether this is already in place.

# EXIT STATUS

*ratclick* exits 0 on success and 1 on failure, printing the reason to standard
error.

# SEE ALSO

*ratclickd*(1), *keyd*(1), *systemctl*(1), *udev*(7)

# BUGS

https://github.com/dixonSolutions/ratclick/issues
