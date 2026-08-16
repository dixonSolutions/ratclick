//! Session-bus client, plus the logic for getting a daemon to talk to.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ratclick_core::ipc;

#[zbus::proxy(
    interface = "io.github.dixonsolutions.RatClick1",
    default_service = "io.github.dixonsolutions.RatClick.Daemon",
    default_path = "/io/github/dixonsolutions/RatClick/Daemon"
)]
pub trait RatClick {
    fn start(&self) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;
    fn toggle(&self) -> zbus::Result<bool>;
    fn status(&self) -> zbus::Result<(bool, u32, String, String, u32, u64)>;
    fn reload_config(&self) -> zbus::Result<()>;
    fn quit(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn running(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;
}

pub async fn connect() -> Result<RatClickProxy<'static>> {
    let conn = zbus::Connection::session()
        .await
        .context("connecting to the session bus")?;
    RatClickProxy::new(&conn)
        .await
        .context("creating the RatClick proxy")
}

/// Is a daemon currently on the bus?
pub async fn is_running() -> bool {
    let Ok(conn) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else {
        return false;
    };
    dbus.name_has_owner(
        ipc::BUS_NAME
            .try_into()
            .expect("BUS_NAME is a valid bus name"),
    )
    .await
    .unwrap_or(false)
}

/// Get a proxy, starting the daemon first if it is not already up.
///
/// D-Bus activation normally handles this, but a developer build or a partly
/// installed package has no `.service` file, so fall back to launching the
/// binary ourselves.
pub async fn connect_or_start() -> Result<RatClickProxy<'static>> {
    let proxy = connect().await?;

    // A cheap call that activates the service if it is activatable.
    if proxy.running().await.is_ok() {
        return Ok(proxy);
    }

    start_daemon().await?;

    let proxy = connect().await?;
    proxy
        .running()
        .await
        .context("daemon started but is not answering on the bus")?;
    Ok(proxy)
}

pub use ratclick_core::daemon::systemd_unit_exists;

/// Start the daemon and wait for it to reach *this* session bus.
///
/// systemd may have started it against a different bus (its user manager keeps
/// its own `DBUS_SESSION_BUS_ADDRESS`), so if the name does not appear here we
/// launch the binary ourselves, which inherits our environment.
pub async fn start_daemon() -> Result<()> {
    use ratclick_core::daemon::{self, Launched};

    let how = daemon::spawn()?;
    if wait_until_up(Duration::from_secs(3)).await.is_ok() {
        return Ok(());
    }
    if how == Launched::Systemd {
        daemon::spawn_direct()?;
        return wait_until_up(Duration::from_secs(5)).await;
    }
    anyhow::bail!("ratclickd did not appear on the session bus")
}

/// Poll the bus until the daemon appears, or give up.
pub async fn wait_until_up(timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if is_running().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("timed out waiting for ratclickd to appear on the session bus")
}

/// Poll the bus until the daemon is gone, or give up.
pub async fn wait_until_down(timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !is_running().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("timed out waiting for ratclickd to exit")
}

/// Ask the daemon to re-read its config, if one is running.
///
/// Not an error when the daemon is down: it will read the new config when it
/// next starts.
pub async fn nudge_reload() {
    if !is_running().await {
        return;
    }
    if let Ok(proxy) = connect().await {
        let _ = proxy.reload_config().await;
    }
}
