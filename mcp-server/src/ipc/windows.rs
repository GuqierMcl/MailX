//! Windows named-pipe endpoint.
//!
//! Security model: the pipe is created with `first_pipe_instance(true)` so a
//! squatter that already owns the name makes creation fail, rejects remote
//! clients, and carries a DACL that grants access to the current user's SID
//! only. The pipe name embeds the per-start instance id so it cannot be
//! predicted. This is the only place in the crate that uses `unsafe`.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::io;
use std::path::Path;
use std::ptr;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{Stream, failure_code};
use crate::status::McpFailureCode;

pub(crate) struct Listener {
    name: String,
    descriptor: SecurityDescriptor,
    next: Option<NamedPipeServer>,
}

impl Listener {
    pub(crate) async fn bind(_data_dir: &Path, instance_id: &str) -> Result<Self, McpFailureCode> {
        let name = format!(r"\\.\pipe\mailx-mcp-{instance_id}");
        let descriptor =
            SecurityDescriptor::current_user_only().map_err(|error| failure_code(&error))?;
        let first =
            create_instance(&name, &descriptor, true).map_err(|error| failure_code(&error))?;
        Ok(Self {
            name,
            descriptor,
            next: Some(first),
        })
    }

    pub(crate) fn endpoint(&self) -> String {
        self.name.clone()
    }

    pub(crate) async fn accept(&mut self) -> io::Result<Stream> {
        let server = match self.next.take() {
            Some(server) => server,
            None => create_instance(&self.name, &self.descriptor, false)?,
        };
        server.connect().await?;
        // Create the next instance before serving this one so the name is
        // never briefly unowned.
        self.next = Some(create_instance(&self.name, &self.descriptor, false)?);
        Ok(Box::new(server))
    }
}

fn create_instance(
    name: &str,
    descriptor: &SecurityDescriptor,
    first: bool,
) -> io::Result<NamedPipeServer> {
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    // SAFETY: `attributes` is a valid SECURITY_ATTRIBUTES for the duration of
    // the call, and `lpSecurityDescriptor` points to a descriptor owned by
    // `descriptor`, which outlives the call. Windows copies the descriptor
    // while creating the pipe, so nothing keeps a reference afterwards.
    unsafe {
        options.create_with_security_attributes_raw(
            name,
            ptr::addr_of_mut!(attributes).cast::<c_void>(),
        )
    }
}

/// Owns a security descriptor allocated by Windows (freed with `LocalFree`).
struct SecurityDescriptor(*mut c_void);

// SAFETY: the descriptor is immutable after creation and only read by the OS
// while a pipe instance is created; it carries no thread affinity.
unsafe impl Send for SecurityDescriptor {}
// SAFETY: see above; shared references only read the pointer value.
unsafe impl Sync for SecurityDescriptor {}

impl SecurityDescriptor {
    /// DACL (protected, no inheritance) granting GENERIC_ALL to the current
    /// user's SID and nobody else.
    fn current_user_only() -> io::Result<Self> {
        let sid = current_user_sid_string()?;
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor: *mut c_void = ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated and outlives the call; `descriptor`
        // is a valid out-pointer; the size out-parameter is optional (null).
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 || descriptor.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: the pointer was allocated by
        // ConvertStringSecurityDescriptorToSecurityDescriptorW, which requires
        // LocalFree, and is freed exactly once here.
        unsafe {
            LocalFree(self.0);
        }
    }
}

/// The current process owner's SID in `S-1-5-…` string form.
fn current_user_sid_string() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing;
    // `token` is a valid out-pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = sid_string_from_token(token);
    // SAFETY: `token` was opened above and is closed exactly once.
    unsafe {
        CloseHandle(token);
    }
    result
}

fn sid_string_from_token(token: HANDLE) -> io::Result<String> {
    let mut needed: u32 = 0;
    // SAFETY: a null buffer with length 0 asks for the required size, which is
    // written to `needed`. The call is expected to fail with
    // ERROR_INSUFFICIENT_BUFFER, so its result is not checked.
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    // u64 elements give the 8-byte alignment TOKEN_USER requires.
    let mut buffer = vec![0_u64; (needed as usize).div_ceil(8)];
    // SAFETY: `buffer` is at least `needed` bytes and suitably aligned.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the buffer now holds a TOKEN_USER written by Windows.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut wide: *mut u16 = ptr::null_mut();
    // SAFETY: `sid` points into `buffer`, which is alive; `wide` is a valid
    // out-pointer.
    if unsafe { ConvertSidToStringSidW(sid, &mut wide) } == 0 || wide.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success `wide` is a NUL-terminated UTF-16 string allocated
    // by Windows; we read up to the terminator, then free it once with
    // LocalFree as the API requires.
    let text = unsafe {
        let mut len = 0;
        while *wide.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(wide, len));
        LocalFree(wide.cast());
        text
    };
    Ok(text)
}
