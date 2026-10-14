// ============================================================================
// Module:       strobe_scene::pin
// Description:  Whether a scene's final pass is pinned to its frame size
//               (D280): it has a root-sized absolute buffer pass and its
//               shader reads one texel by texel.
//
// Dependencies: none (the crate's own GLSL tokenizer)
// ============================================================================

//! The final-pass pin (D280).
//!
//! A scene with a root `WIDTH` x `HEIGHT` and a literal buffer pass of
//! exactly that size has a full frame at the size it was written for
//! ([`SceneDef::frame_size`]). Most such scenes sample it by `_uv`, which
//! reads the whole buffer at any final-pass size, so the render scale keeps
//! lightening their final pass. A few read it with `texelFetch(pass,
//! ivec2(_xy), 0)`, which addresses pixels: with a smaller final pass that
//! reads only the buffer's bottom-left corner (Synesthesia's `neon_city`
//! under a render scale of 0.5 drew a quarter of the city). Those are
//! pinned: their final pass renders at the frame size whatever the host
//! asks, and the output stage scales it to the window.
//!
//! The test is on the shader's tokens, once, when the player is built:
//! comments are skipped and `#include`s were expanded when the folder was
//! read. A `texelFetch` (or `texelFetchOffset`) whose sampler is
//!
//! - a frame-sized pass: pins;
//! - a pass of another size, an image, a cube map or a `syn_` texture:
//!   does not, since none of those is the frame;
//! - anything else, such as a helper function's `sampler2D` parameter
//!   (`neon_city` blurs its passes through one): pins, because it may be
//!   handed a frame-sized pass.
//!
//! Only `texelFetch` takes integer texel coordinates, so it is the one call
//! that misreads a frame-sized pass from a smaller final pass. A scene that
//! computes uv from pixels itself (`texture(pass, _xy / vec2(1920.0,
//! 1080.0))`) would misread it too; none of Synesthesia's 90 does, and such
//! a scene can still be pinned by reading the pass with `texelFetch`.

use crate::load::SceneFolder;
use crate::promote::lex::{Kind, lex};
use crate::scene::SceneDef;

/// The calls whose first argument is read by integer texel coordinates.
const TEXEL_CALLS: [&str; 2] = ["texelFetch", "texelFetchOffset"];

/// The size `scene`'s final pass is pinned to, or `None` when it follows
/// the output and the render scale. See the module docs.
pub fn pinned_size(scene: &SceneFolder) -> Option<[i32; 2]> {
    let frame = scene.def.frame_size?;
    let mut sources =
        std::iter::once(scene.shader.as_str()).chain(scene.pass_shaders.iter().map(String::as_str));
    sources
        .any(|source| fetches_a_frame_pass(&scene.def, frame, source))
        .then_some(frame)
}

/// Whether `source`, or a macro it defines, texel-fetches something that
/// may be a pass of size `frame`.
fn fetches_a_frame_pass(def: &SceneDef, frame: [i32; 2], source: &str) -> bool {
    let lexed = lex(source);
    calls_fetch_a_frame_pass(def, frame, source, &lexed.tokens)
        || lexed.defines.iter().any(|define| {
            let body = lex(define.body);
            calls_fetch_a_frame_pass(def, frame, define.body, &body.tokens)
        })
}

/// Whether one of `tokens` (of `text`) is a texel call whose sampler may be
/// a frame-sized pass.
fn calls_fetch_a_frame_pass(
    def: &SceneDef,
    frame: [i32; 2],
    text: &str,
    tokens: &[crate::promote::lex::Token],
) -> bool {
    let word = |index: usize| {
        tokens
            .get(index)
            .and_then(|token| text.get(token.start..token.end))
    };
    tokens.iter().enumerate().any(|(index, token)| {
        token.kind == Kind::Ident
            && word(index).is_some_and(|name| TEXEL_CALLS.contains(&name))
            && word(index + 1) == Some("(")
            && tokens
                .get(index + 2)
                .is_some_and(|arg| arg.kind == Kind::Ident)
            && word(index + 2).is_some_and(|sampler| may_be_a_frame_pass(def, frame, sampler))
    })
}

/// Whether the sampler called `name` may be a pass of size `frame`: it is
/// one, or it is not one of the scene's other samplers.
fn may_be_a_frame_pass(def: &SceneDef, frame: [i32; 2], name: &str) -> bool {
    if let Some(pass) = def
        .passes
        .iter()
        .find(|pass| pass.target.as_deref() == Some(name))
    {
        return pass.fixed == Some(frame);
    }
    let other = name.starts_with("syn_")
        || def.images.iter().any(|image| image.name == name)
        || def.cubemaps.iter().any(|cube| cube.name == name);
    !other
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::path::PathBuf;

    /// A scene with a root size, a frame-sized pass `frame`, a 10x10 data
    /// pass `data` and an image `img`, running `shader`.
    fn folder(shader: &str) -> Result<SceneFolder> {
        Ok(SceneFolder {
            dir: PathBuf::from("pin-test"),
            def: SceneDef::parse(
                r#"{ "WIDTH": 1920, "HEIGHT": 1080,
                     "PASSES": [ {"TARGET": "frame", "WIDTH": 1920, "HEIGHT": 1080},
                                 {"TARGET": "data", "WIDTH": 10, "HEIGHT": 10} ],
                     "IMAGES": [ {"NAME": "img", "PATH": "img.png"} ] }"#,
                "pin-test",
            )?,
            shader: shader.to_string(),
            pass_shaders: Vec::new(),
            script: None,
        })
    }

    fn pinned(shader: &str) -> Result<Option<[i32; 2]>> {
        Ok(pinned_size(&folder(shader)?))
    }

    #[test]
    fn a_texel_fetch_of_a_frame_sized_pass_pins_the_final_pass() -> Result<()> {
        assert_eq!(
            pinned("vec4 renderMain() { return texelFetch(frame, ivec2(_xy), 0); }")?,
            Some([1920, 1080]),
            "a direct fetch pins"
        );
        assert_eq!(
            pinned(
                "vec4 renderMain() { return texelFetchOffset ( frame, ivec2(_xy), 0, ivec2(1) ); }"
            )?,
            Some([1920, 1080]),
            "so does texelFetchOffset, spaced out"
        );
        assert_eq!(
            pinned(
                "vec3 blur(sampler2D p, ivec2 xy) { return texelFetch(p, xy, 0).rgb; }\n\
                 vec4 renderMain() { return vec4(blur(frame, ivec2(_xy)), 1.0); }"
            )?,
            Some([1920, 1080]),
            "a helper's sampler parameter may be the frame, so it pins"
        );
        assert_eq!(
            pinned(
                "#define FETCH(s) texelFetch(s, ivec2(_xy), 0)\n\
                 vec4 renderMain() { return FETCH(frame); }"
            )?,
            Some([1920, 1080]),
            "a fetch inside a macro counts"
        );
        Ok(())
    }

    #[test]
    fn sampling_by_uv_or_fetching_other_textures_leaves_the_final_pass_free() -> Result<()> {
        for (shader, why) in [
            (
                "vec4 renderMain() { return texture(frame, _uv); }",
                "sampling by uv reads the whole pass at any size",
            ),
            (
                "vec4 renderMain() { return texelFetch(data, ivec2(0), 0) + texture(frame, _uv); }",
                "a fetch from a pass of another size is not the frame",
            ),
            (
                "vec4 renderMain() { return texelFetch(syn_Spectrum, 3, 0) + texelFetch(img, ivec2(0), 0); }",
                "nor is a syn_ texture or an image",
            ),
            (
                "// return texelFetch(frame, ivec2(_xy), 0);\n\
                 /* texelFetch(frame, ivec2(_xy), 0) */\n\
                 vec4 renderMain() { return texture(frame, _uv); }",
                "a fetch in a comment is not code",
            ),
        ] {
            assert_eq!(pinned(shader)?, None, "{why}");
        }
        Ok(())
    }

    #[test]
    fn a_scene_without_a_frame_sized_pass_is_never_pinned() -> Result<()> {
        let mut scene = folder("vec4 renderMain() { return texelFetch(frame, ivec2(_xy), 0); }")?;
        scene.def = SceneDef::parse(
            r#"{ "PASSES": [ {"TARGET": "frame", "WIDTH": 1920, "HEIGHT": 1080} ] }"#,
            "pin-test",
        )?;
        assert_eq!(
            pinned_size(&scene),
            None,
            "no root size declared, so no pass is the scene's frame"
        );
        Ok(())
    }

    #[test]
    fn a_per_pass_source_that_fetches_pins() -> Result<()> {
        let mut scene = folder("vec4 renderMain() { return texture(frame, _uv); }")?;
        scene.pass_shaders = vec![
            "vec4 renderMain() { return vec4(1.0); }".to_string(),
            "vec4 renderMain() { return vec4(0.0); }".to_string(),
            "vec4 renderMain() { return texelFetch(frame, ivec2(_xy), 0); }".to_string(),
        ];
        assert_eq!(
            pinned_size(&scene),
            Some([1920, 1080]),
            "PASS_SOURCES files are read too"
        );
        Ok(())
    }
}
