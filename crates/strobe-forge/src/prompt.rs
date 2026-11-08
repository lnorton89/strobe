// ============================================================================
// Module:       strobe_forge::prompt
// Description:  Builds what `make` sends the model: the skill's files as the
//               system prompt, the picture as a resized JPEG, and the text
//               of each turn (first request, repair, critique).
//
// Dependencies: anyhow, image, base64; crate::author (message parts)
// ============================================================================

//! The prompt.
//!
//! The method lives in one place, the scene-from-image skill
//! (`.claude/skills/scene-from-image/`), and `make` reads it at run time
//! rather than carrying a copy: `SKILL.md`, then every file in `reference/`
//! in name order (the Markdown guides and the template's GLSL and JSON),
//! each under a heading naming it. An edit to the skill reaches the next
//! run without a rebuild, and an agent following the skill by hand and the
//! loop read the same words (D562).
//!
//! The model gets no tools (`docs/LLM_SHADER_CREATION.md`): it answers with
//! the two files in tagged blocks ([`crate::reply`]) and this program does
//! the rendering, the checking and the writing.

use crate::author::Part;
use anyhow::{Context as _, Result, anyhow};
use base64::Engine as _;
use std::path::Path;

/// The longest side the picture is sent at: what the model sees anyway.
const LONG_SIDE: u32 = 1568;
/// JPEG quality for pictures sent.
const QUALITY: u8 = 85;

/// The output contract, appended after the skill.
const CONTRACT: &str = "\
# How to answer

You are running inside `strobe-forge make`, not an agent session: you have no
tools and cannot read or write files. Follow the skill above, but where it says
to run a command, the program does it and sends you the results.

Reply with exactly two files, each in a tagged block, and nothing else besides
short prose before or between them:

<file name=\"main.glsl\">
...the whole shader, starting with the reading header comment...
</file>

<file name=\"scene.json\">
...the whole scene.json...
</file>

No `script.js`, and `scene.json` must not use `IMAGES`, `CUBEMAPS`, `MEDIA` or
`PASS_SOURCES`: the scene is procedural, one `main.glsl`, no reference media.
Its keys are limited to TITLE, CREDIT, DESCRIPTION, WIDTH, HEIGHT, PASSES,
CONTROLS, TAGS, HARD_TRANSITIONS and SMOOTH_TRANSITIONS.
";

/// The skill's text, ready to be the system prompt.
#[derive(Debug, Clone)]
pub(crate) struct Skill {
    /// `SKILL.md` and the references, each under a heading, then the
    /// output contract.
    pub(crate) system: String,
    /// The files read, in order, relative to the skill folder.
    pub(crate) files: Vec<String>,
}

/// Reads the skill in `dir`: `SKILL.md`, then `reference/*` in name order.
pub(crate) fn load_skill(dir: &Path) -> Result<Skill> {
    let mut files = vec!["SKILL.md".to_owned()];
    let reference = dir.join("reference");
    let mut names: Vec<String> = std::fs::read_dir(&reference)
        .with_context(|| format!("reading {}", reference.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .map(|entry| format!("reference/{}", entry.file_name().to_string_lossy()))
        .collect();
    names.sort();
    files.extend(names);
    let mut system = String::new();
    for file in &files {
        let path = dir.join(file);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        system.push_str(&format!("# File: {file}\n\n{}\n\n", text.trim_end()));
    }
    system.push_str(CONTRACT);
    Ok(Skill { system, files })
}

/// `image` (encoded) resized so its long side is at most [`LONG_SIDE`] and
/// re-encoded as JPEG, base64, as a message part.
pub(crate) fn image_part(encoded: &[u8]) -> Result<Part> {
    let picture =
        decode_oriented(encoded).map_err(|e| anyhow!("the picture does not decode: {e}"))?;
    let (w, h) = (picture.width(), picture.height());
    let picture = if w.max(h) > LONG_SIDE {
        picture.resize(LONG_SIDE, LONG_SIDE, image::imageops::FilterType::Lanczos3)
    } else {
        picture
    };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, QUALITY)
        .encode_image(&picture.to_rgb8())?;
    Ok(Part::Image {
        media_type: "image/jpeg".to_owned(),
        base64: base64::engine::general_purpose::STANDARD.encode(jpeg),
    })
}

/// Decodes `encoded` and turns it upright by its EXIF orientation, as a
/// phone photo's viewer would; a picture without one is left as stored.
pub(crate) fn decode_oriented(encoded: &[u8]) -> Result<image::DynamicImage> {
    use image::ImageDecoder as _;
    let mut decoder = image::ImageReader::new(std::io::Cursor::new(encoded))
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut picture = image::DynamicImage::from_decoder(decoder)?;
    picture.apply_orientation(orientation);
    Ok(picture)
}

/// The first turn: the picture and the request.
pub(crate) fn first(picture: Part) -> Vec<Part> {
    vec![
        picture,
        Part::Text(
            "Make a Strobe scene of this artwork, following the skill: read the photo \
             (write the reading as the header comment), plan, build from the template. \
             Answer with the two files."
                .to_owned(),
        ),
    ]
}

/// A repair turn: what was wrong with the last answer.
pub(crate) fn repair(problem: &str) -> Vec<Part> {
    vec![Part::Text(format!(
        "That answer could not be used:\n\n```text\n{}\n```\n\nFix it and answer with both files again, whole.",
        problem.trim()
    ))]
}

/// A critique turn: the forge's report and sheet for the current scene.
pub(crate) fn critique(report: &str, sheet: Option<Part>, note: Option<&str>) -> Vec<Part> {
    let mut parts = Vec::new();
    if let Some(note) = note {
        parts.push(Part::Text(note.to_owned()));
    }
    if let Some(sheet) = sheet {
        parts.push(sheet);
    }
    parts.push(Part::Text(format!(
        "This is the forge's check of the current scene. The sheet shows the photo first, \
         then frames at the times in the report.\n\n{}\n\nGo through reference/critique.md \
         against the photo: write the differences as a list, fix the worst three, and answer \
         with the revised files, whole.",
        report.trim()
    )));
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::ensure;

    fn skill_dir() -> Result<std::path::PathBuf> {
        let dir = crate::fixture::tempdir("skill")?;
        std::fs::create_dir_all(dir.join("reference"))?;
        std::fs::write(
            dir.join("SKILL.md"),
            "# Scene from image\nRead the photo.\n",
        )?;
        std::fs::write(dir.join("reference/grammar.md"), "Archetypes.\n")?;
        std::fs::write(dir.join("reference/critique.md"), "Compare.\n")?;
        std::fs::write(
            dir.join("reference/template.glsl"),
            "vec4 renderMain(void) {}\n",
        )?;
        Ok(dir)
    }

    #[test]
    fn the_skill_comes_first_then_references_in_name_order_then_the_contract() -> Result<()> {
        let skill = load_skill(&skill_dir()?)?;
        ensure!(
            skill.files
                == [
                    "SKILL.md",
                    "reference/critique.md",
                    "reference/grammar.md",
                    "reference/template.glsl"
                ],
            "order: {:?}",
            skill.files
        );
        let at = |needle: &str| skill.system.find(needle);
        ensure!(
            at("Read the photo.") < at("# File: reference/critique.md")
                && at("Compare.") < at("Archetypes.")
                && at("Archetypes.") < at("renderMain")
                && at("renderMain") < at("# How to answer"),
            "assembled in order:\n{}",
            skill.system
        );
        ensure!(
            skill.system.contains("<file name=\"scene.json\">"),
            "the contract names the format"
        );
        Ok(())
    }

    #[test]
    fn a_large_picture_is_sent_at_most_1568_on_its_long_side() -> Result<()> {
        let big = image::RgbImage::from_pixel(3000, 1500, image::Rgb([200, 100, 50]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(big)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
        let Part::Image { media_type, base64 } = image_part(&png)? else {
            return Err(anyhow!("an image part"));
        };
        ensure!(media_type == "image/jpeg", "sent as JPEG");
        let bytes = base64::engine::general_purpose::STANDARD.decode(base64)?;
        let sent = image::load_from_memory(&bytes)?;
        ensure!(
            sent.width() == 1568 && sent.height() == 784,
            "resized keeping aspect: {}x{}",
            sent.width(),
            sent.height()
        );
        Ok(())
    }

    /// A JPEG with an EXIF block saying "rotate 90 degrees clockwise"
    /// (orientation 6) inserted after its start marker.
    fn jpeg_with_orientation(width: u32, height: u32, orientation: u8) -> Result<Vec<u8>> {
        let picture = image::RgbImage::from_pixel(width, height, image::Rgb([90, 120, 200]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90).encode_image(&picture)?;
        let mut tiff = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01".to_vec();
        tiff.extend_from_slice(&[0x01, 0x12, 0x00, 0x03, 0, 0, 0, 1, 0, orientation, 0, 0]);
        tiff.extend_from_slice(&[0, 0, 0, 0]);
        let length = u16::try_from(tiff.len() + 2)?.to_be_bytes();
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xff, 0xe1, length[0], length[1]]);
        out.extend_from_slice(&tiff);
        out.extend_from_slice(&jpeg[2..]);
        Ok(out)
    }

    #[test]
    fn a_phone_photo_is_turned_upright() -> Result<()> {
        let rotated = decode_oriented(&jpeg_with_orientation(40, 20, 6)?)?;
        ensure!(
            (rotated.width(), rotated.height()) == (20, 40),
            "orientation 6 turns a 40x20 landscape upright: {}x{}",
            rotated.width(),
            rotated.height()
        );
        let plain = decode_oriented(&jpeg_with_orientation(40, 20, 1)?)?;
        ensure!(
            (plain.width(), plain.height()) == (40, 20),
            "orientation 1 leaves it: {}x{}",
            plain.width(),
            plain.height()
        );
        Ok(())
    }

    #[test]
    fn a_critique_carries_the_note_the_sheet_and_the_report() -> Result<()> {
        let parts = critique(
            "# Forge check",
            Some(Part::Image {
                media_type: "image/jpeg".to_owned(),
                base64: "AA==".to_owned(),
            }),
            Some("The last revision failed."),
        );
        ensure!(parts.len() == 3, "note, sheet, report: {parts:?}");
        ensure!(
            matches!(parts.last(), Some(Part::Text(text)) if text.contains("# Forge check")),
            "the report comes last"
        );
        Ok(())
    }
}
