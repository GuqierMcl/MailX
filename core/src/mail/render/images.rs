use std::io::Cursor;

use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader, Limits};

use super::{MAX_DATA_IMAGE_URL_BYTES, MAX_INLINE_IMAGE_BYTES, RenderError};

const MAX_IMAGE_DIMENSION: u32 = 8_192;
const MAX_DECODED_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

/// Only fully decoded, static PNG and JPEG images are served. Other declared
/// image/* types, animated PNG, oversized dimensions and decode bombs fail closed.
/// The decode allocation budget is 64 MiB, independent of the 50 MiB input limit.
#[derive(Clone, PartialEq, Eq)]
pub struct InlineImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

pub(super) fn validate(bytes: &[u8], declared: &str) -> Result<InlineImage, RenderError> {
    if bytes.len() > MAX_INLINE_IMAGE_BYTES {
        return Err(RenderError::InvalidImage);
    }
    let (format, mime) = match image::guess_format(bytes) {
        Ok(ImageFormat::Png) if declared.eq_ignore_ascii_case("image/png") => {
            check_png_container(bytes)?;
            (ImageFormat::Png, "image/png")
        }
        Ok(ImageFormat::Jpeg) if declared.eq_ignore_ascii_case("image/jpeg") => {
            if !bytes.ends_with(&[0xff, 0xd9]) {
                return Err(RenderError::InvalidImage);
            }
            (ImageFormat::Jpeg, "image/jpeg")
        }
        _ => return Err(RenderError::InvalidImage),
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_IMAGE_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    // A magic-number match alone is insufficient. Decode under explicit limits.
    reader.decode().map_err(|_| RenderError::InvalidImage)?;
    Ok(InlineImage {
        bytes: bytes.to_vec(),
        mime_type: mime.to_owned(),
    })
}

fn check_png_container(bytes: &[u8]) -> Result<(), RenderError> {
    let mut offset = 8usize;
    while let Some(header) = bytes.get(offset..offset.saturating_add(8)) {
        let size = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let end = offset
            .checked_add(size)
            .and_then(|end| end.checked_add(12))
            .filter(|end| *end <= bytes.len())
            .ok_or(RenderError::InvalidImage)?;
        let kind = &header[4..];
        if kind == b"acTL" || kind == b"fcTL" || kind == b"fdAT" {
            return Err(RenderError::InvalidImage);
        }
        if kind == b"IEND" {
            return if size == 0 && end == bytes.len() {
                Ok(())
            } else {
                Err(RenderError::InvalidImage)
            };
        }
        offset = end;
    }
    Err(RenderError::InvalidImage)
}

pub(super) fn data_url(value: &str) -> Option<String> {
    if value.len() > MAX_DATA_IMAGE_URL_BYTES || !value.get(..5)?.eq_ignore_ascii_case("data:") {
        return None;
    }
    let (metadata, data) = value[5..].split_once(',')?;
    let mut fields = metadata.split(';');
    let mime = fields.next()?;
    let encoded = match fields.next() {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("base64") && fields.next().is_none() => true,
        _ => return None,
    };
    let bytes = if encoded {
        STANDARD.decode(data).ok()?
    } else {
        percent_decode(data)?
    };
    let image = validate(&bytes, mime).ok()?;
    let canonical = format!(
        "data:{};base64,{}",
        image.mime_type,
        STANDARD.encode(image.bytes)
    );
    (canonical.len() <= MAX_DATA_IMAGE_URL_BYTES).then_some(canonical)
}

pub(super) fn percent_decode(value: &str) -> Option<Vec<u8>> {
    let mut input = value.as_bytes().iter().copied();
    let mut bytes = Vec::with_capacity(value.len());
    while let Some(byte) = input.next() {
        bytes.push(if byte == b'%' {
            let high = (input.next()? as char).to_digit(16)?;
            let low = (input.next()? as char).to_digit(16)?;
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    Some(bytes)
}
