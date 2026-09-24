//! Taking a shot and putting it where it belongs.

use std::path::PathBuf;

use crate::config::{Config, Format};
use crate::platform::{self, Image, Rect, WindowInfo};
use crate::{clipboard, naming};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Open the overlay and let the user choose.
    Region,
    /// Take the window that is in front right now, no overlay.
    ActiveWindow,
    /// Everything, across all monitors.
    Fullscreen,
}

/// What a finished capture produced.
pub struct Outcome {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub notes: Vec<String>,
}

/// The part of a frame that shows the desktop rectangle `region`, where the
/// frame covers `screen`. The two may count in different units: under XWayland
/// with display scaling the frame has physical pixels and the desktop logical
/// ones, so the rectangle is scaled into the frame.
pub fn crop_desktop(image: &Image, screen: Rect, region: Rect) -> Image {
    let sx = image.width as f64 / screen.w.max(1) as f64;
    let sy = image.height as f64 / screen.h.max(1) as f64;
    let x0 = ((region.x - screen.x) as f64 * sx).round() as i32;
    let y0 = ((region.y - screen.y) as f64 * sy).round() as i32;
    let x1 = ((region.right() - screen.x) as f64 * sx).round() as i32;
    let y1 = ((region.bottom() - screen.y) as f64 * sy).round() as i32;
    image.crop(Rect::new(x0, y0, x1 - x0, y1 - y0))
}

/// Grab the whole virtual desktop. This is the frame the overlay freezes.
pub fn grab_screen() -> Result<(Image, Rect), String> {
    platform::capture_screen()
}

/// Write a shot to disk and run the configured follow-up actions.
pub fn store(
    image: &Image,
    config: &Config,
    source: Option<&WindowInfo>,
) -> Result<Outcome, String> {
    let ctx = naming::Context {
        time: platform::local_now(),
        window_title: source.map(|w| w.title.as_str()).unwrap_or(""),
        app: source.map(|w| w.app.as_str()).unwrap_or(""),
        width: image.width,
        height: image.height,
    };

    let path = naming::target_path(
        &config.folder,
        &config.subfolder,
        &config.filename,
        config.format.extension(),
        config.anonymous_names,
        &ctx,
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("{} could not be created: {e}", parent.display()))?;
    }

    let encoded = encode(image, config.format, config.jpeg_quality)?;
    std::fs::write(&path, encoded)
        .map_err(|e| format!("{} could not be written: {e}", path.display()))?;

    // Follow-up actions must never lose the file that is already saved, so a
    // failure here is collected and reported rather than returned.
    let mut notes = Vec::new();
    if config.after.copy_image
        && let Err(e) = clipboard::copy_image(image)
    {
        notes.push(format!("The image did not reach the clipboard: {e}"));
    }
    if config.after.copy_path
        && let Err(e) = clipboard::copy_text(&path.to_string_lossy())
    {
        notes.push(format!("The path did not reach the clipboard: {e}"));
    }
    if config.after.open_folder {
        platform::reveal_in_file_manager(&path);
    }

    Ok(Outcome {
        path,
        width: image.width,
        height: image.height,
        notes,
    })
}

pub fn encode(image: &Image, format: Format, jpeg_quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    match format {
        Format::Png => {
            // Fast compression rather than the default. On a 4K shot this is
            // the difference between a tenth of a second and most of one, and
            // the file grows by a few percent.
            let encoder = image::codecs::png::PngEncoder::new_with_quality(
                &mut cursor,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::Adaptive,
            );
            image::ImageEncoder::write_image(
                encoder,
                &image.rgba,
                image.width,
                image.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| format!("PNG: {e}"))?;
        }
        Format::Jpeg => {
            // JPEG has no alpha channel, so the buffer is narrowed first.
            let mut rgb = Vec::with_capacity((image.width * image.height * 3) as usize);
            for px in image.rgba.chunks_exact(4) {
                rgb.extend_from_slice(&px[..3]);
            }
            let encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, jpeg_quality);
            image::ImageEncoder::write_image(
                encoder,
                &rgb,
                image.width,
                image.height,
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| format!("JPEG: {e}"))?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            rgba: (0..width * height)
                .flat_map(|i| [(i % 255) as u8, 40, 90, 255])
                .collect(),
        }
    }

    #[test]
    fn png_output_is_readable_again() {
        let image = square(8, 5);
        let bytes = encode(&image, Format::Png, 90).unwrap();
        let back = image::load_from_memory(&bytes).unwrap();
        assert_eq!((back.width(), back.height()), (8, 5));
    }

    #[test]
    fn jpeg_output_is_readable_again() {
        let image = square(8, 5);
        let bytes = encode(&image, Format::Jpeg, 80).unwrap();
        let back = image::load_from_memory(&bytes).unwrap();
        assert_eq!((back.width(), back.height()), (8, 5));
    }

    #[test]
    fn lower_jpeg_quality_makes_a_smaller_file() {
        let image = square(64, 64);
        let high = encode(&image, Format::Jpeg, 95).unwrap();
        let low = encode(&image, Format::Jpeg, 30).unwrap();
        assert!(low.len() < high.len());
    }

    /// Anonymous file names would be pointless if the file itself carried a
    /// timestamp or a camera-style comment, so the encoders are checked for
    /// anything beyond the image.
    #[test]
    fn png_output_carries_no_metadata() {
        let bytes = encode(&square(16, 16), Format::Png, 90).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");

        let mut tags = Vec::new();
        let mut at = 8;
        while at + 8 <= bytes.len() {
            let length = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            tags.push(String::from_utf8_lossy(&bytes[at + 4..at + 8]).into_owned());
            at += 12 + length;
        }
        for unwanted in ["tEXt", "iTXt", "zTXt", "tIME", "eXIf"] {
            assert!(
                !tags.iter().any(|tag| tag == unwanted),
                "{unwanted} should not be written, found {tags:?}"
            );
        }
    }

    #[test]
    fn jpeg_output_carries_no_exif() {
        let bytes = encode(&square(16, 16), Format::Jpeg, 80).unwrap();
        // APP1 is where EXIF lives; APP0 is the plain JFIF header.
        assert!(
            !bytes.windows(2).any(|pair| pair == [0xFF, 0xE1]),
            "an APP1 segment would mean EXIF"
        );
        assert!(
            !bytes.windows(4).any(|w| w == b"Exif"),
            "no EXIF marker should be present"
        );
    }

    #[test]
    fn cropping_keeps_the_right_pixels() {
        let image = square(4, 4);
        let cropped = image.crop(Rect::new(1, 1, 2, 2));
        assert_eq!((cropped.width, cropped.height), (2, 2));
        assert_eq!(cropped.pixel(0, 0), image.pixel(1, 1));
        assert_eq!(cropped.pixel(1, 1), image.pixel(2, 2));
    }

    #[test]
    fn cropping_outside_the_image_is_clipped_not_a_panic() {
        let image = square(4, 4);
        let cropped = image.crop(Rect::new(3, 3, 10, 10));
        assert_eq!((cropped.width, cropped.height), (1, 1));
    }
}
