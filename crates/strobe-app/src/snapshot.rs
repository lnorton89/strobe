// ============================================================================
// Module:       strobe_app::snapshot
// Description:  A scene's thumbnail taken from its live frame: shrunk,
//               written as thumbnail.jpg in the scene folder, and named in
//               scene.json's IMAGE_PATH.
//
// Dependencies: image; strobe-scene (json_edit)
// ============================================================================

//! "Use this frame as the thumbnail" (backlog U-06, D222).
//!
//! The browser shows a scene's picture from `IMAGE_PATH` (D212). A scene
//! without one, or with one that no longer looks like it, gets its picture
//! from the operator: the frame on screen, read back once, shrunk to
//! [`SIZE`] and written as [`FILE`] beside the scene's code. `scene.json`
//! then names it, edited in place so the rest of the file is untouched
//! (`strobe_scene::json_edit`). An older picture the scene named is left
//! on disk; only the name changes.
//!
//! Both files are written to a temporary name and renamed into place, so a
//! failure leaves the scene as it was.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The thumbnail's file name in the scene folder.
pub const FILE: &str = "thumbnail.jpg";

/// The largest the thumbnail is, in pixels; the frame's aspect is kept.
pub const SIZE: [u32; 2] = [320, 180];

/// JPEG quality: small files, no visible blocking at this size.
const QUALITY: u8 = 85;

/// Writes `pixels` (RGBA8, `size`, top row first) as the thumbnail of the
/// scene in `dir` and points its `IMAGE_PATH` at it. Returns the picture's
/// path.
///
/// # Errors
///
/// When the pixels do not match the size, the picture cannot be encoded,
/// `scene.json` is not a JSON object, or a file cannot be written.
pub fn save(dir: &Path, size: [u32; 2], pixels: Vec<u8>) -> Result<PathBuf, String> {
    let [width, height] = size;
    let frame = image::RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
        format!(
            "a {width}x{height} frame needs {} bytes",
            width * height * 4
        )
    })?;
    let small = image::DynamicImage::ImageRgba8(frame)
        .thumbnail(SIZE[0], SIZE[1])
        .to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, QUALITY)
        .encode_image(&small)
        .map_err(|e| format!("could not encode the picture: {e}"))?;

    let json_path = dir.join("scene.json");
    let text = match std::fs::read_to_string(&json_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}\n".to_string(),
        Err(error) => return Err(format!("could not read scene.json: {error}")),
    };
    let named = format!("\"{FILE}\"");
    let json = strobe_scene::json_edit::set_member(&text, "IMAGE_PATH", &named)
        .map_err(|e| e.to_string())?;

    let picture = dir.join(FILE);
    write_atomically(&picture, &jpeg)?;
    if json != text {
        write_atomically(&json_path, json.as_bytes())?;
    }
    Ok(picture)
}

/// Writes `bytes` beside `path` and renames them into place.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".strobe-tmp");
    let temporary = PathBuf::from(temporary);
    let written = std::fs::File::create(&temporary)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&temporary, path));
    written.map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("could not write {}: {error}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_frame_becomes_the_thumbnail_and_the_scene_names_it() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("strobe-snapshot-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join("scene.json"),
            "{\n    \"TITLE\": \"Glow\",\n    \"IMAGE_PATH\": \"old.png\"\n}\n",
        )?;
        let (w, h) = (640_u32, 360_u32);
        let pixels = [255_u8, 128, 0, 255].repeat((w * h) as usize);

        let picture = save(&dir, [w, h], pixels).map_err(anyhow::Error::msg)?;
        assert_eq!(picture, dir.join(FILE), "written beside the code");
        let read = image::open(&picture)?;
        assert_eq!(
            (read.width(), read.height()),
            (320, 180),
            "shrunk to fit, aspect kept"
        );
        let json = std::fs::read_to_string(dir.join("scene.json"))?;
        assert_eq!(
            json, "{\n    \"TITLE\": \"Glow\",\n    \"IMAGE_PATH\": \"thumbnail.jpg\"\n}\n",
            "only IMAGE_PATH changed"
        );
        assert!(
            save(&dir, [w, h], vec![0; 8]).is_err(),
            "a short frame is refused"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
