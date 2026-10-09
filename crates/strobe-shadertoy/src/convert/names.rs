// ============================================================================
// Module:       strobe_shadertoy::convert::names
// Description:  Every name and file path the converter generates, and the
//               scene folder's name.
//
// Dependencies: std only
// ============================================================================

//! Generated names.
//!
//! Everything the converter adds to a scene has a name that starts `st_`
//! (scene-level: controls, pass targets, images) or `strobe_` (inside GLSL
//! shim code), so it cannot collide with a Shadertoy author's identifiers
//! in practice, and the converter checks that it does not in fact
//! ([`super::ConvertError::NameClash`]).
//!
//! The control names are not the plain `speed`, `zoom` and `hue` of the
//! plan. They are uniforms declared at global scope, `zoom` and `speed`
//! are among the commonest identifiers in Shadertoy code, and a shader
//! with `const float zoom = 2.0;` would then fail to compile. The prefix
//! costs a slightly less pretty label and removes that whole class of
//! failure (D79).

/// Control: playback speed multiplier for `iTime`.
pub(super) const SPEED: &str = "st_speed";
/// Control: how much `iTime`'s pace follows the bass.
pub(super) const AUDIO_TIME: &str = "st_audio_time";
/// Control: brightness kick on bass hits.
pub(super) const BASS_FLASH: &str = "st_bass_flash";
/// Control: zoom of the Image pass.
pub(super) const ZOOM: &str = "st_zoom";
/// Control: hue rotation of the Image pass, degrees.
pub(super) const HUE: &str = "st_hue";

/// The group the five controls sit in.
pub(super) const CONTROL_GROUP: &str = "Strobe";

/// Pass target of the clock pass.
pub(super) const CLOCK_TARGET: &str = "st_clock";
/// Where the clock pass's code lives.
pub(super) const CLOCK_FILE: &str = "passes/st_clock.glsl";
/// The black image's sampler name.
pub(super) const BLACK_IMAGE: &str = "st_black";
/// Where the black image lives.
pub(super) const BLACK_FILE: &str = "images/st_black.png";
/// The standard sampler the player provides for audio inputs.
pub(super) const AUDIO_SAMPLER: &str = "strobe_ShadertoyAudio";

/// The Image pass's file.
pub(super) const IMAGE_FILE: &str = "main.glsl";

/// The pass target for Shadertoy buffer `letter` (0 is A).
pub(super) fn buffer_target(letter: usize) -> String {
    format!("st_buf_{}", letter_name(letter).to_ascii_lowercase())
}

/// The pass file for Shadertoy buffer `letter`.
pub(super) fn buffer_file(letter: usize) -> String {
    format!(
        "passes/buffer_{}.glsl",
        letter_name(letter).to_ascii_lowercase()
    )
}

/// The sampler name of the `n`th distinct texture.
pub(super) fn image_name(n: usize) -> String {
    format!("st_tex{n}")
}

/// `A` to `D` (and `?` beyond, which validation prevents).
pub(super) fn letter_name(letter: usize) -> char {
    char::from(b'A'.wrapping_add(u8::try_from(letter).unwrap_or(0xff)))
}

/// Every name the converter may declare in a scene, for clash checking.
pub(super) fn reserved() -> Vec<String> {
    let mut names: Vec<String> = [
        SPEED,
        AUDIO_TIME,
        BASS_FLASH,
        ZOOM,
        HUE,
        CLOCK_TARGET,
        BLACK_IMAGE,
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    names.extend((0..4).map(buffer_target));
    names.extend((0..16).map(image_name));
    names
}

/// A file-system-safe, lower-case slug of `text`: letters and digits, with
/// runs of anything else collapsed to one `-`.
pub(super) fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-')
        .chars()
        .take(40)
        .collect::<String>()
}

/// `<slug>-<id>.synScene`, or whichever half exists.
pub(super) fn folder_name(title: &str, id: Option<&str>) -> String {
    let slug = slug(title);
    let id = id.map(|id| {
        id.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
    });
    let stem = match (slug.is_empty(), id.as_deref()) {
        (true, None | Some("")) => "shadertoy".to_string(),
        (true, Some(id)) => format!("shadertoy-{id}"),
        (false, None | Some("")) => slug,
        (false, Some(id)) => format!("{slug}-{id}"),
    };
    format!("{stem}.synScene")
}

/// A safe file name for a downloaded asset: the part of `src` after the
/// last `/`, with anything outside `[A-Za-z0-9._-]` replaced by `_`.
pub(super) fn asset_file_name(src: &str) -> String {
    let base = src
        .split(['?', '#'])
        .next()
        .unwrap_or(src)
        .rsplit('/')
        .next()
        .unwrap_or(src);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_start_matches('.');
    if cleaned.is_empty() {
        "asset".to_string()
    } else {
        cleaned.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_safe_and_short() {
        assert_eq!(
            slug("Creation by Silexars!"),
            "creation-by-silexars",
            "words"
        );
        assert_eq!(
            slug("  --Hello__World--  "),
            "hello-world",
            "edges and runs"
        );
        assert_eq!(slug("\u{3c0}\u{3c0}"), "", "nothing ASCII");
        assert_eq!(slug(&"a".repeat(100)).len(), 40, "capped");
    }

    #[test]
    fn folder_names_combine_title_and_id() {
        assert_eq!(
            folder_name("Tunnel Lights", Some("XsXXDn")),
            "tunnel-lights-XsXXDn.synScene",
            "both"
        );
        assert_eq!(folder_name("Tunnel", None), "tunnel.synScene", "no id");
        assert_eq!(
            folder_name("\u{3c0}", Some("abc123")),
            "shadertoy-abc123.synScene",
            "no slug"
        );
        assert_eq!(folder_name("", None), "shadertoy.synScene", "nothing");
        assert_eq!(
            folder_name("x", Some("../..")),
            "x.synScene",
            "an id cannot escape the folder"
        );
    }

    #[test]
    fn asset_names_cannot_escape_images() {
        assert_eq!(
            asset_file_name("/media/a/585f95.jpg"),
            "585f95.jpg",
            "plain"
        );
        assert_eq!(
            asset_file_name("https://x.test/a/b/pic.png?v=2"),
            "pic.png",
            "url with query"
        );
        assert_eq!(
            asset_file_name("../../etc/passwd"),
            "passwd",
            "traversal drops to a base name"
        );
        assert_eq!(asset_file_name("/a/.hidden"), "hidden", "no leading dots");
        assert_eq!(
            asset_file_name("/a/we ird$.png"),
            "we_ird_.png",
            "odd characters"
        );
        assert_eq!(asset_file_name("/"), "asset", "nothing left");
    }

    #[test]
    fn letters_and_targets_follow_shadertoy() {
        assert_eq!(buffer_target(0), "st_buf_a", "A");
        assert_eq!(buffer_file(3), "passes/buffer_d.glsl", "D");
    }
}
