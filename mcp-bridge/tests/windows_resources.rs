//! The Windows bridge carries the MCP icon and a readable description (build.rs),
//! so Task Manager and Explorer do not show the default executable icon.
#![cfg(windows)]

use std::path::Path;

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[test]
fn the_bridge_executable_embeds_the_mcp_icon_and_description() {
    let exe = std::fs::read(env!("CARGO_BIN_EXE_mailx-mcp-bridge")).expect("built bridge");

    // Version strings are stored as UTF-16LE in the resource section.
    let description: Vec<u8> = "MailX MCP Bridge"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert!(contains(&exe, &description), "FileDescription is missing");

    // An icon resource keeps each image's bytes unchanged: every image in the
    // generated .ico must appear in the executable.
    let ico = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../desktop/icons/mcp/mailx-mcp.ico"),
    )
    .expect("generated MCP icon");
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    assert!(count > 0);
    for index in 0..count {
        let entry = 6 + index * 16;
        let size = u32::from_le_bytes(ico[entry + 8..entry + 12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(ico[entry + 12..entry + 16].try_into().unwrap()) as usize;
        assert!(
            contains(&exe, &ico[offset..offset + size]),
            "icon image {index} is not embedded"
        );
    }
}
