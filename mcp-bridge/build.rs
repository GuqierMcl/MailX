//! Gives the Windows bridge (`mailx-mcp.exe`) its own file icon and a readable
//! description, so Task Manager and Explorer show "MailX MCP Bridge" with the
//! MCP icon instead of the default executable icon. Other targets have no
//! file icons for plain executables, so nothing is embedded there.

fn main() {
    println!("cargo:rerun-if-changed=../desktop/icons/mcp/mailx-mcp.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resources();
    }
}

/// The resource compiler ships with the Windows SDK, as for the app itself.
#[cfg(windows)]
fn embed_windows_resources() {
    let icon =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../desktop/icons/mcp/mailx-mcp.ico");
    let mut resource = tauri_winres::WindowsResource::new();
    resource
        .set_icon(icon.to_str().expect("UTF-8 icon path"))
        .set("FileDescription", "MailX MCP Bridge")
        .set("ProductName", "MailX")
        .set("InternalName", "mailx-mcp")
        .set("OriginalFilename", "mailx-mcp.exe");
    resource
        .compile()
        .expect("embed the MCP bridge icon and version information");
}

/// Cross builds for Windows from another host skip the icon.
#[cfg(not(windows))]
fn embed_windows_resources() {}
