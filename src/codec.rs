//! Reading image files, whatever their format.
//!
//! The `image` crate covers PNG, JPEG and WebP. JPEG XL comes from
//! `jxl-oxide`, which plugs into the same decoder interface. Everything that
//! opens a shot goes through here, so the library, the viewer, the editor and
//! copying all understand the same formats.

use std::path::Path;

use image::DynamicImage;

/// The bare JPEG XL codestream starts with these two bytes.
const JXL_CODESTREAM: [u8; 2] = [0xff, 0x0a];
/// A JPEG XL file in the ISO container starts with this box.
const JXL_CONTAINER: [u8; 12] = [
    0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a,
];

/// Whether these bytes start a JPEG XL image. Decided by content rather than
/// by name, so a renamed file still opens.
pub fn is_jxl(bytes: &[u8]) -> bool {
    bytes.starts_with(&JXL_CODESTREAM) || bytes.starts_with(&JXL_CONTAINER)
}

/// Decode an image held in memory.
pub fn decode(bytes: &[u8]) -> Result<DynamicImage, String> {
    if is_jxl(bytes) {
        let decoder = jxl_oxide::integration::JxlDecoder::new(std::io::Cursor::new(bytes))
            .map_err(|e| format!("JPEG XL: {e}"))?;
        return DynamicImage::from_decoder(decoder).map_err(|e| format!("JPEG XL: {e}"));
    }
    image::load_from_memory(bytes).map_err(|e| e.to_string())
}

/// Read and decode an image file.
pub fn open(path: &Path) -> Result<DynamicImage, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("{} could not be read: {e}", path.display()))?;
    decode(&bytes).map_err(|e| format!("{} could not be read: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webp_files_in_the_library_can_be_read() {
        let pixels: Vec<u8> = (0..6 * 4).flat_map(|i| [i as u8, 50, 200, 255]).collect();
        let mut bytes = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
            .encode(&pixels, 6, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        let back = decode(&bytes).unwrap().to_rgba8();
        assert_eq!(back.as_raw(), &pixels);
    }

    #[test]
    fn jpeg_xl_is_recognised_by_content_not_name() {
        assert!(is_jxl(&[0xff, 0x0a, 0x00]));
        assert!(is_jxl(&JXL_CONTAINER));
        assert!(!is_jxl(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_jxl(&[]));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode(&[0xff, 0x0a, 1, 2, 3]).is_err());
        assert!(decode(b"not an image").is_err());
    }
}
