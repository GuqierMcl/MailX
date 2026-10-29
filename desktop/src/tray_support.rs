//! Linux notification-area capability, independent of whether creating an
//! AppIndicator object succeeded. A registered host can still hide an icon at
//! the user's request; this probe cannot establish pixel-level visibility.

use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_millis(750);
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
// Match the namespace used by libayatana-appindicator / libappindicator.
// A host advertising only another protocol cannot display this backend's icon.
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

#[derive(Clone, Copy, Debug)]
pub struct DesktopStatus {
    pub host_available: bool,
    pub session_locked: bool,
}

impl Default for DesktopStatus {
    fn default() -> Self {
        Self::UNAVAILABLE
    }
}

impl DesktopStatus {
    // A disconnected/unresponsive bus is not evidence that the session is
    // unlocked. Defer restoring a hidden window until a later healthy probe.
    const UNAVAILABLE: Self = Self {
        host_available: false,
        session_locked: true,
    };
}

/// Call from a blocking worker, never the UI thread or an async-runtime task.
/// The timeout covers opening/authenticating the session bus and all requests.
/// Missing, unresponsive, and malformed tray hosts fail closed.
pub fn desktop_status() -> DesktopStatus {
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(PROBE_TIMEOUT, probe_session())
            .await
            .unwrap_or(DesktopStatus::UNAVAILABLE)
    })
}

async fn probe_session() -> DesktopStatus {
    let Ok(connection) = zbus::Connection::session().await else {
        return DesktopStatus::UNAVAILABLE;
    };
    let (host, freedesktop_lock, gnome_lock) = tokio::join!(
        registered_host(&connection),
        screen_saver_active(&connection, "org.freedesktop.ScreenSaver", "/ScreenSaver",),
        screen_saver_active(
            &connection,
            "org.gnome.ScreenSaver",
            "/org/gnome/ScreenSaver",
        ),
    );
    DesktopStatus {
        host_available: host.unwrap_or(false),
        // Unsupported APIs return false. Transport/protocol failures stay
        // conservative; either supported screensaver reporting active wins.
        session_locked: freedesktop_lock.unwrap_or(true) || gnome_lock.unwrap_or(true),
    }
}

async fn registered_host(connection: &zbus::Connection) -> zbus::Result<bool> {
    let proxy: zbus::Proxy<'_> = zbus::proxy::Builder::new(connection)
        .destination(WATCHER)?
        .path(WATCHER_PATH)?
        .interface(WATCHER)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    proxy.get_property("IsStatusNotifierHostRegistered").await
}

async fn screen_saver_active(
    connection: &zbus::Connection,
    service: &str,
    path: &str,
) -> zbus::Result<bool> {
    let proxy = zbus::Proxy::new(connection, service, path, service).await?;
    match proxy.call("GetActive", &()).await {
        Err(error) if unsupported_api(&error) => Ok(false),
        result => result,
    }
}

fn unsupported_api(error: &zbus::Error) -> bool {
    use zbus::DBusError;

    let name = match error {
        zbus::Error::MethodError(name, ..) => name.to_string(),
        zbus::Error::FDO(error) => error.name().to_string(),
        _ => return false,
    };
    matches!(
        name.as_str(),
        "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.DBus.Error.UnknownInterface"
            | "org.freedesktop.DBus.Error.UnknownMethod"
            | "org.freedesktop.DBus.Error.NotSupported"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_lock_api_is_distinct_from_a_temporary_bus_failure() {
        for error in [
            zbus::fdo::Error::ServiceUnknown(String::new()),
            zbus::fdo::Error::UnknownMethod(String::new()),
            zbus::fdo::Error::NotSupported(String::new()),
        ] {
            assert!(unsupported_api(&zbus::Error::FDO(Box::new(error))));
        }
        for error in [
            zbus::fdo::Error::NoReply(String::new()),
            zbus::fdo::Error::Disconnected(String::new()),
            zbus::fdo::Error::AccessDenied(String::new()),
        ] {
            assert!(!unsupported_api(&zbus::Error::FDO(Box::new(error))));
        }
    }
}
