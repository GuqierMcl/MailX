//! Actionable Windows verification-code toasts.
//!
//! Protocol activation is used for both the banner and Notification Center.
//! The ordinary plugin's in-process Activated receiver ends at banner timeout,
//! so keeping that receiver alive is not a substitute for this activation path.
//! Windows launches our registered executable with an opaque action URI; Tauri's
//! single-instance callback forwards it to the running process. A cold launch
//! exits quietly because the parent keeps action tokens only for this launch.

use std::{path::Path, time::SystemTime};

use tauri::{AppHandle, Manager};
use uuid::Uuid;
use windows::{
    Data::Xml::Dom::XmlDocument,
    Foundation::{DateTime, IReference, PropertyValue},
    UI::Notifications::{ToastNotification, ToastNotificationManager},
    core::{HSTRING, Interface, h},
};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

use super::{CODE_ACTION_TTL, CodeAction, CodeActionHandler, CodeNotification};

const ACTION_ARGUMENT: &str = "--mailx-notification-action";
const GROUP: &str = "mailx-verification-codes";

struct Native {
    scheme: String,
    app_id: HSTRING,
    handler: CodeActionHandler,
}

/// Install the per-user protocol handler for this exact application identifier.
/// Isolated development builds use their own identifier and cannot replace the
/// installed application's handler. No elevation or external process is used.
pub(super) fn setup(app: &AppHandle, handler: CodeActionHandler) -> Result<(), ()> {
    let identifier = &app.config().identifier;
    let scheme = scheme(identifier).ok_or(())?;
    let executable = std::env::current_exe().map_err(|_| ())?;
    let command = registration_command(&executable).ok_or(())?;
    let classes = RegKey::predef(HKEY_CURRENT_USER);
    let (protocol, _) = classes
        .create_subkey(format!(r"Software\Classes\{scheme}"))
        .map_err(|_| ())?;
    protocol
        .set_value("", &"MailX notification action")
        .map_err(|_| ())?;
    protocol.set_value("URL Protocol", &"").map_err(|_| ())?;
    let (open, _) = protocol
        .create_subkey(r"shell\open\command")
        .map_err(|_| ())?;
    open.set_value("", &command).map_err(|_| ())?;

    // The installed app already has an AUMID shortcut. Register its display
    // name too, so an isolated developer executable has its own attribution.
    let (identity, _) = classes
        .create_subkey(format!(r"Software\Classes\AppUserModelId\{identifier}"))
        .map_err(|_| ())?;
    identity
        .set_value(
            "DisplayName",
            &app.config().product_name.as_deref().unwrap_or("MailX"),
        )
        .map_err(|_| ())?;

    let app_id = HSTRING::from(identifier);
    // Only this feature's group is removed. Ordinary mail notifications keep
    // their existing lifecycle. Old codes cannot regain a token after restart.
    if let Ok(history) = ToastNotificationManager::History() {
        let _ = history.RemoveGroupWithId(&HSTRING::from(GROUP), &app_id);
    }
    app.manage(Native {
        scheme,
        app_id,
        handler,
    });
    Ok(())
}

/// Must run in the single-instance callback before normal window activation.
/// Recognized launch intent is consumed even for malformed/expired requests;
/// such a request must never fall through to a surprise main-window focus.
pub(crate) fn handle_launch(app: &AppHandle, args: &[String]) -> bool {
    if !is_action_launch(args) {
        return false;
    }
    if let Some(native) = app.try_state::<Native>()
        && let Some(action) = parse_launch(&native.scheme, args)
    {
        (native.handler)(action);
    }
    true
}

/// A fresh process with this intent has no live action vault. The host checks
/// this after single-instance setup and exits without creating a main window.
pub(crate) fn is_action_launch(args: &[String]) -> bool {
    args.iter().any(|arg| arg == ACTION_ARGUMENT)
}

/// Remove only code notifications when preview permission is revoked or an
/// account is retired. Action authorization is still checked by the host.
pub(super) fn clear(app: &AppHandle) {
    if let Some(native) = app.try_state::<Native>()
        && let Ok(history) = ToastNotificationManager::History()
    {
        let _ = history.RemoveGroupWithId(&HSTRING::from(GROUP), &native.app_id);
    }
}

pub(super) fn show(app: &AppHandle, notification: CodeNotification) -> Result<(), ()> {
    let native = app.try_state::<Native>().ok_or(())?;
    // windows-core's factory loader handles CO_E_NOTINITIALIZED by calling
    // CoIncrementMTAUsage and retrying. These safe WinRT APIs therefore work
    // on the host's blocking worker without a hand-written unsafe COM init.
    let document = document(&native.scheme, &notification).map_err(|_| ())?;
    let toast = ToastNotification::CreateToastNotification(&document).map_err(|_| ())?;
    toast.SetGroup(&HSTRING::from(GROUP)).map_err(|_| ())?;
    // Windows tags are limited to 16 characters. These are random correlation
    // tags only; action authorization always uses the complete UUID token.
    let tag = Uuid::parse_str(&notification.token)
        .map_err(|_| ())?
        .simple()
        .to_string();
    toast.SetTag(&HSTRING::from(&tag[..16])).map_err(|_| ())?;
    let expires = expiration_time(SystemTime::now()).ok_or(())?;
    let expires: IReference<DateTime> = PropertyValue::CreateDateTime(expires)
        .and_then(|value| value.cast())
        .map_err(|_| ())?;
    toast.SetExpirationTime(&expires).map_err(|_| ())?;
    ToastNotificationManager::CreateToastNotifierWithId(&native.app_id)
        .and_then(|notifier| notifier.Show(&toast))
        .map_err(|_| ())
}

fn document(scheme: &str, notification: &CodeNotification) -> windows::core::Result<XmlDocument> {
    // Tokens and scheme are validated before producing XML; message content is
    // always a text node, never XML or activation arguments.
    let open = action_uri(scheme, "open", &notification.token).ok_or_else(|| {
        windows::core::Error::from_hresult(windows::core::HRESULT(0x80070057_u32 as i32))
    })?;
    let copy = action_uri(scheme, "copy", &notification.token).ok_or_else(|| {
        windows::core::Error::from_hresult(windows::core::HRESULT(0x80070057_u32 as i32))
    })?;
    let document = XmlDocument::new()?;
    let toast = document.CreateElement(h!("toast"))?;
    toast.SetAttribute(h!("activationType"), h!("protocol"))?;
    toast.SetAttribute(h!("launch"), &HSTRING::from(open))?;
    let visual = document.CreateElement(h!("visual"))?;
    let binding = document.CreateElement(h!("binding"))?;
    binding.SetAttribute(h!("template"), h!("ToastGeneric"))?;
    for value in [&notification.title, &notification.body] {
        let text = document.CreateElement(h!("text"))?;
        text.AppendChild(&document.CreateTextNode(&HSTRING::from(value))?)?;
        binding.AppendChild(&text)?;
    }
    visual.AppendChild(&binding)?;
    toast.AppendChild(&visual)?;
    let actions = document.CreateElement(h!("actions"))?;
    let button = document.CreateElement(h!("action"))?;
    button.SetAttribute(h!("content"), &HSTRING::from(&notification.copy_label))?;
    button.SetAttribute(h!("activationType"), h!("protocol"))?;
    button.SetAttribute(h!("arguments"), &HSTRING::from(copy))?;
    actions.AppendChild(&button)?;
    toast.AppendChild(&actions)?;
    let audio = document.CreateElement(h!("audio"))?;
    if notification.sound.is_some() {
        audio.SetAttribute(h!("src"), h!("ms-winsoundevent:Notification.Default"))?;
    } else {
        audio.SetAttribute(h!("silent"), h!("true"))?;
    }
    toast.AppendChild(&audio)?;
    document.AppendChild(&toast)?;
    Ok(document)
}

fn scheme(identifier: &str) -> Option<String> {
    let mut chars = identifier.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        return None;
    }
    Some(format!("{}.notification", identifier.to_ascii_lowercase()))
}

fn registration_command(executable: &Path) -> Option<String> {
    let path = executable.to_str()?;
    if path.is_empty() || path.contains(['"', '\r', '\n']) {
        return None;
    }
    Some(format!("\"{path}\" {ACTION_ARGUMENT} \"%1\""))
}

fn action_uri(scheme: &str, action: &str, token: &str) -> Option<String> {
    if !matches!(action, "copy" | "open")
        || Uuid::parse_str(token).ok()?.hyphenated().to_string() != token
    {
        return None;
    }
    Some(format!("{scheme}://{action}/{token}"))
}

fn parse_launch(scheme: &str, args: &[String]) -> Option<CodeAction> {
    let index = args.iter().position(|arg| arg == ACTION_ARGUMENT)?;
    if args.len() != index + 2 {
        return None;
    }
    let path = args[index + 1].strip_prefix(&format!("{scheme}://"))?;
    let (action, token) = path.split_once('/')?;
    action_uri(scheme, action, token)?;
    match action {
        "copy" => Some(CodeAction::Copy(token.to_owned())),
        "open" => Some(CodeAction::Open(token.to_owned())),
        _ => None,
    }
}

fn expiration_time(now: SystemTime) -> Option<DateTime> {
    // WinRT DateTime counts 100 ns intervals since 1601-01-01 UTC.
    const UNIX_OFFSET_SECONDS: u64 = 11_644_473_600;
    let after_unix = now.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    let ticks = after_unix
        .checked_add(CODE_ACTION_TTL)?
        .as_nanos()
        .checked_div(100)?
        .checked_add(u128::from(UNIX_OFFSET_SECONDS) * 10_000_000)?;
    Some(DateTime {
        UniversalTime: ticks.try_into().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "a6406420-2448-4a63-baa1-074e7c66c04b";

    fn args(uri: &str) -> Vec<String> {
        ["MailX.exe", ACTION_ARGUMENT, uri]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn protocol_actions_round_trip_without_a_code_or_account_in_the_uri() {
        let scheme = scheme("com.mailxapp.desktop").unwrap();
        let copy = action_uri(&scheme, "copy", TOKEN).unwrap();
        assert!(
            matches!(parse_launch(&scheme, &args(&copy)), Some(CodeAction::Copy(token)) if token == TOKEN)
        );
        let open = action_uri(&scheme, "open", TOKEN).unwrap();
        assert!(
            matches!(parse_launch(&scheme, &args(&open)), Some(CodeAction::Open(token)) if token == TOKEN)
        );
        assert_ne!(scheme, super::scheme("com.mailxapp.desktop.dev").unwrap());
    }

    #[test]
    fn malformed_actions_are_consumed_but_never_dispatched() {
        let scheme = "com.mailxapp.desktop.notification";
        for uri in [
            format!("other://copy/{TOKEN}"),
            format!("{scheme}://send/{TOKEN}"),
            format!("{scheme}://copy/{TOKEN}?code=123456"),
            format!("{scheme}://copy/{TOKEN}/extra"),
            format!("{scheme}://copy/123456"),
            format!("{scheme}://copy/{}", TOKEN.replace('-', "")),
        ] {
            assert!(is_action_launch(&args(&uri)));
            assert!(parse_launch(scheme, &args(&uri)).is_none());
        }
        let mut extra = args(&format!("{scheme}://copy/{TOKEN}"));
        extra.push("--extra".into());
        assert!(parse_launch(scheme, &extra).is_none());
        assert!(!is_action_launch(&["MailX.exe".into()]));
    }

    #[test]
    fn executable_and_uri_placeholder_are_independently_quoted() {
        assert_eq!(
            registration_command(Path::new(r"C:\Program Files\MailX\MailX.exe")).unwrap(),
            r#""C:\Program Files\MailX\MailX.exe" --mailx-notification-action "%1""#
        );
        assert!(registration_command(Path::new("bad\"path")).is_none());
        assert!(scheme("bad/identifier").is_none());
    }

    #[test]
    fn notification_xml_keeps_mail_text_out_of_markup_and_actions() {
        let notification = CodeNotification {
            title: "Mail <untrusted> & title".into(),
            body: "Verification code: 123456".into(),
            copy_label: "Copy & continue".into(),
            token: TOKEN.into(),
            sound: None,
        };
        let document = document("com.mailxapp.desktop.notification", &notification).unwrap();
        let xml = document.GetXml().unwrap().to_string();
        assert!(xml.contains("Mail &lt;untrusted&gt; &amp; title"));
        assert!(!xml.contains("<untrusted>"));
        assert_eq!(xml.matches("activationType=\"protocol\"").count(), 2);
        assert!(xml.contains(&format!("copy/{TOKEN}")));
        assert!(!xml.contains("copy/123456"));
        assert!(xml.contains("silent=\"true\""));
    }

    #[test]
    fn expiration_is_bounded_and_uses_winrt_epoch() {
        let at_epoch = expiration_time(SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(
            at_epoch.UniversalTime,
            (11_644_473_600 + CODE_ACTION_TTL.as_secs() as i64) * 10_000_000
        );
    }
}
