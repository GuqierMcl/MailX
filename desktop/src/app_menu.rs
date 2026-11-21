use mailx_core::{ApiResult, AppError, ErrorCode};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Builder, Emitter, Manager, WebviewWindow, WindowEvent, Wry};

use crate::commands::{authorize, parse, respond};
use crate::window_access::{WindowKind, url_allowed};

#[cfg(target_os = "macos")]
const ACTION_EVENT: &str = "mailx://app-menu-action";

fn main_action(id: &str) -> bool {
    matches!(
        id,
        "about"
            | "update"
            | "settings"
            | "accounts"
            | "shortcuts"
            | "guide"
            | "themeSystem"
            | "themeLight"
            | "themeDark"
            | "compose"
            | "quit"
    )
}

fn dispatch_main(app: &AppHandle, id: &str) -> Result<(), AppError> {
    if id == "quit" {
        crate::compose::request_quit(app);
        return Ok(());
    }
    let label = crate::window_chrome::MAIN_WINDOW;
    if !crate::lifecycle::desktop_ready(app) {
        return Err(AppError::new(ErrorCode::CoreNotReady));
    }
    focus(app, label).map_err(|_| AppError::new(ErrorCode::Internal))?;
    // This is the listener whose subscription the ready handshake acknowledges.
    app.emit_to(
        tauri::EventTarget::webview_window(label),
        "mailx://desktop-action",
        serde_json::json!({ "menuCommand": id }),
    )
    .map_err(|_| AppError::new(ErrorCode::Internal))
}

#[derive(Deserialize)]
struct ActionArgs {
    id: String,
}

#[tauri::command]
pub(crate) fn app_menu_main_action(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> ApiResult<()> {
    respond(
        authorize(&window, "app_menu_main_action")
            .and_then(|_| parse::<ActionArgs>(request.body()))
            .and_then(|args| {
                if !main_action(&args.id) {
                    return Err(AppError::validation("id"));
                }
                dispatch_main(window.app_handle(), &args.id)
            }),
    )
}

fn trusted(window: &WebviewWindow) -> bool {
    WindowKind::parse(window.label()).is_some() && window.url().is_ok_and(|url| url_allowed(&url))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MenuWindow {
    label: String,
    title: String,
    focused: bool,
}

fn windows(app: &AppHandle) -> Vec<MenuWindow> {
    let mut windows: Vec<_> = app
        .webview_windows()
        .into_values()
        .filter(trusted)
        .map(|window| MenuWindow {
            label: window.label().to_owned(),
            title: window.title().unwrap_or_default(),
            focused: window.is_focused().unwrap_or(false),
        })
        .collect();
    windows.sort_by(|a, b| a.label.cmp(&b.label));
    windows
}

fn focus(app: &AppHandle, label: &str) -> tauri::Result<()> {
    let window = app
        .get_webview_window(label)
        .filter(trusted)
        .ok_or(tauri::Error::WindowNotFound)?;
    #[cfg(target_os = "macos")]
    app.show()?;
    window.unminimize()?;
    window.show()?;
    window.set_focus()
}

#[derive(Deserialize)]
struct FocusArgs {
    label: String,
}

#[tauri::command]
pub(crate) fn app_menu_windows(window: WebviewWindow) -> ApiResult<Vec<MenuWindow>> {
    respond(authorize(&window, "app_menu_windows").map(|_| windows(window.app_handle())))
}

#[tauri::command]
pub(crate) fn app_menu_focus_window(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> ApiResult<()> {
    respond(
        authorize(&window, "app_menu_focus_window")
            .and_then(|_| parse::<FocusArgs>(request.body()))
            .and_then(|args| {
                focus(window.app_handle(), &args.label)
                    .map_err(|_| AppError::new(ErrorCode::Internal))
            }),
    )
}

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct MenuArgs {
    groups: Vec<Group>,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct Group {
    id: String,
    label: String,
    items: Vec<Item>,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct Item {
    id: String,
    label: String,
    enabled: bool,
    accelerator: Option<String>,
    checked: Option<bool>,
    native: Option<bool>,
    #[serde(default)]
    items: Vec<Item>,
    #[serde(default, rename = "separatorBefore")]
    separator_before: bool,
}

#[tauri::command]
pub(crate) fn app_menu_update(
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> ApiResult<()> {
    respond(
        authorize(&window, "app_menu_update")
            .and_then(|_| parse::<MenuArgs>(request.body()))
            .map(|_args| {
                #[cfg(target_os = "macos")]
                macos::update(&window, _args.groups);
            }),
    )
}

pub(crate) fn register(builder: Builder<Wry>) -> Builder<Wry> {
    let builder = builder.on_window_event(|window, event| {
        if WindowKind::parse(window.label()).is_none() {
            return;
        }
        if matches!(event, WindowEvent::Focused(_) | WindowEvent::Destroyed) {
            // Tauri removes the window and its webviews from the manager
            // before invoking these listeners for Destroyed.
            let app = window.app_handle();
            #[cfg(target_os = "macos")]
            macos::window_changed(app, window.label(), event);
            windows_changed(app);
        }
    });
    #[cfg(target_os = "macos")]
    let builder = macos::register(builder);
    builder
}

pub(crate) fn windows_changed(app: &AppHandle) {
    let _ = app.emit_filter("mailx://app-menu-windows-changed", (), |target| {
        matches!(target, tauri::EventTarget::WebviewWindow { label } if WindowKind::parse(label).is_some())
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn main_forwarding_accepts_only_global_menu_actions() {
        for action in [
            "about",
            "update",
            "settings",
            "accounts",
            "shortcuts",
            "guide",
            "themeSystem",
            "themeLight",
            "themeDark",
            "compose",
            "quit",
        ] {
            assert!(super::main_action(action), "{action}");
        }
        for action in [
            "",
            "archive",
            "delete",
            "reply",
            "send",
            "close",
            "window:main",
        ] {
            assert!(!super::main_action(action), "{action}");
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{collections::HashMap, sync::Mutex};

    use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

    use super::*;
    use crate::window_chrome::MAIN_WINDOW;

    const ACTION_PREFIX: &str = "mailx-app-menu:";

    #[derive(Default)]
    struct Menus {
        by_window: HashMap<String, Vec<Group>>,
        displayed: Option<Vec<Group>>,
        built: Option<Built>,
        fallback_generation: u64,
    }

    /// The installed AppKit menu. Replacing `NSApp.mainMenu` redraws the whole
    /// menu bar, so state-only changes are applied to these handles instead.
    struct Built {
        shape: Vec<Group>,
        handles: Vec<Handle>,
    }

    enum Handle {
        Submenu(Submenu<Wry>),
        Item(MenuItem<Wry>),
        Check(CheckMenuItem<Wry>),
    }

    fn predefined(item: &Item) -> bool {
        item.native != Some(false)
            && matches!(
                item.id.as_str(),
                "undo"
                    | "redo"
                    | "cut"
                    | "copy"
                    | "paste"
                    | "selectAll"
                    | "hide"
                    | "hideOthers"
                    | "showAll"
                    | "services"
                    | "bringAllToFront"
            )
    }

    fn has_submenu(item: &Item) -> bool {
        item.id == "openWindows" || !item.items.is_empty()
    }

    /// What needs a rebuild: everything except labels, enabled and checked
    /// state of the items MailX creates itself.
    fn shape(groups: &[Group]) -> Vec<Group> {
        fn strip(items: &mut [Item]) {
            for item in items {
                strip(&mut item.items);
                // Applied in place; AppKit manages predefined items itself.
                item.enabled = false;
                if predefined(item) {
                    continue;
                }
                item.label.clear();
                item.checked = item.checked.map(|_| false);
            }
        }
        let mut groups = groups.to_vec();
        for group in &mut groups {
            group.label.clear();
            strip(&mut group.items);
        }
        groups
    }

    impl Menus {
        fn snapshot(&self, focused: Option<&str>) -> Option<Vec<Group>> {
            if let Some(groups) = focused.and_then(|label| self.by_window.get(label)) {
                return Some(groups.clone());
            }
            let mut groups = self.by_window.get(MAIN_WINDOW)?.clone();
            for group in &mut groups {
                allow_without_focus(&mut group.items);
            }
            Some(groups)
        }
    }

    type State = Mutex<Menus>;

    pub(super) fn register(builder: Builder<Wry>) -> Builder<Wry> {
        // Keep the default AppKit menu until a trusted app page publishes its
        // first snapshot. System quit already goes through ExitRequested.
        builder.manage(State::default()).on_menu_event(on_action)
    }

    pub(super) fn update(window: &WebviewWindow, groups: Vec<Group>) {
        let app = window.app_handle();
        let state = app.state::<State>();
        if let Ok(mut menus) = state.lock() {
            menus.by_window.insert(window.label().to_owned(), groups);
        }
        refresh(app);
    }

    pub(super) fn window_changed(app: &AppHandle, label: &str, event: &WindowEvent) {
        let state = app.state::<State>();
        if matches!(event, WindowEvent::Destroyed)
            && let Ok(mut menus) = state.lock()
        {
            menus.by_window.remove(label);
        }
        refresh(app);
    }

    fn focused_window(app: &AppHandle) -> Option<WebviewWindow> {
        // Cocoa queues the resign and become callbacks separately. Both may
        // run after AppKit has already switched the key window.
        app.webview_windows()
            .into_values()
            .find(|window| window.is_focused().unwrap_or(false) && trusted(window))
    }

    fn allow_without_focus(items: &mut [Item]) {
        for item in items {
            allow_without_focus(&mut item.items);
            item.enabled &= main_action(&item.id)
                || matches!(
                    item.id.as_str(),
                    "hide"
                        | "hideOthers"
                        | "showAll"
                        | "services"
                        | "mainWindow"
                        | "bringAllToFront"
                )
                || item.id.starts_with("window:")
                || item.items.iter().any(|child| child.enabled);
        }
    }

    fn refresh(app: &AppHandle) {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || refresh_on_main_thread(&handle, None));
    }

    fn refresh_on_main_thread(app: &AppHandle, fallback_generation: Option<u64>) {
        let focused = focused_window(app);
        let state = app.state::<State>();
        let Ok(mut menus) = state.lock() else { return };
        let Some(groups) = menus.snapshot(focused.as_ref().map(WebviewWindow::label)) else {
            return;
        };
        if let Some(generation) = fallback_generation {
            if generation != menus.fallback_generation || focused.is_some() {
                return;
            }
        } else {
            menus.fallback_generation = menus.fallback_generation.wrapping_add(1);
        }
        if menus.displayed.as_ref() == Some(&groups) {
            return;
        }
        // AppKit can briefly report no key window between resign and become.
        if focused.is_none() && fallback_generation.is_none() && menus.displayed.is_some() {
            let generation = menus.fallback_generation;
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let handle = app.clone();
                let _ = app
                    .run_on_main_thread(move || refresh_on_main_thread(&handle, Some(generation)));
            });
            return;
        }
        let shape = shape(&groups);
        let updated = match menus.built.as_ref() {
            Some(built) if built.shape == shape => apply(&built.handles, &groups).is_ok(),
            _ => false,
        };
        if updated {
            menus.displayed = Some(groups);
            return;
        }
        if let Ok(handles) = rebuild(app, &groups) {
            menus.built = Some(Built { shape, handles });
            menus.displayed = Some(groups);
        }
    }

    fn rebuild(app: &AppHandle, groups: &[Group]) -> tauri::Result<Vec<Handle>> {
        let menu = Menu::new(app)?;
        let mut handles = Vec::new();
        let mut help = None;
        for group in groups {
            let submenu = Submenu::new(app, &group.label, true)?;
            handles.push(Handle::Submenu(submenu.clone()));
            append_items(app, &submenu, &group.items, &mut handles)?;
            menu.append(&submenu)?;
            if group.id == "help" {
                help = Some(submenu);
            }
        }
        app.set_menu(menu)?;
        if let Some(help) = help {
            help.set_as_help_menu_for_nsapp()?;
        }
        Ok(handles)
    }

    /// Visits items in the order `rebuild` created their handles.
    fn apply(handles: &[Handle], groups: &[Group]) -> tauri::Result<()> {
        fn items(
            handles: &mut std::slice::Iter<'_, Handle>,
            list: &[Item],
        ) -> tauri::Result<()> {
            for item in list {
                if item.id == "separator" || predefined(item) {
                    continue;
                }
                match (handles.next(), item.checked) {
                    (Some(Handle::Submenu(handle)), _) if has_submenu(item) => {
                        set_text(handle.text()?, &item.label, |text| handle.set_text(text))?;
                        if handle.is_enabled()? != item.enabled {
                            handle.set_enabled(item.enabled)?;
                        }
                        items(handles, &item.items)?;
                    }
                    (Some(Handle::Check(handle)), Some(checked)) => {
                        set_text(handle.text()?, &item.label, |text| handle.set_text(text))?;
                        if handle.is_enabled()? != item.enabled {
                            handle.set_enabled(item.enabled)?;
                        }
                        if handle.is_checked()? != checked {
                            handle.set_checked(checked)?;
                        }
                    }
                    (Some(Handle::Item(handle)), None) => {
                        set_text(handle.text()?, &item.label, |text| handle.set_text(text))?;
                        if handle.is_enabled()? != item.enabled {
                            handle.set_enabled(item.enabled)?;
                        }
                    }
                    _ => return Err(tauri::Error::WindowNotFound),
                }
            }
            Ok(())
        }
        fn set_text(
            current: String,
            next: &str,
            set: impl FnOnce(&str) -> tauri::Result<()>,
        ) -> tauri::Result<()> {
            if current == next { Ok(()) } else { set(next) }
        }
        let mut handles = handles.iter();
        for group in groups {
            let Some(Handle::Submenu(handle)) = handles.next() else {
                return Err(tauri::Error::WindowNotFound);
            };
            set_text(handle.text()?, &group.label, |text| handle.set_text(text))?;
            items(&mut handles, &group.items)?;
        }
        Ok(())
    }

    fn append_items(
        app: &AppHandle,
        submenu: &Submenu<Wry>,
        items: &[Item],
        handles: &mut Vec<Handle>,
    ) -> tauri::Result<()> {
        for item in items {
            if item.separator_before || item.id == "separator" {
                submenu.append(&PredefinedMenuItem::separator(app)?)?;
            }
            if item.id == "separator" {
                continue;
            }
            if has_submenu(item) {
                let nested = Submenu::new(app, &item.label, item.enabled)?;
                handles.push(Handle::Submenu(nested.clone()));
                append_items(app, &nested, &item.items, handles)?;
                submenu.append(&nested)?;
                continue;
            }
            let label = Some(item.label.as_str());
            let native = if item.native == Some(false) {
                None
            } else {
                match item.id.as_str() {
                    "undo" => Some(PredefinedMenuItem::undo(app, label)?),
                    "redo" => Some(PredefinedMenuItem::redo(app, label)?),
                    "cut" => Some(PredefinedMenuItem::cut(app, label)?),
                    "copy" => Some(PredefinedMenuItem::copy(app, label)?),
                    "paste" => Some(PredefinedMenuItem::paste(app, label)?),
                    "selectAll" => Some(PredefinedMenuItem::select_all(app, label)?),
                    "hide" => Some(PredefinedMenuItem::hide(app, label)?),
                    "hideOthers" => Some(PredefinedMenuItem::hide_others(app, label)?),
                    "showAll" => Some(PredefinedMenuItem::show_all(app, label)?),
                    "services" => Some(PredefinedMenuItem::services(app, label)?),
                    "bringAllToFront" => Some(PredefinedMenuItem::bring_all_to_front(app, label)?),
                    _ => None,
                }
            };
            let id = format!("{ACTION_PREFIX}{}", item.id);
            if let Some(native) = native {
                submenu.append(&native)?;
            } else if let Some(checked) = item.checked {
                let check = CheckMenuItem::with_id(
                    app,
                    id,
                    &item.label,
                    item.enabled,
                    checked,
                    item.accelerator.as_deref(),
                )?;
                submenu.append(&check)?;
                handles.push(Handle::Check(check));
            } else {
                let plain = MenuItem::with_id(
                    app,
                    id,
                    &item.label,
                    item.enabled,
                    item.accelerator.as_deref(),
                )?;
                submenu.append(&plain)?;
                handles.push(Handle::Item(plain));
            }
        }
        Ok(())
    }

    fn item_enabled(items: &[Item], id: &str) -> bool {
        items
            .iter()
            .any(|item| item.enabled && (item.id == id || item_enabled(&item.items, id)))
    }

    #[cfg(test)]
    #[test]
    fn snapshots_follow_the_current_key_window_and_keep_global_actions_without_one() {
        fn groups(label: &str) -> Vec<Group> {
            serde_json::from_value(serde_json::json!([{
                "id": "test", "label": label, "items": [
                    {"id": "settings", "label": "Settings", "enabled": true},
                    {"id": "close", "label": "Close", "enabled": true}
                ]
            }]))
            .unwrap()
        }
        let mut menus = Menus::default();
        menus
            .by_window
            .insert(MAIN_WINDOW.to_owned(), groups("Main"));
        menus
            .by_window
            .insert("composer".to_owned(), groups("Composer"));

        let main = menus.snapshot(Some(MAIN_WINDOW)).unwrap();
        assert_eq!(main[0].label, "Main");
        let composer = menus.snapshot(Some("composer")).unwrap();
        assert_eq!(composer[0].label, "Composer");
        assert!(item_enabled(&composer[0].items, "close"));
        // Two focus callbacks after the key window switches produce the same menu.
        assert!(menus.snapshot(Some("composer")).as_ref() == Some(&composer));

        menus.by_window.remove("composer");
        for focused in [None, Some("composer")] {
            let fallback = menus.snapshot(focused).unwrap();
            assert_eq!(fallback[0].label, "Main");
            assert!(!item_enabled(&fallback[0].items, "close"));
            assert!(item_enabled(&fallback[0].items, "settings"));
        }
    }

    #[cfg(test)]
    #[test]
    fn only_structural_changes_rebuild_the_menu() {
        fn groups(value: serde_json::Value) -> Vec<Group> {
            serde_json::from_value(value).unwrap()
        }
        let base = groups(serde_json::json!([{
            "id": "message", "label": "Message", "items": [
                {"id": "reply", "label": "Reply", "enabled": false, "accelerator": "CmdOrCtrl+R"},
                {"id": "toggleRead", "label": "Mark as Read", "enabled": true},
                {"id": "toggleReader", "label": "Reader", "enabled": true, "checked": false},
                {"id": "copy", "label": "Copy", "enabled": true}
            ]
        }]));
        let state = groups(serde_json::json!([{
            "id": "message", "label": "Nachricht", "items": [
                {"id": "reply", "label": "Reply", "enabled": true, "accelerator": "CmdOrCtrl+R"},
                {"id": "toggleRead", "label": "Mark as Unread", "enabled": false},
                {"id": "toggleReader", "label": "Reader", "enabled": false, "checked": true},
                {"id": "copy", "label": "Copy", "enabled": false}
            ]
        }]));
        assert!(shape(&base) == shape(&state));

        for changed in [
            // A new accelerator, a plain item becoming a check item, a native
            // edit item replaced by MailX's own, and a renamed native item.
            serde_json::json!({"id": "reply", "label": "Reply", "enabled": false}),
            serde_json::json!({"id": "toggleRead", "label": "Mark as Read", "enabled": true, "checked": false}),
            serde_json::json!({"id": "copy", "label": "Copy", "enabled": true, "native": false}),
            serde_json::json!({"id": "copy", "label": "Kopieren", "enabled": true}),
        ] {
            let mut next = base.clone();
            let id = changed["id"].as_str().unwrap().to_owned();
            let slot = next[0].items.iter_mut().find(|item| item.id == id).unwrap();
            *slot = serde_json::from_value(changed).unwrap();
            assert!(shape(&base) != shape(&next), "{id}");
        }
    }

    fn on_action(app: &AppHandle, event: MenuEvent) {
        let Some(id) = event.id.as_ref().strip_prefix(ACTION_PREFIX) else {
            return;
        };
        let focused = focused_window(app);
        let state = app.state::<State>();
        let Ok(menus) = state.lock() else { return };
        let is_enabled =
            |groups: &[Group]| groups.iter().any(|group| item_enabled(&group.items, id));
        let enabled = menus.displayed.as_deref().is_some_and(is_enabled)
            && (main_action(id)
                || id == "mainWindow"
                || id.starts_with("window:")
                || focused.as_ref().is_some_and(|label| {
                    menus
                        .by_window
                        .get(label.label())
                        .is_some_and(|groups| is_enabled(groups))
                }));
        drop(menus);
        if !enabled {
            return;
        }
        if let Some(label) = id.strip_prefix("window:") {
            let _ = focus(app, label);
        } else if id == "mainWindow" {
            let _ = focus(app, MAIN_WINDOW);
        } else if id == "quit" {
            crate::compose::request_quit(app);
        } else if focused.is_none() && main_action(id) {
            // The no-focus menu comes from main's subscribed webview and only
            // exposes commands that main has already registered.
            if focus(app, MAIN_WINDOW).is_ok() {
                let _ = app.emit_to(
                    tauri::EventTarget::webview_window(MAIN_WINDOW),
                    ACTION_EVENT,
                    id,
                );
            }
        } else if let Some(window) = focused {
            let _ = window.emit_to(
                tauri::EventTarget::webview_window(window.label()),
                ACTION_EVENT,
                id,
            );
        }
    }
}
