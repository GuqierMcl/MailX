//! Native-feeling custom title bar support.
//!
//! macOS keeps the system traffic lights over a transparent title bar; Windows
//! and Linux drop system decorations and the frontend draws the controls. The
//! host reports which controls to draw and performs title bar gestures the way
//! the operating system preferences say, instead of always toggling maximize.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, EventTarget, Manager, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

pub(crate) const MAIN_WINDOW: &str = "main";
const STATE_EVENT: &str = "mailx://window-state";

/// Traffic light inset for the 40px title bar. wry treats `y` as the height of
/// the title bar container minus one button, and the buttons keep their native
/// ~9pt bottom margin inside it, so their top lands at about `y - 9`: 22 centres
/// the 14pt buttons at 20pt.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_POSITION: tauri::LogicalPosition<f64> = tauri::LogicalPosition::new(16.0, 22.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Platform {
    Macos,
    Windows,
    Linux,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(crate) enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WindowState {
    maximized: bool,
    fullscreen: bool,
    focused: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WindowChrome {
    platform: Platform,
    /// Controls drawn by the frontend before the title content.
    leading: Vec<WindowButton>,
    /// Controls drawn by the frontend after the title content.
    trailing: Vec<WindowButton>,
    state: WindowState,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WindowAction {
    Minimize,
    ToggleMaximize,
    Close,
    StartDragging,
    TitleBarDoubleClick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TitleBarGesture {
    ToggleMaximize,
    Minimize,
    Nothing,
}

/// Builds the main window from `tauri.conf.json` with platform title bar settings.
pub(crate) fn create_main_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN_WINDOW)
        .cloned()
        .ok_or_else(|| tauri::Error::WindowNotFound)?;
    let builder = WebviewWindowBuilder::from_config(app, &config)?;
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(TRAFFIC_LIGHT_POSITION);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.decorations(false);
    let window = builder.build()?;
    watch_state(&window);
    Ok(window)
}

pub(crate) fn chrome(window: &WebviewWindow) -> WindowChrome {
    let (leading, trailing) = button_layout();
    WindowChrome {
        platform: current_platform(),
        leading,
        trailing,
        state: read_state(window, None),
    }
}

pub(crate) fn perform(window: &WebviewWindow, action: WindowAction) -> tauri::Result<()> {
    match action {
        WindowAction::Minimize => window.minimize(),
        WindowAction::ToggleMaximize => toggle_maximize(window),
        WindowAction::Close => window.close(),
        WindowAction::StartDragging => window.start_dragging(),
        WindowAction::TitleBarDoubleClick => match title_bar_double_click() {
            TitleBarGesture::ToggleMaximize => toggle_maximize(window),
            TitleBarGesture::Minimize => window.minimize(),
            TitleBarGesture::Nothing => Ok(()),
        },
    }
}

fn toggle_maximize(window: &WebviewWindow) -> tauri::Result<()> {
    if window.is_maximized()? {
        window.unmaximize()
    } else {
        window.maximize()
    }
}

fn read_state(window: &WebviewWindow, focused: Option<bool>) -> WindowState {
    WindowState {
        maximized: window.is_maximized().unwrap_or(false),
        fullscreen: window.is_fullscreen().unwrap_or(false),
        focused: focused.unwrap_or_else(|| window.is_focused().unwrap_or(true)),
    }
}

/// Pushes maximize, fullscreen and focus changes so controls can switch glyphs
/// and dim like native ones. Only changes are emitted, not every resize step.
fn watch_state(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    let last = Mutex::new(None);
    window.on_window_event(move |event| {
        let focused = match event {
            WindowEvent::Focused(focused) => Some(*focused),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => None,
            _ => return,
        };
        let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
            return;
        };
        let state = read_state(&window, focused);
        let Ok(mut last) = last.lock() else {
            return;
        };
        if *last != Some(state) {
            *last = Some(state);
            let _ = app.emit_to(EventTarget::webview_window(MAIN_WINDOW), STATE_EVENT, state);
        }
    });
}

fn current_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        Platform::Linux
    }
}

#[cfg(target_os = "macos")]
fn button_layout() -> (Vec<WindowButton>, Vec<WindowButton>) {
    // The system draws the traffic lights.
    (Vec::new(), Vec::new())
}

#[cfg(target_os = "windows")]
fn button_layout() -> (Vec<WindowButton>, Vec<WindowButton>) {
    use WindowButton::{Close, Maximize, Minimize};
    (Vec::new(), vec![Minimize, Maximize, Close])
}

#[cfg(target_os = "linux")]
fn button_layout() -> (Vec<WindowButton>, Vec<WindowButton>) {
    let layout = linux::gtk_setting("gtk-decoration-layout");
    parse_decoration_layout(layout.as_deref().unwrap_or("menu:minimize,maximize,close"))
}

/// Parses GTK's `gtk-decoration-layout`, e.g. `"icon:minimize,maximize,close"`;
/// buttons before the colon go on the left. Non-button entries are ignored.
#[cfg(any(target_os = "linux", test))]
fn parse_decoration_layout(layout: &str) -> (Vec<WindowButton>, Vec<WindowButton>) {
    let buttons = |side: &str| {
        side.split(',')
            .filter_map(|name| match name.trim() {
                "minimize" => Some(WindowButton::Minimize),
                "maximize" => Some(WindowButton::Maximize),
                "close" => Some(WindowButton::Close),
                _ => None,
            })
            .collect()
    };
    match layout.split_once(':') {
        Some((leading, trailing)) => (buttons(leading), buttons(trailing)),
        None => (buttons(layout), Vec::new()),
    }
}

#[cfg(target_os = "macos")]
fn title_bar_double_click() -> TitleBarGesture {
    use objc2_foundation::{NSString, NSUserDefaults};

    // System Settings > Desktop & Dock > "Double-click a window's title bar to".
    let defaults = NSUserDefaults::standardUserDefaults();
    let action = defaults.stringForKey(&NSString::from_str("AppleActionOnDoubleClick"));
    match action {
        Some(action) => macos_double_click_gesture(&action.to_string()),
        // Older systems only stored a minimize flag.
        None if defaults.boolForKey(&NSString::from_str("AppleMiniaturizeOnDoubleClick")) => {
            TitleBarGesture::Minimize
        }
        None => TitleBarGesture::ToggleMaximize,
    }
}

/// "Fill" has no public window API; zoom is the closest native behaviour.
#[cfg(any(target_os = "macos", test))]
fn macos_double_click_gesture(action: &str) -> TitleBarGesture {
    match action {
        "Minimize" => TitleBarGesture::Minimize,
        "None" => TitleBarGesture::Nothing,
        _ => TitleBarGesture::ToggleMaximize,
    }
}

#[cfg(target_os = "windows")]
fn title_bar_double_click() -> TitleBarGesture {
    TitleBarGesture::ToggleMaximize
}

#[cfg(target_os = "linux")]
fn title_bar_double_click() -> TitleBarGesture {
    linux_double_click_gesture(
        linux::gtk_setting("gtk-titlebar-double-click")
            .as_deref()
            .unwrap_or("toggle-maximize"),
    )
}

/// `lower` and `menu` need window manager calls a webview gesture cannot make.
#[cfg(any(target_os = "linux", test))]
fn linux_double_click_gesture(action: &str) -> TitleBarGesture {
    match action {
        "toggle-maximize" => TitleBarGesture::ToggleMaximize,
        "minimize" => TitleBarGesture::Minimize,
        _ => TitleBarGesture::Nothing,
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use gtk::prelude::ObjectExt;

    /// Reads a string GtkSettings property. Must run on the GTK main thread,
    /// which is where Tauri runs synchronous commands.
    pub(super) fn gtk_setting(name: &str) -> Option<String> {
        let settings = gtk::Settings::default()?;
        settings.find_property(name)?;
        settings.property::<Option<String>>(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WindowButton::{Close, Maximize, Minimize};

    #[test]
    fn decoration_layout_splits_sides_and_skips_non_buttons() {
        assert_eq!(
            parse_decoration_layout("icon:minimize,maximize,close"),
            (vec![], vec![Minimize, Maximize, Close])
        );
        assert_eq!(
            parse_decoration_layout("close,minimize:menu"),
            (vec![Close, Minimize], vec![])
        );
        assert_eq!(
            parse_decoration_layout("appmenu:close"),
            (vec![], vec![Close])
        );
        assert_eq!(parse_decoration_layout(""), (vec![], vec![]));
    }

    #[test]
    fn double_click_preferences_map_to_gestures() {
        assert_eq!(
            macos_double_click_gesture("Maximize"),
            TitleBarGesture::ToggleMaximize
        );
        assert_eq!(
            macos_double_click_gesture("Fill"),
            TitleBarGesture::ToggleMaximize
        );
        assert_eq!(
            macos_double_click_gesture("Minimize"),
            TitleBarGesture::Minimize
        );
        assert_eq!(macos_double_click_gesture("None"), TitleBarGesture::Nothing);
        assert_eq!(
            linux_double_click_gesture("toggle-maximize"),
            TitleBarGesture::ToggleMaximize
        );
        assert_eq!(
            linux_double_click_gesture("minimize"),
            TitleBarGesture::Minimize
        );
        assert_eq!(
            linux_double_click_gesture("lower"),
            TitleBarGesture::Nothing
        );
    }
}
