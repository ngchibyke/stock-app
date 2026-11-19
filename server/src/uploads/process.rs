//! Checking and cleaning an uploaded image.
//!
//! - The type comes from the bytes' magic numbers, never from a file name or a
//!   header: a renamed `.png` that is not an image is refused.
//! - Only JPEG, PNG and WebP are accepted, at most [`MAX_SIDE`] pixels a side.
//! - The image is decoded, turned upright by its EXIF orientation, and written
//!   out again. Writing a fresh file is what removes EXIF (GPS location, camera,
//!   time) and anything hidden after the picture data.
//! - Decoding is bounded (dimensions and memory), so a small file that claims to be
//!   enormous cannot exhaust the server.

use std::io::Cursor;

use image::{
    DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits,
    codecs::{jpeg::JpegEncoder, png::PngEncoder, webp::WebPEncoder},
    error::LimitErrorKind,
};

use crate::error::AppError;

pub const MAX_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_SIDE: u32 = 4096;
const MAX_ALLOC: u64 = 256 * 1024 * 1024;
const JPEG_QUALITY: u8 = 85;

#[derive(Debug)]
pub struct Processed {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub extension: &'static str,
    pub width: u32,
    pub height: u32,
}

fn unsupported() -> AppError {
    AppError::Unprocessable(
        "unsupported_image",
        "Use a JPEG, PNG or WebP photo.".to_string(),
    )
}

fn unreadable() -> AppError {
    AppError::Unprocessable(
        "invalid_image",
        "That file is not a photo we can read. Try another one.".to_string(),
    )
}

fn too_big() -> AppError {
    AppError::Unprocessable(
        "image_too_large",
        format!("Photos can be at most {MAX_SIDE} pixels wide or tall."),
    )
}

fn decode_error(e: ImageError) -> AppError {
    match e {
        ImageError::Limits(l)
            if matches!(
                l.kind(),
                LimitErrorKind::DimensionError | LimitErrorKind::InsufficientMemory
            ) =>
        {
            too_big()
        }
        _ => unreadable(),
    }
}

/// Validates and re-encodes `bytes`. CPU-heavy: call it from `spawn_blocking`.
pub fn process(bytes: &[u8]) -> Result<Processed, AppError> {
    let format = image::guess_format(bytes).map_err(|_| unsupported())?;
    if !matches!(
        format,
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP
    ) {
        return Err(unsupported());
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let orientation = decoder.orientation().map_err(decode_error)?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    img.apply_orientation(orientation);
    let (width, height) = (img.width(), img.height());

    let mut out = Vec::new();
    let (content_type, extension) = match format {
        ImageFormat::Jpeg => {
            let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
            rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))
                .map_err(|_| unreadable())?;
            ("image/jpeg", "jpg")
        }
        ImageFormat::Png => {
            img.write_with_encoder(PngEncoder::new(&mut out))
                .map_err(|_| unreadable())?;
            ("image/png", "png")
        }
        _ => {
            let rgba = DynamicImage::ImageRgba8(img.to_rgba8());
            rgba.write_with_encoder(WebPEncoder::new_lossless(&mut out))
                .map_err(|_| unreadable())?;
            ("image/webp", "webp")
        }
    };
    Ok(Processed {
        bytes: out,
        content_type,
        extension,
        width,
        height,
    })
}
