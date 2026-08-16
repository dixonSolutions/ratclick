//! On-disk configuration: `~/.config/ratclick/config.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::accel::Accel;

/// Bumped only when a migration is needed; `load` refuses to read a newer file.
pub const CONFIG_VERSION: u32 = 1;

pub const MIN_CPM: u32 = 1;
pub const DEFAULT_CPM: u32 = 600;

/// Which mouse button the virtual device presses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

impl Button {
    pub fn as_str(self) -> &'static str {
        match self {
            Button::Left => "left",
            Button::Right => "right",
            Button::Middle => "middle",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Button> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" | "l" | "1" => Some(Button::Left),
            "right" | "r" | "3" => Some(Button::Right),
            "middle" | "m" | "2" => Some(Button::Middle),
            _ => None,
        }
    }
}

/// Whether a clicking run stops on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ClickMode {
    /// Runs until toggled off.
    #[default]
    Endless,
    /// Runs for [`ClickConfig::duration_minutes`] and then stops itself.
    Timed,
}

impl ClickMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ClickMode::Endless => "endless",
            ClickMode::Timed => "timed",
        }
    }
}

/// Which mechanism grabs the global shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ShortcutBackend {
    /// A GNOME custom keybinding managed through gsettings. Session-scoped,
    /// needs no privileges, and is the default on a GNOME desktop.
    #[default]
    Gnome,
    /// The RatClick GNOME Shell extension registers the key with the compositor
    /// directly. Survives gsettings being reset and shows a panel indicator.
    Extension,
    /// A system-wide keyd binding. Works outside GNOME and on the console, but
    /// writes to `/etc/keyd` and therefore needs root to install.
    Keyd,
    /// No global shortcut; toggle from the GUI, the CLI, or the panel only.
    None,
}

impl ShortcutBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            ShortcutBackend::Gnome => "gnome",
            ShortcutBackend::Extension => "extension",
            ShortcutBackend::Keyd => "keyd",
            ShortcutBackend::None => "none",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<ShortcutBackend> {
        match s.trim().to_ascii_lowercase().as_str() {
            "gnome" | "gsettings" => Some(ShortcutBackend::Gnome),
            "extension" | "shell" => Some(ShortcutBackend::Extension),
            "keyd" => Some(ShortcutBackend::Keyd),
            "none" | "off" | "disabled" => Some(ShortcutBackend::None),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClickConfig {
    pub button: Button,
    /// Clicks per minute.
    pub cpm: u32,
    pub mode: ClickMode,
    /// Duration of a timed run. Stored in whole minutes; the GUI presents it as
    /// hours + minutes but never persists that split.
    pub duration_minutes: u32,
}

impl Default for ClickConfig {
    fn default() -> Self {
        ClickConfig {
            button: Button::Left,
            cpm: DEFAULT_CPM,
            mode: ClickMode::Endless,
            duration_minutes: 30,
        }
    }
}

impl ClickConfig {
    /// Interval between clicks. There is no upper bound on `cpm` — only a
    /// floor at [`MIN_CPM`] so the division below can never be by zero.
    pub fn interval(&self) -> std::time::Duration {
        let cpm = self.cpm.max(MIN_CPM) as u64;
        std::time::Duration::from_nanos(60_000_000_000 / cpm)
    }

    /// Total run length for a timed run, or `None` when endless.
    pub fn duration(&self) -> Option<std::time::Duration> {
        match self.mode {
            ClickMode::Endless => None,
            ClickMode::Timed => Some(std::time::Duration::from_secs(
                self.duration_minutes.max(1) as u64 * 60,
            )),
        }
    }

    /// Split `duration_minutes` into the hours + minutes the GUI displays.
    pub fn duration_hm(&self) -> (u32, u32) {
        (self.duration_minutes / 60, self.duration_minutes % 60)
    }

    pub fn set_duration_hm(&mut self, hours: u32, minutes: u32) {
        self.duration_minutes = (hours * 60 + minutes).max(1);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShortcutConfig {
    pub backend: ShortcutBackend,
    /// Every accelerator that toggles clicking. More than one is allowed so a
    /// laptop and an external keyboard can each have something comfortable.
    pub bindings: Vec<Accel>,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        ShortcutConfig {
            backend: ShortcutBackend::Gnome,
            bindings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    /// Set once the first-run wizard has been completed. An unset flag is what
    /// makes `ratclick gui` and `ratclick setup` start the guided flow.
    pub setup_complete: bool,
    /// Begin clicking as soon as the daemon starts. Off by default — a clicker
    /// that starts clicking on login is a hazard.
    pub start_clicking_on_launch: bool,
    pub click: ClickConfig,
    pub shortcut: ShortcutConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CONFIG_VERSION,
            setup_complete: false,
            start_clicking_on_launch: false,
            click: ClickConfig::default(),
            shortcut: ShortcutConfig::default(),
        }
    }
}

impl Config {
    /// `$XDG_CONFIG_HOME/ratclick/config.toml`, honouring `RATCLICK_CONFIG` so
    /// tests and the nested-session harness can point at a scratch file.
    pub fn path() -> Result<PathBuf> {
        if let Some(over) = std::env::var_os("RATCLICK_CONFIG") {
            return Ok(PathBuf::from(over));
        }
        let dirs = directories::ProjectDirs::from("io.github", "dixonSolutions", "ratclick")
            .context("could not determine a config directory")?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Load the config, or return the defaults when the file does not exist.
    ///
    /// A missing file is the "first run" signal, so it is deliberately not an
    /// error: callers check [`Config::setup_complete`] instead.
    pub fn load() -> Result<Config> {
        Self::load_from(&Self::path()?)
    }

    pub fn load_from(path: &Path) -> Result<Config> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        // An empty or whitespace-only file is treated the same as a missing one
        // rather than a parse error; `touch config.toml` is a plausible thing
        // for someone to have done.
        if raw.trim().is_empty() {
            return Ok(Config::default());
        }
        let cfg: Config =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        if cfg.version > CONFIG_VERSION {
            anyhow::bail!(
                "{} was written by a newer RatClick (config version {}, this build understands {})",
                path.display(),
                cfg.version,
                CONFIG_VERSION
            );
        }
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path()?)
    }

    /// Write atomically so a crash mid-write cannot leave a truncated config.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = toml::to_string_pretty(self).context("serialising config")?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// True when there is nothing usable on disk yet and the wizard should run.
    pub fn needs_setup(&self) -> bool {
        !self.setup_complete
    }

    /// Clamp anything out of range rather than refusing to run, and report what
    /// was adjusted so the caller can tell the user.
    pub fn normalise(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.click.cpm < MIN_CPM {
            notes.push(format!("clicks per minute raised to {MIN_CPM}"));
            self.click.cpm = MIN_CPM;
        }
        if self.click.mode == ClickMode::Timed && self.click.duration_minutes == 0 {
            notes.push("timed duration raised to 1 minute".into());
            self.click.duration_minutes = 1;
        }
        let before = self.shortcut.bindings.len();
        self.shortcut.bindings.dedup();
        if self.shortcut.bindings.len() != before {
            notes.push("removed duplicate shortcuts".into());
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults_needing_setup() {
        let dir = std::env::temp_dir().join(format!("ratclick-t{}", std::process::id()));
        let path = dir.join("nope.toml");
        let cfg = Config::load_from(&path).unwrap();
        assert!(cfg.needs_setup());
        assert_eq!(cfg.click.cpm, DEFAULT_CPM);
    }

    #[test]
    fn roundtrips_through_disk() {
        let dir = std::env::temp_dir().join(format!("ratclick-rt{}", std::process::id()));
        let path = dir.join("config.toml");
        let cfg = Config {
            setup_complete: true,
            click: ClickConfig {
                cpm: 1200,
                mode: ClickMode::Timed,
                duration_minutes: 90,
                ..Default::default()
            },
            shortcut: ShortcutConfig {
                bindings: vec![Accel::parse("<Super><Shift>c").unwrap()],
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.save_to(&path).unwrap();

        let back = Config::load_from(&path).unwrap();
        assert_eq!(back, cfg);
        assert_eq!(back.click.duration_hm(), (1, 30));
        let _ = fs::remove_dir_all(&dir);
    }

    fn with_cpm(cpm: u32) -> ClickConfig {
        ClickConfig {
            cpm,
            ..Default::default()
        }
    }

    #[test]
    fn interval_matches_cpm() {
        assert_eq!(
            with_cpm(600).interval(),
            std::time::Duration::from_millis(100)
        );
        assert_eq!(with_cpm(60).interval(), std::time::Duration::from_secs(1));
    }

    #[test]
    fn interval_is_never_zero_even_for_absurd_cpm() {
        assert!(with_cpm(u32::MAX).interval() > std::time::Duration::ZERO);
    }

    #[test]
    fn hours_and_minutes_convert_to_whole_minutes() {
        let mut c = ClickConfig::default();
        c.set_duration_hm(2, 15);
        assert_eq!(c.duration_minutes, 135);
        assert_eq!(c.duration_hm(), (2, 15));
    }

    #[test]
    fn normalise_leaves_high_cpm_uncapped() {
        let mut cfg = Config::default();
        cfg.click.cpm = 999_999;
        let notes = cfg.normalise();
        assert_eq!(cfg.click.cpm, 999_999);
        assert!(notes.is_empty());
    }

    #[test]
    fn normalise_floors_zero_cpm() {
        let mut cfg = Config::default();
        cfg.click.cpm = 0;
        let notes = cfg.normalise();
        assert_eq!(cfg.click.cpm, MIN_CPM);
        assert!(!notes.is_empty());
    }

    #[test]
    fn refuses_config_from_the_future() {
        let dir = std::env::temp_dir().join(format!("ratclick-fut{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, "version = 99\n").unwrap();
        assert!(Config::load_from(&path).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
