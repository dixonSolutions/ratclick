ratclick(1)

# NAME

ratclick - a desktop-independent auto-clicker for Linux

# SYNOPSIS

*ratclick* [_COMMAND_]

# DESCRIPTION

RatClick clicks your mouse for you, at a rate and for a length of time you
choose. It is made of three parts:

- *ratclick*, this command line;
- *ratclick-gui*, a libadwaita window;
- *ratclickd*, a background service that owns the virtual pointer and the
  session D-Bus name _io.github.dixonsolutions.RatClick.Daemon_. (The app
  itself owns _io.github.dixonsolutions.RatClick_, so the two must differ.)

Running *ratclick* with no command prints this help. Use *ratclick gui* to open
the window.

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

Read and change settings. With no subcommand, *config* opens the file in an
editor.

*config* [*edit*]
	Open the configuration file in *$VISUAL*, or *$EDITOR*, falling back to
	*nano* and then *vi*. The file is created with its defaults first if it
	does not exist yet, so every key is visible.

	On save the file is parsed before it is accepted. If it will not parse,
	the error is shown and you are offered the editor again — RatClick will
	not adopt a configuration it cannot read. The running service is told to
	reload once the file is good.

*config read*
	Print the configuration file exactly as it is on disk, comments and all.
	Contrast with *config show*, which prints an interpreted summary.

*config show*
	Print the current configuration as a readable summary.

*config path*
	Print the path of the configuration file.

*config set* _KEY_ _VALUE_
	Change a single setting. _KEY_ is one of:

	- *cpm* - clicks per minute, a whole number.
	- *button* - _left_, _right_ or _middle_.
	- *mode* - _endless_ or _timed_.
	- *duration* - how long a timed run lasts. Accepts _90_, _1h30m_ or
	  _1:30_. Setting it switches *mode* to _timed_.
	- *hours* - the hours part of the duration. Switches to _timed_.
	- *minutes* - the minutes part of the duration, 0-59. Switches to
	  _timed_.
	- *autostart* - _on_ or _off_; start clicking as soon as the service
	  starts.
	- *effects* - _on_ or _off_; master switch for the toggle effects.
	- *effect-on*, *effect-off* - _none_, _ripple_, _pulse_ or _logo_. See
	  *TOGGLE EFFECTS* below.

*config reset* [*--yes*]
	Delete the configuration, so the next run starts the wizard again. This
	also unbinds the shortcut. *--yes* skips the confirmation prompt.

## shortcut

Manage the global toggle shortcut. RatClick can register it through GNOME's own
custom-keybinding list, through the bundled GNOME Shell extension, or through
*keyd*(1) for a binding that works on any Linux desktop and outside the
graphical session. Clicking itself is desktop-independent; only shortcut
registration varies between desktop environments.

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

_/usr/share/dbus-1/services/io.github.dixonsolutions.RatClick.Daemon.service_
	D-Bus session activation, so the service starts on demand.

_/usr/lib/udev/rules.d/60-ratclick-uinput.rules_
	Gives the *input* group write access to _/dev/uinput_.

_/usr/libexec/ratclick/ratclick-keyd-dispatch_
	Helper that lets a root-owned keyd binding reach the logged-in user's
	session bus.

# TOGGLE EFFECTS

RatClick can draw a brief flourish at the pointer when clicking starts or
stops, so a toggle you fired from a keyboard shortcut is visibly acknowledged
even when the window is not on screen.

Only the compositor can draw over other windows on Wayland, so the effects are
rendered by the RatClick GNOME Shell extension. Without it installed the
settings are accepted and stored but nothing is drawn.

The start effect is drawn in green and the stop effect in red, so the two are
distinguishable even when they use the same shape.

*none*
	Draw nothing.

*ripple*
	Concentric rings, expanding outward when clicking starts and contracting
	inward when it stops.

*pulse*
	A single disc that expands and fades. Quicker and less busy than
	*ripple*.

*logo*
	The RatClick icon fading in beside the pointer and drifting upward.

Set them with *config set effects on*, *config set effect-on ripple* and
*config set effect-off pulse*, or from the *Toggle effect* section of the app.

# PERMISSIONS

The daemon creates a virtual pointer through _/dev/uinput_, which is root-only
by default. The shipped udev rule grants the *input* group write access. Add
yourself to it and log in again:

```
sudo usermod -aG input $USER
```

*ratclick doctor* tells you whether this is already in place.

# ENVIRONMENT

*VISUAL*, *EDITOR*
	Which editor *config edit* opens. *VISUAL* wins; *nano* then *vi* are the
	fallbacks. The value is run through the shell, so
	_EDITOR="code -w"_ works.

*RATCLICK_CONFIG*
	Overrides the configuration file path. Used by the test harness, and by
	*sudo*/*pkexec* escalation so the privileged step edits your
	configuration rather than root's.

# EXIT STATUS

*ratclick* exits 0 on success and 1 on failure, printing the reason to standard
error.

# SEE ALSO

*ratclickd*(1), *keyd*(1), *systemctl*(1), *udev*(7)

# BUGS

https://github.com/dixonSolutions/ratclick/issues
