//! Native-feeling custom title bar support.
//!
//! macOS keeps the system traffic lights over a transparent title bar; Windows
//! and Linux drop system decorations and the frontend draws the controls. The
//! host reports which controls to draw and performs title bar gestures the way
//! the operating system preferences say, instead of always toggling maximize.

use std::sync::Mutex;

use mailx_core::Activity;
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, EventTarget, Manager, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
#[cfg(not(feature = "body-webview-test"))]
use tauri::{Runtime, plugin::TauriPlugin};
#[cfg(not(feature = "body-webview-test"))]
use tauri_plugin_window_state::StateFlags;

pub(crate) const MAIN_WINDOW: &str = "main";
const STATE_EVENT: &str = "mailx://window-state";

/// What the window-state plugin saves and restores. Two flags are left out on
/// purpose. DECORATIONS: this module owns the title bar style per platform.
/// VISIBLE: the plugin would persist "hidden" if the app is quit while hidden
/// (macOS Cmd+H), and the next launch would then never show the window.
#[cfg(not(feature = "body-webview-test"))]
pub(crate) const WINDOW_STATE_FLAGS: StateFlags = StateFlags::SIZE
    .union(StateFlags::POSITION)
    .union(StateFlags::MAXIMIZED)
    .union(StateFlags::FULLSCREEN);

/// Remembers the main window's size, position and maximized / fullscreen state
/// between launches. A saved position on a monitor that is gone is ignored by
/// the plugin, which leaves placement to the operating system.
#[cfg(not(feature = "body-webview-test"))]
pub(crate) fn window_state_plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri_plugin_window_state::Builder::default()
        .with_state_flags(WINDOW_STATE_FLAGS)
        // Composer windows are placed by the host each time (v0.1 does not
        // remember their size or position).
        .with_filter(|label| label == MAIN_WINDOW)
        .build()
}

/// Windows: WebView2 settings that page scripts cannot be relied on for.
/// Keyboard accelerators are handled by the browser layer, and the form-data
/// prompts and touch gestures are not events at all. The plugin owns the
/// `unsafe` COM calls this workspace forbids; its own keyboard and pointer
/// script is not used (the frontend has one, with an exception for text fields),
/// and default context menus stay on so text fields keep their edit menu.
#[cfg(all(windows, not(feature = "body-webview-test")))]
pub(crate) fn browser_defaults_plugin<R: Runtime>() -> TauriPlugin<R> {
    use tauri_plugin_prevent_default::{Builder, Flags, PlatformOptions};
    Builder::new()
        .with_flags(Flags::empty())
        .platform(
            PlatformOptions::new()
                .browser_accelerator_keys(false)
                .swipe_navigation(false)
                .pinch_zoom(false)
                // A mail client must not offer to remember or autofill form data.
                .general_autofill(false)
                .password_autosave(false),
        )
        .build_with_manual_injection()
}

/// Brings the existing window forward when a second launch is attempted. Before
/// the window exists (the first instance is still starting) there is nothing to do.
#[cfg(not(feature = "body-webview-test"))]
pub(crate) fn focus_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

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
// Windows has no title bar double-click preference; it always toggles maximize.
#[cfg_attr(target_os = "windows", allow(dead_code))]
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
    let navigation_app = app.clone();
    // Created hidden: the window-state plugin restores size and position while
    // the window is built, and showing it afterwards avoids a jump from the
    // default geometry.
    let builder = style_builder(
        WebviewWindowBuilder::from_config(app, &config)?
            .visible(false)
            .on_navigation(move |url| crate::body_protocol::navigation(&navigation_app, url)),
    );
    let window = builder.build()?;
    watch_state(&window);
    crate::compose::watch_close(&window);
    window.show()?;
    window.set_focus()?;
    Ok(window)
}

/// The platform title bar treatment shared by every window: macOS keeps the
/// traffic lights over a transparent title bar, other platforms drop system
/// decorations and the frontend draws the controls.
pub(crate) fn style_builder<'a, R: tauri::Runtime, M: Manager<R>>(
    builder: WebviewWindowBuilder<'a, R, M>,
) -> WebviewWindowBuilder<'a, R, M> {
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(TRAFFIC_LIGHT_POSITION)
        // No link preview popup on long press; the builder method is macOS-only.
        .allow_link_preview(false);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.decorations(false);
    builder
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

/// How the sync should treat the app: someone is looking at the window, or not.
/// A locked screen is "not": the window loses focus, and that is exactly when a
/// new-mail notification is wanted, so the sync keeps going on the slow rhythm.
/// Sleep is not reported to windows on desktop and needs no handling: a
/// sleeping computer runs no timers, and the first round after waking is due.
pub(crate) fn activity_of(focused: bool, visible: bool, minimized: bool) -> Activity {
    if focused && visible && !minimized {
        Activity::Active
    } else {
        Activity::Background
    }
}

/// The app is in front when any of its windows is. `focused` is the focus the
/// event just reported for `current`, which its window may not show yet.
fn app_activity(app: &AppHandle, current: &str, focused: Option<bool>) -> Activity {
    any_active(app.webview_windows().iter().map(|(label, window)| {
        read_activity(window, if label == current { focused } else { None })
    }))
}

pub(crate) fn any_active(activities: impl IntoIterator<Item = Activity>) -> Activity {
    if activities
        .into_iter()
        .any(|activity| activity == Activity::Active)
    {
        Activity::Active
    } else {
        Activity::Background
    }
}

fn read_activity(window: &WebviewWindow, focused: Option<bool>) -> Activity {
    activity_of(
        focused.unwrap_or_else(|| window.is_focused().unwrap_or(false)),
        window.is_visible().unwrap_or(false),
        window.is_minimized().unwrap_or(false),
    )
}

/// Pushes maximize, fullscreen and focus changes so controls can switch glyphs
/// and dim like native ones. Only changes are emitted, not every resize step.
/// The same events tell the sync whether the app is in front: any of its
/// windows (the main window or a composer) being in front counts.
pub(crate) fn watch_state(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    let label = window.label().to_owned();
    let last = Mutex::new(None);
    window.on_window_event(move |event| {
        let focused = match event {
            WindowEvent::Focused(focused) => Some(*focused),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => None,
            _ => return,
        };
        let Some(window) = app.get_webview_window(&label) else {
            return;
        };
        // Minimizing and hiding arrive as resizes or focus changes; the core
        // ignores repeats, so reporting on every one of them is cheap.
        if let Some(state) = app.try_state::<crate::state::DesktopState>() {
            state.set_activity(app_activity(&app, &label, focused));
        }
        let state = read_state(&window, focused);
        let Ok(mut last) = last.lock() else {
            return;
        };
        if *last != Some(state) {
            *last = Some(state);
            let _ = app.emit_to(EventTarget::webview_window(&label), STATE_EVENT, state);
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
    #[test]
    fn the_app_is_in_front_when_any_window_is() {
        use mailx_core::Activity::{Active, Background};
        assert_eq!(any_active([]), Background);
        assert_eq!(any_active([Background, Background]), Background);
        // A composer in front while the main window is minimized or behind.
        assert_eq!(any_active([Background, Active]), Active);
        assert_eq!(any_active([Active, Background]), Active);
    }

    #[test]
    fn only_a_focused_visible_unminimized_window_counts_as_in_front() {
        use mailx_core::Activity::{Active, Background};
        for focused in [false, true] {
            for visible in [false, true] {
                for minimized in [false, true] {
                    let expected = if focused && visible && !minimized {
                        Active
                    } else {
                        Background
                    };
                    assert_eq!(
                        activity_of(focused, visible, minimized),
                        expected,
                        "focused={focused} visible={visible} minimized={minimized}"
                    );
                }
            }
        }
    }

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

    #[cfg(not(feature = "body-webview-test"))]
    #[test]
    fn window_state_never_restores_decorations_or_visibility() {
        assert!(!WINDOW_STATE_FLAGS.contains(StateFlags::DECORATIONS));
        assert!(!WINDOW_STATE_FLAGS.contains(StateFlags::VISIBLE));
        for kept in [
            StateFlags::SIZE,
            StateFlags::POSITION,
            StateFlags::MAXIMIZED,
            StateFlags::FULLSCREEN,
        ] {
            assert!(WINDOW_STATE_FLAGS.contains(kept), "{kept:?}");
        }
    }

    #[cfg(not(feature = "body-webview-test"))]
    #[test]
    fn a_second_launch_before_the_window_exists_is_ignored() {
        let app = tauri::test::mock_app();
        focus_main_window(app.handle());
        assert!(app.get_webview_window(MAIN_WINDOW).is_none());
    }

    #[cfg(not(feature = "body-webview-test"))]
    // The mock runtime reports every window as visible, so showing and focusing
    // can only be checked for not failing here; the real effect needs a real window.
    #[test]
    fn a_second_launch_with_a_running_window_leaves_the_windows_in_place() {
        let app = tauri::test::mock_app();
        for label in [MAIN_WINDOW, "other"] {
            WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::default())
                .build()
                .unwrap();
        }
        focus_main_window(app.handle());
        assert!(app.get_webview_window(MAIN_WINDOW).is_some());
        assert!(app.get_webview_window("other").is_some());
    }
}
