use std::io::{self, Cursor, Write};

use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits};

use super::{MAX_DATA_IMAGE_URL_BYTES, MAX_INLINE_IMAGE_BYTES, RenderError};

const MAX_IMAGE_DIMENSION: u32 = 8_192;
const MAX_DECODED_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_REMOTE_IMAGE_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_REMOTE_IMAGE_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// PNG/JPEG are fully decoded; GIF/WebP become a static PNG first frame. Other declared
/// image/* types, animated PNG, oversized dimensions and decode bombs fail closed.
/// The decode allocation budget is 64 MiB, independent of the 50 MiB input limit.
#[derive(Clone, PartialEq, Eq)]
pub struct InlineImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

pub fn validate(bytes: &[u8], declared: &str) -> Result<InlineImage, RenderError> {
    if bytes.len() > MAX_INLINE_IMAGE_BYTES {
        return Err(RenderError::InvalidImage);
    }
    let (format, mime) = match image::guess_format(bytes) {
        Ok(ImageFormat::Png) if declared.eq_ignore_ascii_case("image/png") => {
            check_png_container(bytes, false)?;
            (ImageFormat::Png, "image/png")
        }
        Ok(ImageFormat::Jpeg) if declared.eq_ignore_ascii_case("image/jpeg") => {
            if !bytes.ends_with(&[0xff, 0xd9]) {
                return Err(RenderError::InvalidImage);
            }
            (ImageFormat::Jpeg, "image/jpeg")
        }
        Ok(ImageFormat::Gif) if declared.eq_ignore_ascii_case("image/gif") => {
            (ImageFormat::Gif, "image/gif")
        }
        Ok(ImageFormat::WebP) if declared.eq_ignore_ascii_case("image/webp") => {
            (ImageFormat::WebP, "image/webp")
        }
        // Favicons. The decoder picks the largest embedded entry; the result is
        // re-encoded as PNG below, like every other non-static format.
        Ok(ImageFormat::Ico)
            if ["image/x-icon", "image/vnd.microsoft.icon", "image/ico"]
                .iter()
                .any(|mime| declared.eq_ignore_ascii_case(mime)) =>
        {
            (ImageFormat::Ico, "image/png")
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
    let decoded = reader.decode().map_err(|_| RenderError::InvalidImage)?;
    // Render a bounded first frame for animation-capable formats. Serving the
    // original would expose frames that were never decoded under our budget.
    if matches!(
        format,
        ImageFormat::Gif | ImageFormat::WebP | ImageFormat::Ico
    ) {
        let mut png = Cursor::new(Vec::new());
        decoded
            .write_to(&mut png, ImageFormat::Png)
            .map_err(|_| RenderError::InvalidImage)?;
        return Ok(InlineImage {
            bytes: png.into_inner(),
            mime_type: "image/png".into(),
        });
    }
    Ok(InlineImage {
        bytes: bytes.to_vec(),
        mime_type: mime.to_owned(),
    })
}

/// Recognize formats from bytes, never the URL suffix or an upstream MIME label.
/// TGA has no unique header; only version 2's complete footer is recognizable.
pub(crate) fn remote_raster_format(bytes: &[u8]) -> Option<ImageFormat> {
    let format = image::guess_format(bytes).ok().or_else(|| {
        (bytes.len() >= 44 && bytes.ends_with(b"TRUEVISION-XFILE.\0")).then_some(ImageFormat::Tga)
    })?;
    matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Ico
            | ImageFormat::Bmp
            | ImageFormat::Tiff
            | ImageFormat::Pnm
            | ImageFormat::Qoi
            | ImageFormat::Dds
            | ImageFormat::Hdr
            | ImageFormat::Farbfeld
            | ImageFormat::Tga
    )
    .then_some(format)
}

/// Decode a remote raster under explicit input, dimension and allocation limits.
/// JPEG retains its validated compressed representation; all other formats are
/// re-encoded as a bounded static PNG, including the default image of an APNG.
/// CID/data inputs deliberately retain their separate declared-type policy.
pub fn decode_remote_raster(bytes: &[u8]) -> Result<InlineImage, RenderError> {
    if bytes.len() > MAX_REMOTE_IMAGE_INPUT_BYTES {
        return Err(RenderError::TooLarge);
    }
    let format = remote_raster_format(bytes).ok_or(RenderError::InvalidImage)?;
    if format == ImageFormat::Png {
        // A PNG decoder may finish after IDAT, before noticing a missing IEND.
        check_png_container(bytes, true)?;
    }
    if format == ImageFormat::Jpeg && !bytes.ends_with(&[0xff, 0xd9]) {
        return Err(RenderError::InvalidImage);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_IMAGE_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(image_error)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) * 4 > MAX_DECODED_IMAGE_BYTES {
        return Err(RenderError::TooLarge);
    }
    // Reserve the destination pixels before granting the decoder its remaining
    // workspace, matching ImageReader::decode's allocation accounting.
    limits.reserve(decoder.total_bytes()).map_err(image_error)?;
    decoder.set_limits(limits).map_err(image_error)?;
    let decoded = image::DynamicImage::from_decoder(decoder).map_err(image_error)?;
    if format == ImageFormat::Jpeg {
        return Ok(InlineImage {
            bytes: bytes.to_vec(),
            mime_type: "image/jpeg".into(),
        });
    }
    // Consume rather than clone the decoded image. Eight-bit RGBA also makes
    // float HDR and high-bit-depth sources displayable with one output format.
    let rgba = decoded.into_rgba8();
    let mut output = BoundedOutput::new(MAX_REMOTE_IMAGE_OUTPUT_BYTES);
    image::codecs::png::PngEncoder::new(&mut output)
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| {
            if output.exceeded {
                RenderError::TooLarge
            } else {
                image_error(error)
            }
        })?;
    Ok(InlineImage {
        bytes: output.bytes,
        mime_type: "image/png".into(),
    })
}

fn image_error(error: image::ImageError) -> RenderError {
    match error {
        image::ImageError::Limits(_) => RenderError::TooLarge,
        _ => RenderError::InvalidImage,
    }
}

struct BoundedOutput {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl BoundedOutput {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded: false,
        }
    }
}

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("image output exceeds its limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn check_png_container(bytes: &[u8], allow_animation: bool) -> Result<(), RenderError> {
    let mut offset = 8usize;
    while let Some(header) = bytes.get(offset..offset.saturating_add(8)) {
        let size = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let end = offset
            .checked_add(size)
            .and_then(|end| end.checked_add(12))
            .filter(|end| *end <= bytes.len())
            .ok_or(RenderError::InvalidImage)?;
        let kind = &header[4..];
        if !allow_animation && matches!(kind, b"acTL" | b"fcTL" | b"fdAT") {
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

#[cfg(test)]
mod remote_tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    fn encoded(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb([255, 0, 0])));
        let image = match format {
            ImageFormat::Ico => DynamicImage::ImageRgba8(image.into_rgba8()),
            ImageFormat::Hdr => DynamicImage::ImageRgb32F(image.into_rgb32f()),
            ImageFormat::Farbfeld => DynamicImage::ImageRgba16(image.into_rgba16()),
            _ => image,
        };
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        let mut bytes = bytes.into_inner();
        if format == ImageFormat::Tga {
            // image-rs emits the v1 body. Add the standard v2 footer so that
            // detection does not mistake arbitrary short binary input for TGA.
            bytes.extend_from_slice(&[0; 8]);
            bytes.extend_from_slice(b"TRUEVISION-XFILE.\0");
        }
        bytes
    }

    #[test]
    fn remote_rasters_are_detected_and_normalized_without_mime_labels() {
        for format in [
            ImageFormat::Png,
            ImageFormat::Jpeg,
            ImageFormat::Gif,
            ImageFormat::WebP,
            ImageFormat::Ico,
            ImageFormat::Bmp,
            ImageFormat::Tiff,
            ImageFormat::Pnm,
            ImageFormat::Qoi,
            ImageFormat::Hdr,
            ImageFormat::Farbfeld,
            ImageFormat::Tga,
        ] {
            let bytes = encoded(format, 3, 2);
            assert_eq!(remote_raster_format(&bytes), Some(format));
            let image =
                decode_remote_raster(&bytes).unwrap_or_else(|error| panic!("{format:?}: {error}"));
            let displayed = image::load_from_memory(&image.bytes).unwrap().to_rgb8();
            assert_eq!(displayed.dimensions(), (3, 2), "{format:?}");
            assert!(
                displayed
                    .pixels()
                    .all(|pixel| pixel[0] >= 250 && pixel[1] <= 5 && pixel[2] <= 5),
                "{format:?}"
            );
            assert_eq!(
                image.mime_type,
                if format == ImageFormat::Jpeg {
                    "image/jpeg"
                } else {
                    "image/png"
                }
            );
            if format == ImageFormat::Jpeg {
                assert_eq!(image.bytes, bytes);
            }
            // A format signature cannot rescue a truncated image.
            assert!(
                decode_remote_raster(&bytes[..bytes.len() / 2]).is_err(),
                "{format:?}"
            );
        }
    }

    #[test]
    fn dds_dxt1_is_decoded_and_unsupported_compression_is_rejected() {
        let mut bytes = vec![0; 136];
        bytes[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [
            (4, 124u32),
            (8, 0x1007),
            (12, 4),
            (16, 4),
            (76, 32),
            (80, 4),
            (108, 0x1000),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[84..88].copy_from_slice(b"DXT1");
        let image = decode_remote_raster(&bytes).unwrap();
        let displayed = image::load_from_memory(&image.bytes).unwrap().to_rgb8();
        assert_eq!(displayed.dimensions(), (4, 4));
        assert!(displayed.pixels().all(|pixel| pixel.0 == [0, 0, 0]));
        assert!(decode_remote_raster(&bytes[..135]).is_err());
        bytes[84..88].copy_from_slice(b"DXT9");
        assert!(decode_remote_raster(&bytes).is_err());
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = (data.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(data);
        let mut crc = u32::MAX;
        for byte in &bytes[4..] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320u32.wrapping_mul(crc & 1));
            }
        }
        bytes.extend_from_slice(&(!crc).to_be_bytes());
        bytes
    }

    #[test]
    fn apng_default_image_becomes_a_static_png_and_cid_rules_stay_separate() {
        let png = encoded(ImageFormat::Png, 3, 2);
        let mut apng = png[..33].to_vec();
        apng.extend(png_chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
        let mut control = vec![0; 26];
        control[4..8].copy_from_slice(&3u32.to_be_bytes());
        control[8..12].copy_from_slice(&2u32.to_be_bytes());
        control[20..22].copy_from_slice(&1u16.to_be_bytes());
        control[22..24].copy_from_slice(&10u16.to_be_bytes());
        apng.extend(png_chunk(b"fcTL", &control));
        apng.extend_from_slice(&png[33..]);
        let image = decode_remote_raster(&apng).unwrap();
        assert_eq!(image.mime_type, "image/png");
        assert!(!image.bytes.windows(4).any(|part| part == b"acTL"));
        assert!(validate(&apng, "image/png").is_err());
        assert_eq!(
            image::load_from_memory(&image.bytes).unwrap().to_rgb8(),
            image::load_from_memory(&png).unwrap().to_rgb8()
        );
        assert!(decode_remote_raster(&apng[..apng.len() - 1]).is_err());
    }

    #[test]
    fn malformed_ambiguous_and_unsupported_bytes_do_not_become_images() {
        for bytes in [
            b"<html><img src='private'></html>".as_slice(),
            b"<svg xmlns='http://www.w3.org/2000/svg'/>".as_slice(),
            b"BMnot a bitmap".as_slice(),
            b"qoifnot an image".as_slice(),
            b"\x76\x2f\x31\x01unsupported exr".as_slice(),
            b"\0\0\0\x20ftypavifunsupported avif".as_slice(),
        ] {
            assert!(decode_remote_raster(bytes).is_err());
        }
        let tga = encoded(ImageFormat::Tga, 3, 2);
        assert!(remote_raster_format(&tga[..tga.len() - 26]).is_none());
        assert!(remote_raster_format(b"\x76\x2f\x31\x01unsupported exr").is_none());
    }

    #[test]
    fn dimensions_and_pixel_budget_are_checked_for_new_decoders() {
        for format in [
            ImageFormat::Bmp,
            ImageFormat::Tiff,
            ImageFormat::Pnm,
            ImageFormat::Qoi,
            ImageFormat::Hdr,
            ImageFormat::Farbfeld,
            ImageFormat::Tga,
        ] {
            let bytes = encoded(format, MAX_IMAGE_DIMENSION + 1, 1);
            assert!(
                matches!(decode_remote_raster(&bytes), Err(RenderError::TooLarge)),
                "{format:?}"
            );
        }
        // The dimensions fit individually, but allocating their pixel matrix
        // exceeds the budget. No pixel payload is needed to reject this header.
        assert!(matches!(
            decode_remote_raster(b"P6\n8192 8192\n255\n"),
            Err(RenderError::TooLarge)
        ));
        assert!(matches!(
            decode_remote_raster(&vec![0; MAX_REMOTE_IMAGE_INPUT_BYTES + 1]),
            Err(RenderError::TooLarge)
        ));
    }

    #[test]
    fn normalized_output_has_a_hard_byte_bound() {
        let mut output = BoundedOutput::new(64);
        let pixels = vec![255; 4 * 4 * 4];
        assert!(
            image::codecs::png::PngEncoder::new(&mut output)
                .write_image(&pixels, 4, 4, image::ExtendedColorType::Rgba8)
                .is_err()
        );
        assert!(output.exceeded);
        assert!(output.bytes.len() <= 64);
    }
}
