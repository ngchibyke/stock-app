//! Checking and re-encoding an uploaded photo.
//!
//! Nothing a phone sends is trusted. The bytes must really be a JPEG, PNG or
//! WebP image of the type that was declared; its canvas must be small enough to
//! decode safely (checked from the header, before any pixel is allocated); and it
//! comes out as a fresh JPEG, which carries none of the original's metadata (no
//! EXIF, no GPS position, no embedded thumbnails). The picture is turned upright
//! first, because the orientation tag is exactly what is thrown away, and a
//! transparent PNG is laid on white, because JPEG has no transparency.

use std::io::Cursor;

use image::{
    DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits, RgbImage,
    codecs::jpeg::JpegEncoder, imageops::FilterType,
};

use crate::limits::{MAX_UPLOAD_EDGE, OUTPUT_JPEG_QUALITY, OUTPUT_LONGEST_EDGE};

/// Why a photo was refused. The messages are fixed copy: a decoder's own text
/// is never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    NotAnImage,
    TooLarge,
    TooManyPixels,
    Corrupt,
    SizeMismatch,
}

impl Rejected {
    /// What is stored in `uploads.failure`.
    pub fn code(self) -> &'static str {
        match self {
            Rejected::NotAnImage => "not_an_image",
            Rejected::TooLarge => "too_large",
            Rejected::TooManyPixels => "too_many_pixels",
            Rejected::Corrupt => "corrupt",
            Rejected::SizeMismatch => "size_mismatch",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        [
            Rejected::NotAnImage,
            Rejected::TooLarge,
            Rejected::TooManyPixels,
            Rejected::Corrupt,
            Rejected::SizeMismatch,
        ]
        .into_iter()
        .find(|r| r.code() == code)
    }

    pub fn message(self) -> &'static str {
        match self {
            Rejected::NotAnImage => "That file is not a JPEG, PNG or WebP image",
            Rejected::TooLarge => "The photo is larger than allowed",
            Rejected::TooManyPixels => "The photo is too big in pixels; shrink it and try again",
            Rejected::Corrupt => "The photo could not be read",
            Rejected::SizeMismatch => "The uploaded file is not the size that was declared",
        }
    }
}

fn declared_format(content_type: &str) -> Option<ImageFormat> {
    match content_type {
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/png" => Some(ImageFormat::Png),
        "image/webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

/// Decodes `raw` (declared as `content_type`) and returns it as an upright,
/// metadata-free JPEG no larger than the output limits.
pub fn reencode(raw: &[u8], content_type: &str) -> Result<Vec<u8>, Rejected> {
    let declared = declared_format(content_type).ok_or(Rejected::NotAnImage)?;
    // What the bytes really are, not what the phone said.
    match image::guess_format(raw) {
        Ok(found) if found == declared => {}
        _ => return Err(Rejected::NotAnImage),
    }

    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_UPLOAD_EDGE);
    limits.max_image_height = Some(MAX_UPLOAD_EDGE);
    limits.max_alloc = Some(256 * 1024 * 1024);
    let mut reader = ImageReader::with_format(Cursor::new(raw), declared);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(classify)?;
    // Read before the decoder is consumed: this is the tag that gets dropped.
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(classify)?;
    img.apply_orientation(orientation);

    let (w, h) = (img.width(), img.height());
    if w.max(h) > OUTPUT_LONGEST_EDGE {
        img = img.resize(
            OUTPUT_LONGEST_EDGE,
            OUTPUT_LONGEST_EDGE,
            FilterType::Triangle,
        );
    }
    let rgb = flatten(&img);

    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, OUTPUT_JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|_| Rejected::Corrupt)?;
    Ok(out)
}

/// RGB with any transparency laid on white.
fn flatten(img: &DynamicImage) -> RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = u32::from(p[3]);
        let blend = |c: u8| ((u32::from(c) * a + 255 * (255 - a)) / 255) as u8;
        out.put_pixel(x, y, image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
    }
    out
}

fn classify(e: ImageError) -> Rejected {
    match e {
        ImageError::Limits(_) => Rejected::TooManyPixels,
        ImageError::Unsupported(_) => Rejected::NotAnImage,
        _ => Rejected::Corrupt,
    }
}
