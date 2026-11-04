use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use tauri::{AppHandle, Manager};

use super::{ENTRY_ID, MARKER};

#[derive(Clone, Copy)]
enum Format {
    #[cfg(any(target_os = "macos", test))]
    LaunchAgent,
    #[cfg(any(target_os = "linux", test))]
    DesktopEntry,
}

pub(super) struct Entry {
    path: PathBuf,
    executable: PathBuf,
    format: Format,
}

impl Entry {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) fn current(app: &AppHandle) -> io::Result<Self> {
        let home = app.path().home_dir().map_err(io::Error::other)?;
        let executable = std::env::current_exe()?;
        #[cfg(target_os = "macos")]
        {
            // Do not register a copy running from a mounted installer image.
            if executable.starts_with("/Volumes")
                || executable.to_string_lossy().contains("/AppTranslocation/")
                || !executable.to_string_lossy().contains(".app/")
            {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "install the app before enabling login startup",
                ));
            }
            Ok(Self {
                path: home
                    .join("Library/LaunchAgents")
                    .join(format!("{ENTRY_ID}.plist")),
                executable,
                format: Format::LaunchAgent,
            })
        }
        #[cfg(target_os = "linux")]
        {
            let executable = app.env().appimage.map(PathBuf::from).unwrap_or(executable);
            if !executable.is_absolute() || !executable.is_file() {
                return Err(io::Error::other("invalid installed executable"));
            }
            let config = config_home(
                &home,
                std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            );
            Ok(Self {
                path: config.join("autostart").join(format!("{ENTRY_ID}.desktop")),
                executable,
                format: Format::DesktopEntry,
            })
        }
    }

    fn content(&self) -> io::Result<Vec<u8>> {
        let executable = self
            .executable
            .to_str()
            .ok_or_else(|| io::Error::other("non-UTF-8 executable"))?;
        match self.format {
            #[cfg(any(target_os = "macos", test))]
            Format::LaunchAgent => {
                let mut dict = plist::Dictionary::new();
                dict.insert("Label".into(), ENTRY_ID.into());
                dict.insert("RunAtLoad".into(), true.into());
                dict.insert("ProgramArguments".into(), plist::Value::Array(vec![executable.into(), MARKER.into()]));
                let mut bytes = Vec::new();
                plist::Value::Dictionary(dict).to_writer_xml(&mut bytes).map_err(io::Error::other)?;
                Ok(bytes)
            }
            #[cfg(any(target_os = "linux", test))]
            Format::DesktopEntry => Ok(format!(
                "[Desktop Entry]\nType=Application\nName=MailX\nExec={} {MARKER}\nTerminal=false\nStartupNotify=false\n",
                quote_exec(executable)?).into_bytes()),
        }
    }

    pub(super) fn enabled(&self) -> io::Result<bool> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        let registered = match self.format {
            #[cfg(any(target_os = "macos", test))]
            Format::LaunchAgent => {
                let value =
                    plist::Value::from_reader_xml(bytes.as_slice()).map_err(io::Error::other)?;
                let dict = value
                    .as_dictionary()
                    .ok_or_else(|| io::Error::other("invalid login entry"))?;
                dict.get("RunAtLoad").and_then(plist::Value::as_boolean) == Some(true)
                    && dict.get("Disabled").and_then(plist::Value::as_boolean) != Some(true)
                    && dict.get("Label").and_then(plist::Value::as_string) == Some(ENTRY_ID)
                    && dict
                        .get("ProgramArguments")
                        .and_then(plist::Value::as_array)
                        .is_some_and(|args| {
                            args.len() == 2
                                && args[0].as_string() == self.executable.to_str()
                                && args[1].as_string() == Some(MARKER)
                        })
            }
            #[cfg(any(target_os = "linux", test))]
            Format::DesktopEntry => desktop_enabled(&bytes, &self.content()?)?,
        };
        #[cfg(target_os = "macos")]
        if registered && matches!(self.format, Format::LaunchAgent) {
            return Ok(!launch_agent_disabled()?);
        }
        Ok(registered)
    }

    pub(super) fn set(&self, enabled: bool) -> io::Result<()> {
        if enabled {
            atomic_write(&self.path, &self.content()?)?;
            #[cfg(target_os = "macos")]
            if matches!(self.format, Format::LaunchAgent) {
                let output = std::process::Command::new("/bin/launchctl")
                    .args(["enable", &format!("{}/{ENTRY_ID}", gui_domain()?)])
                    .output()?;
                if !output.status.success() {
                    return Err(io::Error::other("cannot enable login agent"));
                }
            }
            Ok(())
        } else {
            match fs::remove_file(&self.path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn gui_domain() -> io::Result<String> {
    let output = std::process::Command::new("/usr/bin/id")
        .arg("-u")
        .output()?;
    let uid = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    let uid = uid.trim();
    if !output.status.success() || uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit()) {
        return Err(io::Error::other("cannot resolve current user"));
    }
    Ok(format!("gui/{uid}"))
}

#[cfg(target_os = "macos")]
fn launch_agent_disabled() -> io::Result<bool> {
    let output = std::process::Command::new("/bin/launchctl")
        .args(["print-disabled", &gui_domain()?])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("cannot read login agent status"));
    }
    let text = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    Ok(text.lines().any(|line| {
        line.trim()
            .strip_prefix(&format!("\"{ENTRY_ID}\" =>"))
            .is_some_and(|value| value.trim() == "true")
    }))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("missing entry directory"))?;
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!(".mailx-autostart-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    let _ = fs::remove_file(temporary);
    result
}

#[cfg(any(target_os = "linux", test))]
fn config_home(home: &Path, configured: Option<PathBuf>) -> PathBuf {
    configured
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
}

/// Desktop Entry string escaping happens before Exec argument unquoting.
#[cfg(any(target_os = "linux", test))]
fn quote_exec(value: &str) -> io::Result<String> {
    if value.chars().any(char::is_control) {
        return Err(io::Error::other("invalid executable path"));
    }
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        match ch {
            '%' => quoted.push_str("%%"),
            '\\' | '"' | '`' | '$' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    Ok(quoted.replace('\\', "\\\\"))
}

#[cfg(any(target_os = "linux", test))]
fn desktop_enabled(bytes: &[u8], expected: &[u8]) -> io::Result<bool> {
    fn fields(bytes: &[u8]) -> io::Result<std::collections::HashMap<&str, &str>> {
        let text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
        let mut entry = false;
        let mut fields = std::collections::HashMap::new();
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                entry = line == "[Desktop Entry]";
            } else if entry
                && !line.starts_with('#')
                && let Some((key, value)) = line.split_once('=')
            {
                fields.insert(key.trim(), value.trim());
            }
        }
        Ok(fields)
    }
    let entry = fields(bytes)?;
    Ok(entry.get("Type") == Some(&"Application")
        && entry.get("Exec") == fields(expected)?.get("Exec")
        && entry.get("Hidden") != Some(&"true")
        && entry.get("X-GNOME-Autostart-enabled") != Some(&"false"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_entries_roundtrip_paths_and_disable_idempotently_without_touching_the_os() {
        for format in [Format::LaunchAgent, Format::DesktopEntry] {
            let temp = tempfile::tempdir().unwrap();
            let entry = Entry {
                path: temp.path().join("nested/startup"),
                executable: PathBuf::from("/应用/Mail X & \"资料\"/mailx"),
                format,
            };
            assert!(!entry.enabled().unwrap());
            // Test file registration directly: never call launchctl or register
            // an actual login item on the developer's machine.
            let bytes = entry.content().unwrap();
            atomic_write(&entry.path, &bytes).unwrap();
            match format {
                Format::LaunchAgent => {
                    let value = plist::Value::from_reader_xml(bytes.as_slice()).unwrap();
                    assert_eq!(
                        value.as_dictionary().unwrap()["ProgramArguments"]
                            .as_array()
                            .unwrap()[0]
                            .as_string(),
                        entry.executable.to_str()
                    );
                }
                Format::DesktopEntry => assert!(entry.enabled().unwrap()),
            }
            entry.set(false).unwrap();
            entry.set(false).unwrap();
            assert!(!entry.enabled().unwrap());
            assert_eq!(
                fs::read_dir(entry.path.parent().unwrap()).unwrap().count(),
                0
            );
        }
    }

    #[test]
    fn xdg_external_disabling_and_configuration_directories_are_respected() {
        let bytes = b"[Desktop Entry]\nType=Application\nExec=\"/opt/Mail X\" --mailx-autostart\n";
        for extra in [
            "Hidden=true",
            "X-GNOME-Autostart-enabled=false",
            "Exec=/tmp/old-mailx",
        ] {
            let modified = format!("{}{extra}\n", std::str::from_utf8(bytes).unwrap());
            assert!(!desktop_enabled(modified.as_bytes(), bytes).unwrap());
        }
        assert!(desktop_enabled(bytes, bytes).unwrap());
        // Use native absolute paths: a Unix-rooted path is not absolute when
        // this portable file-format test is compiled on Windows.
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home");
        let custom = directory.path().join("custom-config");
        assert_eq!(config_home(&home, Some(custom.clone())), custom);
        assert_eq!(
            config_home(&home, Some("relative".into())),
            home.join(".config")
        );
        assert_eq!(
            quote_exec("/应用/Mail X%/mailx").unwrap(),
            "\"/应用/Mail X%%/mailx\""
        );
        assert!(quote_exec("/tmp/injected\nExec=other").is_err());
    }
}
