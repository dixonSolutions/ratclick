//! The keyd shortcut backend.
//!
//! keyd binds keys system-wide at the evdev layer, so a RatClick shortcut
//! installed this way works outside GNOME, on another compositor, and on a TTY.
//!
//! # Why we edit existing files instead of dropping in our own
//!
//! keyd matches each keyboard against the `[ids]` section of the configs in
//! `/etc/keyd`, and when several configs match a device only one of them wins.
//! A fresh `ratclick.conf` with `[ids] *` would therefore lose to a pre-existing
//! `default.conf` with the same wildcard and silently do nothing. So instead we
//! append a clearly delimited block to *every* config present, which means
//! whichever config a given keyboard ends up using, our binding is in it. The
//! markers make removal exact.
//!
//! # Why the binding is a layer and not a prefixed key
//!
//! keyd does not accept modifier prefixes on the left of a mapping: `M-S-f12 =`
//! is rejected with "not a valid key or alias". A modified shortcut has to be
//! written as the bare key inside the layer named by its modifiers, so
//! Super+Shift+F12 becomes `[shift+meta]` / `f12 = …`. That is also why the
//! block is appended rather than inserted: it declares layer sections of its
//! own, and dropping those into the middle of a file would silently swallow
//! every following line of the user's config into our last layer.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::accel::Accel;

pub const CONFIG_DIR: &str = "/etc/keyd";
const BEGIN: &str = "# >>> ratclick begin (managed by RatClick — do not edit) >>>";
const END: &str = "# <<< ratclick end <<<";

/// Directory keyd reads, overridable so tests never touch the real one.
pub fn config_dir() -> PathBuf {
    std::env::var_os("RATCLICK_KEYD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(CONFIG_DIR))
}

/// Why keyd cannot be used, when it cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Ready {
        version: String,
    },
    NotInstalled,
    /// Installed but the service is not running, so bindings would not apply.
    ServiceStopped,
}

impl Availability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Availability::Ready { .. })
    }

    pub fn explain(&self) -> String {
        match self {
            Availability::Ready { version } => format!("keyd {version} is running"),
            Availability::NotInstalled => {
                "keyd is not installed (`apt install keyd` / `dnf install keyd`)".into()
            }
            Availability::ServiceStopped => {
                "keyd is installed but not running (`sudo systemctl enable --now keyd`)".into()
            }
        }
    }
}

pub fn availability() -> Availability {
    let Ok(out) = Command::new("keyd").arg("--version").output() else {
        return Availability::NotInstalled;
    };
    if !out.status.success() {
        return Availability::NotInstalled;
    }
    let version = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .unwrap_or("?")
        .trim_start_matches('v')
        .to_string();

    let running = Command::new("systemctl")
        .args(["is-active", "--quiet", "keyd"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if running {
        Availability::Ready { version }
    } else {
        Availability::ServiceStopped
    }
}

/// A binding found in a keyd config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub accel: Accel,
    pub file: PathBuf,
    /// The layer it was declared in, e.g. `main`.
    pub layer: String,
    /// Right-hand side of the mapping, for display.
    pub action: String,
    /// True when the line sits inside RatClick's own managed block.
    pub ours: bool,
}

impl Binding {
    pub fn describe(&self) -> String {
        format!(
            "keyd [{}] in {}: {} → {}",
            self.layer,
            self.file.file_name().unwrap_or_default().to_string_lossy(),
            self.accel.to_keyd(),
            self.action
        )
    }
}

fn config_files() -> Vec<PathBuf> {
    let dir = config_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "conf"))
        .collect();
    files.sort();
    files
}

/// Parse every mapping keyd currently has.
pub fn scan() -> Vec<Binding> {
    let mut out = Vec::new();
    for file in config_files() {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        out.extend(parse_config(&text, &file));
    }
    out
}

fn parse_config(text: &str, file: &Path) -> Vec<Binding> {
    let mut out = Vec::new();
    let mut layer = String::from("main");
    let mut in_ids = false;
    let mut ours = false;

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed == BEGIN {
            ours = true;
            continue;
        }
        if trimmed == END {
            ours = false;
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(section) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_ids = section == "ids";
            if !in_ids {
                // Layer headers can carry a base layer (`[nav:C]`); the name is
                // everything before the colon.
                layer = section.split(':').next().unwrap_or(section).to_string();
            }
            continue;
        }
        if in_ids {
            continue;
        }

        let Some((lhs, rhs)) = trimmed.split_once('=') else {
            continue;
        };

        // The layer supplies the modifiers; the left-hand side is a bare key.
        // A layer that is not built from modifier layers (`[nav]`, say) is only
        // reachable while another key is held, so nothing in it can collide
        // with a global shortcut.
        let Some(mods) = Accel::modifiers_from_keyd_layer(&layer) else {
            continue;
        };
        let Ok(key) = Accel::parse(lhs.trim()) else {
            continue;
        };
        // A prefixed left-hand side is invalid keyd; if one is present the line
        // is broken anyway, so do not pretend to understand it.
        if !key.mods.is_empty() {
            continue;
        }

        out.push(Binding {
            accel: Accel::new(mods, key.key),
            file: file.to_path_buf(),
            layer: layer.clone(),
            action: rhs.trim().to_string(),
            ours,
        });
    }
    out
}

/// Bindings that would collide with `accel`, excluding RatClick's own.
pub fn conflicts_for(accel: &Accel) -> Vec<Binding> {
    scan()
        .into_iter()
        .filter(|b| &b.accel == accel && !b.ours)
        .collect()
}

/// Accelerators RatClick currently has installed in keyd.
pub fn installed() -> Vec<Accel> {
    let mut seen: Vec<Accel> = Vec::new();
    for b in scan().into_iter().filter(|b| b.ours) {
        if !seen.contains(&b.accel) {
            seen.push(b.accel);
        }
    }
    seen
}

/// Build the block RatClick appends to a keyd config.
///
/// Each accelerator becomes a bare key inside the modifier layer its modifiers
/// name, because that is the only left-hand-side form keyd accepts. Bindings
/// are grouped so one layer is declared once even with several accelerators.
fn managed_block(accels: &[Accel], command: &str) -> String {
    // BTreeMap keeps the output deterministic, which matters because we compare
    // the rendered file against what is on disk to decide whether to write.
    let mut by_layer: std::collections::BTreeMap<String, Vec<&Accel>> = Default::default();
    for accel in accels {
        by_layer
            .entry(accel.to_keyd_layer().unwrap_or_else(|| "main".to_string()))
            .or_default()
            .push(accel);
    }

    let mut s = String::from(BEGIN);
    s.push('\n');
    for (layer, accels) in by_layer {
        s.push_str(&format!("[{layer}]\n"));
        for accel in accels {
            // keyd's `command()` runs the string through /bin/sh as root, so
            // the dispatcher is responsible for dropping into the user session.
            s.push_str(&format!("{} = command({command})\n", accel.key));
        }
    }
    s.push_str(END);
    s.push('\n');
    s
}

/// Remove any existing managed block from `text`.
fn strip_managed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut skipping = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == BEGIN {
            skipping = true;
            continue;
        }
        if trimmed == END {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    // Collapse the blank-line gap a removed block leaves behind, including the
    // separator line `splice` puts before it, so removing and re-adding the
    // block is byte-for-byte reversible.
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Append `block` to `text`, replacing any previous managed block.
///
/// The block goes at the *end* of the file rather than inside `[main]`, because
/// it declares its own layer sections and inserting those mid-file would
/// silently move every following line of the user's config into our last
/// layer. Position is otherwise irrelevant: keyd merges all `[ids]` sections in
/// a file into one device-match list, so layers declared anywhere in the file
/// apply to every device that file matches.
fn splice(text: &str, block: &str) -> String {
    let cleaned = strip_managed(text);
    let has_ids = cleaned.lines().any(|l| l.trim() == "[ids]");

    let mut out = cleaned;
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    // A config with no `[ids]` matches no devices at all, so supply the
    // wildcard when we are creating the file from scratch.
    if !has_ids {
        out.push_str("[ids]\n*\n");
    }
    out.push('\n');
    out.push_str(block);
    out
}

fn require_root() -> Result<()> {
    // SAFETY: geteuid is always safe; it takes no arguments and cannot fail.
    let euid = unsafe { libc_geteuid() };
    if euid != 0 {
        anyhow::bail!(
            "writing {} needs root — re-run with sudo, or let the GUI ask via pkexec",
            config_dir().display()
        );
    }
    Ok(())
}

// Avoid pulling in the whole `libc` crate for one call.
extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

/// Install `accels` into every keyd config, pointing at `command`.
pub fn install(accels: &[Accel], command: &str) -> Result<Vec<PathBuf>> {
    require_root()?;
    let dir = config_dir();
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let block = managed_block(accels, command);
    let mut files = config_files();
    if files.is_empty() {
        files.push(dir.join("default.conf"));
    }

    let mut written = Vec::new();
    for file in files {
        let existing = fs::read_to_string(&file).unwrap_or_default();
        let updated = if accels.is_empty() {
            strip_managed(&existing)
        } else {
            splice(&existing, &block)
        };
        if updated != existing {
            write_atomic(&file, &updated)?;
            written.push(file);
        }
    }

    reload()?;
    Ok(written)
}

/// Remove RatClick's block from every keyd config.
pub fn uninstall() -> Result<Vec<PathBuf>> {
    require_root()?;
    let mut written = Vec::new();
    for file in config_files() {
        let existing = fs::read_to_string(&file).unwrap_or_default();
        if !existing.contains(BEGIN) {
            continue;
        }
        let updated = strip_managed(&existing);
        write_atomic(&file, &updated)?;
        written.push(file);
    }
    if !written.is_empty() {
        reload()?;
    }
    Ok(written)
}

fn write_atomic(path: &Path, body: &str) -> Result<()> {
    let tmp = path.with_extension("conf.ratclick-tmp");
    fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Ask keyd to re-read its configuration.
///
/// A failure here is reported but not fatal: the config on disk is already
/// correct and will be picked up on the next keyd restart.
pub fn reload() -> Result<()> {
    let out = Command::new("keyd").arg("reload").output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            tracing::warn!(
                "keyd reload failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
            Ok(())
        }
        Err(e) => {
            tracing::warn!("could not run `keyd reload`: {e}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accel(s: &str) -> Accel {
        Accel::parse(s).unwrap()
    }

    #[test]
    fn a_modified_key_becomes_a_modifier_layer() {
        let block = managed_block(&[accel("<Super><Shift>c")], "/usr/bin/dispatch toggle");

        // keyd rejects `S-M-c = ...` outright; the key must be bare inside the
        // layer its modifiers name.
        assert!(block.contains("[shift+meta]"));
        assert!(block.contains("c = command(/usr/bin/dispatch toggle)"));
        assert!(!block.contains("S-M-c ="));
    }

    #[test]
    fn an_unmodified_key_stays_in_main() {
        let block = managed_block(&[accel("F9")], "cmd");
        assert!(block.contains("[main]"));
        assert!(block.contains("f9 = command(cmd)"));
    }

    #[test]
    fn accelerators_sharing_a_layer_are_grouped_under_one_header() {
        let block = managed_block(&[accel("<Super>F9"), accel("<Super>F10")], "cmd");
        assert_eq!(block.matches("[meta]").count(), 1);
        assert!(block.contains("f9 = command(cmd)"));
        assert!(block.contains("f10 = command(cmd)"));
    }

    #[test]
    fn the_block_goes_after_the_users_config_not_inside_it() {
        // Splicing layer headers into the middle of the file would capture
        // every following line into our layer.
        let original = "[ids]\n*\n\n[main]\nf20 = f18\n";
        let out = splice(original, &managed_block(&[accel("<Super><Shift>c")], "cmd"));

        assert!(out.contains("f20 = f18"));
        assert!(out.find("f20 = f18").unwrap() < out.find(BEGIN).unwrap());
        assert!(out.trim_end().ends_with(END));
    }

    #[test]
    fn creates_structure_when_the_file_is_empty() {
        let out = splice("", &managed_block(&[accel("<Super>F9")], "cmd"));
        // Without an [ids] section keyd matches no devices at all.
        assert!(out.contains("[ids]"));
        assert!(out.contains("[meta]"));
        assert!(out.contains("f9 = command(cmd)"));
    }

    #[test]
    fn reinstalling_replaces_rather_than_duplicates() {
        let original = "[ids]\n*\n\n[main]\nf20 = f18\n";
        let first = splice(original, &managed_block(&[accel("<Super>F9")], "cmd"));
        let second = splice(&first, &managed_block(&[accel("<Super>F10")], "cmd"));

        assert_eq!(second.matches(BEGIN).count(), 1);
        assert!(!second.contains("f9 = command"));
        assert!(second.contains("f10 = command"));
        assert!(second.contains("f20 = f18"));
    }

    #[test]
    fn uninstall_restores_the_original_bytes() {
        let original = "[ids]\n*\n\n[main]\nf20 = f18\n";
        let spliced = splice(original, &managed_block(&[accel("<Super>F9")], "cmd"));
        assert_eq!(strip_managed(&spliced), original);
    }

    #[test]
    fn a_binding_in_a_modifier_layer_is_read_back_with_its_modifiers() {
        let text = "[ids]\n*\n\n[main]\nf20 = f18\n\n[meta+shift]\nc = noop\n";
        let found = parse_config(text, Path::new("/x.conf"));

        assert!(found.iter().any(|b| b.accel == accel("F20")));
        assert!(
            found.iter().any(|b| b.accel == accel("<Super><Shift>c")),
            "layer modifiers were not applied: {found:?}"
        );
    }

    #[test]
    fn round_trips_through_generate_and_parse() {
        let want = accel("<Control><Alt>F9");
        let text = splice("[ids]\n*\n", &managed_block(std::slice::from_ref(&want), "cmd"));
        let found = parse_config(&text, Path::new("/x.conf"));

        let ours: Vec<&Binding> = found.iter().filter(|b| b.ours).collect();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].accel, want);
    }

    #[test]
    fn scan_flags_our_own_bindings() {
        let text = format!(
            "[ids]\n*\n\n[main]\nf20 = f18\n{}",
            managed_block(&[accel("<Super>F9")], "cmd")
        );
        let found = parse_config(&text, Path::new("/etc/keyd/default.conf"));

        let theirs = found.iter().find(|b| b.accel == accel("F20")).unwrap();
        assert!(!theirs.ours);
        assert_eq!(theirs.action, "f18");

        let ours = found
            .iter()
            .find(|b| b.accel == accel("<Super>F9"))
            .unwrap();
        assert!(ours.ours);
    }

    #[test]
    fn ids_section_is_not_parsed_as_bindings() {
        let text = "[ids]\n0001:0001:abc\n*\n\n[main]\nf20 = f18\n";
        let found = parse_config(text, Path::new("/x.conf"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].accel, accel("F20"));
    }

    #[test]
    fn user_defined_layers_are_not_treated_as_global_shortcuts() {
        // `[nav]` is only active while some other key is held, so `h` there
        // cannot collide with a global accelerator.
        let text = "[ids]\n*\n\n[main]\na = b\n\n[nav:C]\nh = left\n";
        let found = parse_config(text, Path::new("/x.conf"));

        assert!(found.iter().any(|b| b.accel == accel("a")));
        assert!(!found.iter().any(|b| b.accel.key == "h"));
    }
}
