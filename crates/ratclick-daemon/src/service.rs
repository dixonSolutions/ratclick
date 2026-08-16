//! The session-bus surface of the daemon.

use std::sync::{Arc, RwLock};

use ratclick_core::config::Config;

use tokio::sync::Notify;
use zbus::object_server::SignalEmitter;

use crate::engine::Engine;

/// Shared, hot-reloadable configuration. Held under a plain `RwLock` because
/// every critical section here is a clone of a handful of integers.
pub type SharedConfig = Arc<RwLock<Config>>;

pub struct Service {
    engine: Arc<Engine>,
    config: SharedConfig,
    quit: Arc<Notify>,
}

impl Service {
    pub fn new(engine: Arc<Engine>, config: SharedConfig, quit: Arc<Notify>) -> Self {
        Service {
            engine,
            config,
            quit,
        }
    }

    fn click_config(&self) -> ratclick_core::config::ClickConfig {
        self.config.read().expect("config poisoned").click.clone()
    }
}

#[zbus::interface(name = "io.github.dixonsolutions.RatClick1")]
impl Service {
    /// Begin clicking with the current configuration. A no-op when already running.
    async fn start(&self) -> zbus::fdo::Result<()> {
        let cfg = self.click_config();
        self.engine
            .start_clicking(&cfg)
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Stop clicking. A no-op when already stopped.
    async fn stop(&self) -> zbus::fdo::Result<()> {
        self.engine
            .stop_clicking()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Flip the current state. Returns the state it flipped to.
    ///
    /// This is what the global shortcut calls, which is why it reads the state
    /// and acts on it in one place rather than making the caller do it — two
    /// rapid presses must not race into an inconsistent state.
    async fn toggle(&self) -> zbus::fdo::Result<bool> {
        if self.engine.is_running() {
            self.engine
                .stop_clicking()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            Ok(false)
        } else {
            let cfg = self.click_config();
            self.engine
                .start_clicking(&cfg)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            Ok(true)
        }
    }

    /// `(running, cpm, button, mode, remaining_seconds, clicks)`
    async fn status(&self) -> (bool, u32, String, String, u32, u64) {
        let st = self.engine.state();
        let cfg = self.click_config();
        (
            st.running,
            cfg.cpm,
            cfg.button.as_str().to_string(),
            cfg.mode.as_str().to_string(),
            st.remaining_seconds(),
            st.clicks,
        )
    }

    /// Re-read `config.toml`. Called by the GUI and CLI after they save.
    ///
    /// A run already in progress is restarted with the new settings so that a
    /// CPM change takes effect immediately instead of at the next toggle.
    async fn reload_config(&self) -> zbus::fdo::Result<()> {
        let mut fresh = Config::load().map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        fresh.normalise();
        *self.config.write().expect("config poisoned") = fresh.clone();

        if self.engine.is_running() {
            self.engine
                .start_clicking(&fresh.click)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }
        tracing::info!("configuration reloaded");
        Ok(())
    }

    /// Stop clicking and exit.
    async fn quit(&self) {
        let _ = self.engine.stop_clicking();
        self.quit.notify_waiters();
    }

    #[zbus(property)]
    async fn running(&self) -> bool {
        self.engine.is_running()
    }

    #[zbus(property)]
    async fn cpm(&self) -> u32 {
        self.click_config().cpm
    }

    #[zbus(property)]
    async fn mode(&self) -> String {
        self.click_config().mode.as_str().to_string()
    }

    #[zbus(property)]
    async fn remaining_seconds(&self) -> u32 {
        self.engine.state().remaining_seconds()
    }

    #[zbus(property)]
    async fn version(&self) -> String {
        ratclick_core::VERSION.to_string()
    }

    #[zbus(signal)]
    pub async fn state_changed(
        emitter: &SignalEmitter<'_>,
        running: bool,
        remaining_seconds: u32,
    ) -> zbus::Result<()>;
}
