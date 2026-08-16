/* effects.js
 *
 * Transient on-screen feedback drawn at the pointer when the clicker starts
 * or stops.
 *
 * Why this lives in the shell
 * ---------------------------
 * On Wayland only the compositor may put something on screen at an arbitrary
 * position, so the daemon cannot draw its own overlay. It only says *what* to
 * draw — the `EffectOn`/`EffectOff` properties on its interface; the drawing
 * happens here.
 *
 * Lifetime
 * --------
 * Every effect is one top-level actor parented to `uiGroup`, tracked in
 * `_actors`, and destroyed from the `onComplete` of its own last transition.
 * Sequencing (the ripple stagger, the logo's hold) uses Clutter transition
 * *delays* rather than GLib timeouts: a delay lives on the actor, so
 * destroying the actor cancels it, and there is no separate source that could
 * outlive `disable()`. `destroyAll()` therefore only has to sweep actors.
 *
 * Everything is added with `uiGroup.add_child()` rather than
 * `Main.layoutManager.addChrome()`, which keeps the actors out of the input
 * region: an effect that swallowed a click would be worse than no effect.
 */

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

/* The whole point of the feature: the same shape has to read as "started" or
 * "stopped" without the user having to think about it. */
const COLOR_ON = '#2ec27e';
const COLOR_OFF = '#e01b24';

/* Shipped with the RatClick application; falls back to a stock icon. */
const ICON_NAME = 'io.github.dixonsolutions.RatClick-symbolic';
const FALLBACK_ICON_NAME = 'input-mouse-symbolic';

/* Lengths here are *logical* pixels. St scales CSS lengths by the UI scale
 * factor itself, so only the geometry set directly on an actor gets multiplied
 * — see `_scaleFactor()`. */
const RIPPLE = {
    rings: 3,
    diameter: 180,
    stroke: 3,
    stagger: 90,
    duration: 520,
    opacity: 235,
    /* Never exactly zero: a zero scale collapses the actor's paint volume and
     * the first frames are dropped. */
    minScale: 0.02,
};

const PULSE = {
    fromDiameter: 40,
    toDiameter: 160,
    duration: 350,
    /* 0.45 alpha as a Clutter 0-255 opacity. */
    opacity: 115,
};

const LOGO = {
    size: 96,
    /* Above and to the right of the pointer, so the cursor itself and whatever
     * it is hovering stay visible. */
    offsetX: 26,
    offsetY: -86,
    scaleFrom: 0.6,
    fadeIn: 180,
    hold: 220,
    fadeOut: 260,
    drift: 20,
};

/* A spammed toggle must not pile up actors. Three concurrent effects is enough
 * that a fast on/off/on still looks continuous. */
const MAX_CONCURRENT = 3;

/** The effect names the daemon is allowed to ask for. */
export const EFFECT_NAMES = ['none', 'ripple', 'pulse', 'logo'];

/** Every live effect actor, in creation order. */
const _actors = new Set();

/**
 * @returns {number} the current UI scale factor (2 on HiDPI, 1 otherwise)
 */
function _scaleFactor() {
    return St.ThemeContext.get_for_stage(global.stage).scale_factor;
}

/**
 * Put a freshly built effect on the stage and take ownership of it.
 *
 * Called *before* any transition is started: a Clutter transition only
 * advances once its actor has a frame clock, which it gets from its parent.
 *
 * @param {Clutter.Actor} actor - the effect's root actor
 * @param {string} name - actor name, also read by the test harness
 */
function _mount(actor, name) {
    /* Sets iterate in insertion order, so the first entry is the oldest. */
    while (_actors.size >= MAX_CONCURRENT) {
        const oldest = _actors.values().next().value;
        _actors.delete(oldest);
        oldest.destroy();
    }

    actor.name = name;
    _actors.add(actor);
    actor.connect('destroy', () => _actors.delete(actor));
    Main.layoutManager.uiGroup.add_child(actor);
}

/**
 * Common construction properties for anything we put on the stage: it is
 * decoration, so it must be invisible to input and to the focus chain.
 *
 * @param {object} props - the effect-specific properties
 * @returns {object} properties for a St.Widget/St.Icon constructor
 */
function _inert(props) {
    return {
        reactive: false,
        can_focus: false,
        track_hover: false,
        ...props,
    };
}

/**
 * Three concentric rings that expand out of the pointer when clicking starts
 * and collapse back into it when clicking stops, so the direction alone
 * distinguishes the two even before the colour registers.
 *
 * @param {number} x - pointer x in stage coordinates
 * @param {number} y - pointer y in stage coordinates
 * @param {boolean} isOn - whether this is the start-clicking effect
 * @param {string} color - CSS colour
 * @param {string} name - actor name
 * @returns {Clutter.Actor} the effect's root actor
 */
function _ripple(x, y, isOn, color, name) {
    const size = RIPPLE.diameter * _scaleFactor();

    const group = new Clutter.Actor({
        reactive: false,
        x: Math.round(x - size / 2),
        y: Math.round(y - size / 2),
        width: size,
        height: size,
    });
    _mount(group, name);

    const [from, to] = isOn ? [RIPPLE.minScale, 1] : [1, RIPPLE.minScale];

    for (let i = 0; i < RIPPLE.rings; i++) {
        const ring = new St.Widget(_inert({
            width: size,
            height: size,
            style: `border: ${RIPPLE.stroke}px solid ${color};` +
                `border-radius: ${RIPPLE.diameter / 2}px;`,
            opacity: RIPPLE.opacity,
        }));
        ring.set_pivot_point(0.5, 0.5);
        ring.set_scale(from, from);
        group.add_child(ring);

        /* One ease per ring: a second ease() on the same actor would cancel
         * the transitions of any property it also animates. */
        const isLast = i === RIPPLE.rings - 1;
        ring.ease({
            scale_x: to,
            scale_y: to,
            opacity: 0,
            duration: RIPPLE.duration,
            delay: i * RIPPLE.stagger,
            mode: isOn
                ? Clutter.AnimationMode.EASE_OUT_QUAD
                : Clutter.AnimationMode.EASE_IN_QUAD,
            /* The last ring starts last and so finishes last. */
            onComplete: isLast ? () => group.destroy() : null,
        });
    }

    return group;
}

/**
 * A single filled disc: snappier and quieter than the ripple.
 *
 * @param {number} x - pointer x in stage coordinates
 * @param {number} y - pointer y in stage coordinates
 * @param {boolean} isOn - whether this is the start-clicking effect
 * @param {string} color - CSS colour
 * @param {string} name - actor name
 * @returns {Clutter.Actor} the effect's root actor
 */
function _pulse(x, y, isOn, color, name) {
    const size = PULSE.toDiameter * _scaleFactor();
    const small = PULSE.fromDiameter / PULSE.toDiameter;
    const [from, to] = isOn ? [small, 1] : [1, small];

    const disc = new St.Widget(_inert({
        x: Math.round(x - size / 2),
        y: Math.round(y - size / 2),
        width: size,
        height: size,
        style: `background-color: ${color};` +
            `border-radius: ${PULSE.toDiameter / 2}px;`,
        opacity: PULSE.opacity,
    }));
    disc.set_pivot_point(0.5, 0.5);
    disc.set_scale(from, from);
    _mount(disc, name);

    disc.ease({
        scale_x: to,
        scale_y: to,
        opacity: 0,
        duration: PULSE.duration,
        mode: Clutter.AnimationMode.EASE_OUT_QUAD,
        onComplete: () => disc.destroy(),
    });

    return disc;
}

/**
 * The RatClick icon, tinted, popping in beside the pointer and drifting off.
 *
 * @param {number} x - pointer x in stage coordinates
 * @param {number} y - pointer y in stage coordinates
 * @param {boolean} isOn - whether this is the start-clicking effect
 * @param {string} color - CSS colour
 * @param {string} name - actor name
 * @returns {Clutter.Actor} the effect's root actor
 */
function _logo(x, y, isOn, color, name) {
    const factor = _scaleFactor();

    const icon = new St.Icon(_inert({
        /* A GThemedIcon with several names resolves to the first one the icon
         * theme actually has, which gives us the fallback for free. */
        gicon: Gio.ThemedIcon.new_from_names([ICON_NAME, FALLBACK_ICON_NAME]),
        /* St.Icon sizes in logical pixels and applies the scale factor. */
        icon_size: LOGO.size,
        style: `color: ${color};`,
        opacity: 0,
    }));
    icon.set_position(
        Math.round(x + LOGO.offsetX * factor),
        Math.round(y + LOGO.offsetY * factor));
    icon.set_pivot_point(0.5, 0.5);
    icon.set_scale(LOGO.scaleFrom, LOGO.scaleFrom);
    _mount(icon, name);

    const driftTo = icon.y - LOGO.drift * factor;

    icon.ease({
        opacity: 255,
        scale_x: 1,
        scale_y: 1,
        duration: LOGO.fadeIn,
        /* A little overshoot on the way in; stopping stays flat so it does not
         * read as celebratory. */
        mode: isOn
            ? Clutter.AnimationMode.EASE_OUT_BACK
            : Clutter.AnimationMode.EASE_OUT_QUAD,
        onComplete: () => {
            icon.ease({
                opacity: 0,
                y: driftTo,
                delay: LOGO.hold,
                duration: LOGO.fadeOut,
                mode: Clutter.AnimationMode.EASE_IN_QUAD,
                onComplete: () => icon.destroy(),
            });
        },
    });

    return icon;
}

const _BUILDERS = {
    ripple: _ripple,
    pulse: _pulse,
    logo: _logo,
};

/**
 * Play one effect at the current pointer position.
 *
 * Never throws: an unknown effect name from the daemon, or a shell with
 * animations switched off, is a no-op rather than an error.
 *
 * @param {string} name - one of `EFFECT_NAMES`
 * @param {object} [options] - options
 * @param {boolean} [options.isOn] - true for "clicking started", false for
 *   "clicking stopped"; selects both the colour and the direction
 * @returns {?Clutter.Actor} the root actor, or null if nothing was drawn
 */
export function playEffect(name, {isOn = true} = {}) {
    /* 'none' must not even allocate an actor. */
    if (!name || name === 'none')
        return null;

    const build = _BUILDERS[name];
    if (!build) {
        console.debug(`RatClick: unknown effect '${name}'`);
        return null;
    }

    /* With animations off every duration collapses to zero, which would leave
     * a fully drawn actor on screen for one frame. Skip instead. */
    if (!St.Settings.get().enable_animations)
        return null;

    const [x, y] = global.get_pointer();
    return build(x, y, isOn, isOn ? COLOR_ON : COLOR_OFF,
        `ratclick-effect-${name}-${isOn ? 'on' : 'off'}`);
}

/**
 * Destroy every live effect. Must leave nothing behind: `disable()` calls it,
 * and the shell may unload this module straight afterwards.
 */
export function destroyAll() {
    for (const actor of [..._actors])
        actor.destroy();
    _actors.clear();
}

/**
 * @returns {number} how many effect actors are on the stage right now
 */
export function liveCount() {
    return _actors.size;
}
