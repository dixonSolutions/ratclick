//! `ratclickd` — the RatClick background service.
//!
//! Owns the virtual mouse and the click loop, and exposes both on the session
//! bus so the CLI, the GUI, the GNOME Shell extension and the global shortcut
//! all drive the same state.

use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use clap::Parser;
use ratclick_core::config::Config;
use ratclick_core::ipc;
use tokio::sync::Notify;
use zbus::connection::Builder;

use ratclick_daemon::engine::Engine;
use ratclick_daemon::service::{self, Service};

#[derive(Parser, Debug)]
#[command(
    name = "ratclickd",
    version,
    about = "RatClick background click service"
)]
struct Args {
    /// Log more. Also settable with RUST_LOG.
    #[arg(short, long)]
    verbose: bool,

    /// Exit immediately after checking that the virtual mouse can be created.
    ///
    /// Used by the installer and by `ratclick doctor` to give a clear answer
    /// about /dev/uinput permissions without leaving a daemon behind.
    #[arg(long)]
    check: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_logging(args.verbose);

    if args.check {
        return match Engine::start() {
            Ok(_) => {
                println!("ok: virtual mouse can be created");
                Ok(())
            }
            Err(e) => Err(e),
        };
    }

    let mut cfg = Config::load().context("loading configuration")?;
    for note in cfg.normalise() {
        tracing::warn!("{note}");
    }
    let start_on_launch = cfg.start_clicking_on_launch;
    let click_cfg = cfg.click.clone();
    let config: service::SharedConfig = Arc::new(RwLock::new(cfg));

    let engine = Arc::new(Engine::start()?);
    let quit = Arc::new(Notify::new());

    let mut states = engine.subscribe();

    let conn = Builder::session()
        .context("connecting to the session bus")?
        .name(ipc::BUS_NAME)
        .context("claiming the bus name")?
        .serve_at(
            ipc::OBJECT_PATH,
            Service::new(Arc::clone(&engine), Arc::clone(&config), Arc::clone(&quit)),
        )
        .context("exporting the service object")?
        .build()
        .await
        .with_context(|| {
            format!(
                "could not take {} on the session bus — is another ratclickd already running?",
                ipc::BUS_NAME
            )
        })?;

    tracing::info!(
        version = ratclick_core::VERSION,
        "ratclickd ready on {}",
        ipc::BUS_NAME
    );

    // Republish engine state onto the bus.
    let emitter_conn = conn.clone();
    let broadcaster = tokio::spawn(async move {
        let iface_ref = match emitter_conn
            .object_server()
            .interface::<_, Service>(ipc::OBJECT_PATH)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("cannot get interface reference: {e}");
                return;
            }
        };
        let mut last_running = None;
        while states.changed().await.is_ok() {
            let st = states.borrow_and_update().clone();
            let emitter = iface_ref.signal_emitter();

            if let Err(e) =
                Service::state_changed(emitter, st.running, st.remaining_seconds()).await
            {
                tracing::warn!("emitting StateChanged failed: {e}");
            }

            // Property notifications are only worth sending when the value
            // really moved; the countdown fires once a second on its own.
            let iface = iface_ref.get().await;
            if last_running != Some(st.running) {
                last_running = Some(st.running);
                if let Err(e) = iface.running_changed(emitter).await {
                    tracing::warn!("notifying Running failed: {e}");
                }
            }
            if let Err(e) = iface.remaining_seconds_changed(emitter).await {
                tracing::warn!("notifying RemainingSeconds failed: {e}");
            }
        }
    });

    if start_on_launch {
        tracing::info!("start_clicking_on_launch is set; starting immediately");
        engine.start_clicking(&click_cfg)?;
    }

    wait_for_shutdown(&quit).await;

    tracing::info!("shutting down");
    broadcaster.abort();
    let _ = engine.stop_clicking();
    drop(conn);
    Ok(())
}

async fn wait_for_shutdown(quit: &Notify) {
    use tokio::signal::unix::{signal, SignalKind};

    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("cannot listen for SIGTERM: {e}");
            quit.notified().await;
            return;
        }
    };

    tokio::select! {
        _ = quit.notified() => tracing::info!("Quit() called"),
        _ = term.recv() => tracing::info!("SIGTERM"),
        r = tokio::signal::ctrl_c() => {
            if r.is_ok() {
                tracing::info!("SIGINT");
            }
        }
    }
}

fn init_logging(verbose: bool) {
    use tracing_subscriber::{fmt, EnvFilter};

    let default = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("ratclickd={default},ratclick_core={default}")));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        // systemd adds its own timestamps to the journal.
        .without_time()
        .init();
}
