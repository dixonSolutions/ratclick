//! The GNOME (gsettings) shortcut backend.
//!
//! Installs RatClick's toggle as a GNOME *custom keybinding* — the same thing
//! Settings ▸ Keyboard ▸ Custom Shortcuts creates — and scans every schema
//! GNOME keeps keybindings in so we can tell the user what a shortcut would
//! collide with before taking it.

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{Context, Result};

use super::gvariant::{format_string, format_string_list, parse_string_list};
use crate::accel::Accel;

const MEDIA_KEYS_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const CUSTOM_PATH_ROOT: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings";

/// Prefix for the custom-keybinding paths RatClick owns. Anything under this
/// prefix is ours and is safe to rewrite or delete.
const OUR_PATH_PREFIX: &str = "ratclick";

/// Schemas that hold keyboard shortcuts. Scanned in this order so the most
/// recognisable owner (window manager, then shell) is reported first.
const KEYBINDING_SCHEMAS: &[&str] = &[
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.shell.keybindings",
    "org.gnome.mutter.keybindings",
    "org.gnome.mutter.wayland.keybindings",
    MEDIA_KEYS_SCHEMA,
];

/// Keys inside the scanned schemas whose values are not accelerators.
const NOT_ACCELERATORS: &[&str] = &["custom-keybindings", "available", "active"];

/// One accelerator that some part of the desktop has already claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub accel: Accel,
    /// Where it lives, for both display and for taking it away again.
    pub owner: Owner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// A plain `schema key` pair holding an array of accelerators.
    Setting { schema: String, key: String },
    /// A custom keybinding, identified by its dconf path.
    Custom {
        path: String,
        name: String,
        command: String,
    },
}

impl Owner {
    /// Human-readable description for conflict messages.
    pub fn describe(&self) -> String {
        match self {
            Owner::Setting { schema, key } => {
                let area = match schema.as_str() {
                    "org.gnome.desktop.wm.keybindings" => "Window management",
                    "org.gnome.shell.keybindings" => "GNOME Shell",
                    "org.gnome.mutter.keybindings" | "org.gnome.mutter.wayland.keybindings" => {
                        "Mutter"
                    }
                    MEDIA_KEYS_SCHEMA => "System shortcuts",
                    _ => "GNOME",
                };
                format!("{area}: {}", key.replace('-', " "))
            }
            Owner::Custom { name, command, .. } => {
                if name.is_empty() {
                    format!("Custom shortcut: {command}")
                } else {
                    format!("Custom shortcut “{name}” ({command})")
                }
            }
        }
    }

    /// Whether this binding is one RatClick installed itself.
    pub fn is_ours(&self) -> bool {
        match self {
            Owner::Custom { path, .. } => path.contains(OUR_PATH_PREFIX),
            Owner::Setting { .. } => false,
        }
    }
}

/// Is the gsettings backend usable on this machine?
pub fn is_available() -> bool {
    which(super::gsettings_bin()).is_some() && schema_exists(MEDIA_KEYS_SCHEMA)
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::path::Path::new(bin);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(bin))
            .find(|p| p.is_file())
    })
}

fn schema_exists(schema: &str) -> bool {
    Command::new(super::gsettings_bin())
        .args(["list-keys", schema])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn gsettings(args: &[&str]) -> Result<String> {
    let out = Command::new(super::gsettings_bin())
        .args(args)
        .output()
        .with_context(|| format!("running `gsettings {}`", args.join(" ")))?;
    if !out.status.success() {
        anyhow::bail!(
            "`gsettings {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Best-effort variant: a schema that is not installed is not an error, it just
/// contributes no bindings.
fn gsettings_opt(args: &[&str]) -> Option<String> {
    let out = Command::new(super::gsettings_bin())
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Every accelerator currently bound anywhere GNOME keeps them.
pub fn scan() -> Vec<Binding> {
    let mut found = Vec::new();

    for schema in KEYBINDING_SCHEMAS {
        let Some(dump) = gsettings_opt(&["list-recursively", schema]) else {
            continue;
        };
        for line in dump.lines() {
            // Each line is `<schema> <key> <gvariant value>`.
            let mut parts = line.splitn(3, ' ');
            let (Some(sch), Some(key), Some(value)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if NOT_ACCELERATORS.contains(&key) {
                continue;
            }
            for raw in parse_string_list(value) {
                // GNOME writes a disabled binding as the literal string
                // "disabled" or an empty string.
                if raw.is_empty() || raw == "disabled" {
                    continue;
                }
                if let Ok(accel) = Accel::parse(&raw) {
                    found.push(Binding {
                        accel,
                        owner: Owner::Setting {
                            schema: sch.to_string(),
                            key: key.to_string(),
                        },
                    });
                }
            }
        }
    }

    for path in custom_paths() {
        let (name, command, bindings) = read_custom(&path);
        for raw in bindings {
            if raw.is_empty() || raw == "disabled" {
                continue;
            }
            if let Ok(accel) = Accel::parse(&raw) {
                found.push(Binding {
                    accel,
                    owner: Owner::Custom {
                        path: path.clone(),
                        name: name.clone(),
                        command: command.clone(),
                    },
                });
            }
        }
    }

    found
}

/// Bindings that would collide with `accel`, excluding RatClick's own.
pub fn conflicts_for(accel: &Accel) -> Vec<Binding> {
    scan()
        .into_iter()
        .filter(|b| &b.accel == accel && !b.owner.is_ours())
        .collect()
}

/// The user's custom-keybinding paths, or `None` if the list could not be read.
///
/// The distinction matters. `install` rewrites this list, so treating an
/// unreadable list as an empty one would delete every custom shortcut the user
/// has. Callers that only need to *look* at the list can flatten to empty;
/// callers that write it must refuse instead.
fn custom_paths_checked() -> Option<Vec<String>> {
    gsettings_opt(&["get", MEDIA_KEYS_SCHEMA, "custom-keybindings"]).map(|v| parse_string_list(&v))
}

fn custom_paths() -> Vec<String> {
    custom_paths_checked().unwrap_or_default()
}

fn custom_schema_for(path: &str) -> String {
    format!("{CUSTOM_SCHEMA}:{path}")
}

fn read_custom(path: &str) -> (String, String, Vec<String>) {
    let schema = custom_schema_for(path);
    let get = |key: &str| {
        gsettings_opt(&["get", &schema, key])
            .map(|v| parse_string_list(&v).into_iter().next().unwrap_or_default())
            .unwrap_or_default()
    };
    let bindings = gsettings_opt(&["get", &schema, "binding"])
        .map(|v| parse_string_list(&v))
        .unwrap_or_default();
    (get("name"), get("command"), bindings)
}

/// Remove `accel` from whatever currently holds it.
///
/// Returns a description of every binding that was changed, so the caller can
/// tell the user exactly what it took. GNOME's own settings are *edited* (the
/// accelerator is dropped from the list, leaving the action bound to whatever
/// else it had); foreign custom shortcuts are edited the same way, and are only
/// deleted outright if that would leave them with no accelerator at all.
pub fn steal(accel: &Accel) -> Result<Vec<String>> {
    let mut stolen = Vec::new();

    for binding in conflicts_for(accel) {
        match &binding.owner {
            Owner::Setting { schema, key } => {
                let current = gsettings(&["get", schema, key])?;
                let kept: Vec<String> = parse_string_list(&current)
                    .into_iter()
                    .filter(|raw| Accel::parse(raw).as_ref() != Ok(accel))
                    .collect();
                gsettings(&["set", schema, key, &format_string_list(&kept)])?;
                stolen.push(binding.owner.describe());
            }
            Owner::Custom { path, .. } => {
                // A custom keybinding's `binding` is a single string, so taking
                // its accelerator always leaves it with none. Rather than
                // leaving a shortcut that is bound to nothing and looks broken
                // in Settings, drop the whole entry.
                remove_custom_path(path)?;
                stolen.push(binding.owner.describe());
            }
        }
    }

    Ok(stolen)
}

/// Install `accels` as RatClick custom keybindings running `command`.
///
/// Replaces any previous RatClick bindings, so calling this repeatedly is safe
/// and never accumulates stale entries.
pub fn install(accels: &[Accel], command: &str) -> Result<()> {
    uninstall()?;
    if accels.is_empty() {
        return Ok(());
    }

    // Read the existing list before touching anything. We are about to write it
    // back with our entries appended, so a failed read must abort rather than
    // be treated as "no custom shortcuts" — that would silently delete every
    // custom keybinding the user has.
    let mut paths = custom_paths_checked().context(
        "could not read the existing custom keyboard shortcuts; refusing to overwrite them",
    )?;
    for (i, accel) in accels.iter().enumerate() {
        let path = format!("{CUSTOM_PATH_ROOT}/{OUR_PATH_PREFIX}{i}/");
        let schema = custom_schema_for(&path);
        let label = if accels.len() == 1 {
            "RatClick: toggle clicking".to_string()
        } else {
            format!("RatClick: toggle clicking ({})", i + 1)
        };
        gsettings(&["set", &schema, "name", &format_string(&label)])?;
        gsettings(&["set", &schema, "command", &format_string(command)])?;
        // `binding` is type `s`, not `as` — a custom keybinding holds exactly
        // one accelerator, which is why each accelerator gets its own path.
        // Writing an array literal here is accepted by gsettings (it is a valid
        // string) but stores the literal text `['<Super>c']`, which nothing
        // matches against.
        gsettings(&["set", &schema, "binding", &format_string(&accel.to_gtk())])?;
        if !paths.contains(&path) {
            paths.push(path);
        }
    }

    gsettings(&[
        "set",
        MEDIA_KEYS_SCHEMA,
        "custom-keybindings",
        &format_string_list(&paths),
    ])?;
    Ok(())
}

/// Remove every custom keybinding RatClick owns.
pub fn uninstall() -> Result<()> {
    // Same reasoning as `install`: this rewrites the list, so an unreadable
    // list must not be mistaken for an empty one.
    let paths = custom_paths_checked().context(
        "could not read the existing custom keyboard shortcuts; refusing to overwrite them",
    )?;
    let (ours, theirs): (Vec<String>, Vec<String>) =
        paths.into_iter().partition(|p| p.contains(OUR_PATH_PREFIX));
    if ours.is_empty() {
        return Ok(());
    }
    gsettings(&[
        "set",
        MEDIA_KEYS_SCHEMA,
        "custom-keybindings",
        &format_string_list(&theirs),
    ])?;
    for path in ours {
        reset_custom(&path);
    }
    Ok(())
}

fn remove_custom_path(path: &str) -> Result<()> {
    let kept: Vec<String> = custom_paths().into_iter().filter(|p| p != path).collect();
    gsettings(&[
        "set",
        MEDIA_KEYS_SCHEMA,
        "custom-keybindings",
        &format_string_list(&kept),
    ])?;
    reset_custom(path);
    Ok(())
}

/// Clear the keys under a custom-keybinding path.
///
/// Best-effort: an already-empty path makes `gsettings reset` complain, and a
/// leftover dconf entry is harmless once the path is off the list.
fn reset_custom(path: &str) {
    let schema = custom_schema_for(path);
    for key in ["binding", "command", "name"] {
        let _ = gsettings_opt(&["reset", &schema, key]);
    }
}

/// What RatClick currently has installed, as accelerators.
pub fn installed() -> Vec<Accel> {
    let mut out = Vec::new();
    let mut paths: Vec<String> = custom_paths()
        .into_iter()
        .filter(|p| p.contains(OUR_PATH_PREFIX))
        .collect();
    paths.sort();
    for path in paths {
        let (_, _, bindings) = read_custom(&path);
        for raw in bindings {
            if let Ok(a) = Accel::parse(&raw) {
                out.push(a);
            }
        }
    }
    out
}

/// Group a scan by accelerator — used by `ratclick shortcut list`.
pub fn by_accel() -> BTreeMap<Accel, Vec<Owner>> {
    let mut map: BTreeMap<Accel, Vec<Owner>> = BTreeMap::new();
    for b in scan() {
        map.entry(b.accel).or_default().push(b.owner);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_paths_are_recognised_as_ours() {
        let ours = Owner::Custom {
            path: format!("{CUSTOM_PATH_ROOT}/ratclick0/"),
            name: String::new(),
            command: String::new(),
        };
        assert!(ours.is_ours());

        let theirs = Owner::Custom {
            path: format!("{CUSTOM_PATH_ROOT}/custom0/"),
            name: "Terminal".into(),
            command: "gnome-terminal".into(),
        };
        assert!(!theirs.is_ours());
    }

    #[test]
    fn setting_owners_describe_their_area() {
        let o = Owner::Setting {
            schema: "org.gnome.desktop.wm.keybindings".into(),
            key: "toggle-maximized".into(),
        };
        assert_eq!(o.describe(), "Window management: toggle maximized");
    }

    #[test]
    fn custom_owners_name_themselves() {
        let o = Owner::Custom {
            path: "/x/".into(),
            name: "Screenshot".into(),
            command: "flameshot".into(),
        };
        assert!(o.describe().contains("Screenshot"));
        assert!(o.describe().contains("flameshot"));
    }
}
