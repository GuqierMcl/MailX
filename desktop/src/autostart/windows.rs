use std::io;

use tauri::AppHandle;
use winreg::{
    RegKey, RegValue,
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_BINARY},
};

use super::{ENTRY_ID, MARKER};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

pub(super) struct Entry {
    command: String,
}

impl Entry {
    pub(super) fn current(_: &AppHandle) -> io::Result<Self> {
        let path = dunce::canonicalize(std::env::current_exe()?)?;
        let path = path
            .to_str()
            .ok_or_else(|| io::Error::other("invalid executable path"))?;
        Ok(Self {
            command: format!("\"{path}\" {MARKER}"),
        })
    }

    pub(super) fn enabled(&self) -> io::Result<bool> {
        let user = RegKey::predef(HKEY_CURRENT_USER);
        let command = match user
            .open_subkey_with_flags(RUN, KEY_READ)
            .and_then(|key| key.get_value::<String, _>(ENTRY_ID))
        {
            Ok(command) => command,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        if command != self.command {
            return Ok(false);
        }
        match user
            .open_subkey_with_flags(APPROVED, KEY_READ)
            .and_then(|key| key.get_raw_value(ENTRY_ID))
        {
            // StartupApproved values start with a DWORD: 2/6 enabled, 3/7
            // disabled. Unknown formats are not claimed to be enabled.
            Ok(value) => Ok(value.vtype == REG_BINARY
                && value.bytes.len() >= 4
                && matches!(
                    u32::from_le_bytes(value.bytes[..4].try_into().unwrap()),
                    2 | 6
                )),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(e) => Err(e),
        }
    }

    pub(super) fn set(&self, enabled: bool) -> io::Result<()> {
        let user = RegKey::predef(HKEY_CURRENT_USER);
        if enabled {
            user.create_subkey(RUN)?
                .0
                .set_value(ENTRY_ID, &self.command)?;
            // Only explicit user enabling clears a Task Manager override.
            let mut bytes = vec![0; 12];
            bytes[0] = 2;
            user.create_subkey(APPROVED)?.0.set_raw_value(
                ENTRY_ID,
                &RegValue {
                    bytes,
                    vtype: REG_BINARY,
                },
            )
        } else {
            match user
                .open_subkey_with_flags(RUN, KEY_WRITE)
                .and_then(|key| key.delete_value(ENTRY_ID))
            {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            }
        }
    }
}
