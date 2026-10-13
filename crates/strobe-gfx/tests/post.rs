// ============================================================================
// Module:       post (integration test)
// Description:  The output post stage against a real GLSL compiler: neutral
//               values reproduce the input bit for bit, and each control does
//               what it says on known colours, in both dialects.
//
// Dependencies: anyhow; strobe-gfx (headless context, post stage, gl)
// ============================================================================

//! Post-stage tests.
//!
//! A tiny texture of known colours stands in for a finished frame, so the
//! expected output is arithmetic. Like `render.rs`, each test opens a
//! headless EGL context (Mesa llvmpipe on CI) and reports a skip when none
//! exists, unless `STROBE_REQUIRE_HEADLESS_GL` is set.

use anyhow::{Result, anyhow};
use strobe_gfx::gl::{Format, Gl, Sampling, Texture};
use strobe_gfx::{Api, Caps, Headless, Post};
use strobe_scene::meta::MetaValues;

const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];

fn context(api: Api) -> Result<Option<Headless>> {
    match Headless::new(api) {
        Ok(headless) => Ok(Some(headless)),
        Err(error) if std::env::var_os("STROBE_REQUIRE_HEADLESS_GL").is_none() => {
            eprintln!("skipping: no headless {api:?} context ({error})");
            Ok(None)
        }
        Err(error) => Err(anyhow!("headless {api:?} context required: {error}")),
    }
}

/// A texture whose pixel at (`x`, `y`), `y` counted from the top, is
/// `color(x, y)`.
fn source(gl: &Gl, size: [i32; 2], color: impl Fn(i32, i32) -> [u8; 4]) -> Result<Texture> {
    let mut bytes = Vec::new();
    for row in (0..size[1]).rev() {
        for x in 0..size[0] {
            bytes.extend_from_slice(&color(x, row));
        }
    }
    gl.texture_2d(
        size[0],
        size[1],
        Format::Rgba8,
        Sampling::LINEAR_CLAMP,
        Some(&bytes),
    )
    .map_err(|e| anyhow!(e))
}

/// A texture read back as RGBA8, top row first.
fn read(gl: &Gl, texture: &Texture) -> Result<Vec<u8>> {
    let fbo = gl.framebuffer(texture).map_err(|e| anyhow!(e))?;
    let (w, h) = (texture.width(), texture.height());
    let bottom_up = gl.read_rgba8(&fbo, w, h);
    let row = w as usize * 4;
    Ok(bottom_up
        .chunks_exact(row)
        .rev()
        .flatten()
        .copied()
        .collect())
}

fn pixel(pixels: &[u8], width: i32, x: i32, y: i32) -> Result<[u8; 4]> {
    let offset = ((y * width + x) * 4) as usize;
    let px = pixels
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow!("readback too short"))?;
    Ok([px[0], px[1], px[2], px[3]])
}

fn near(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual
        .iter()
        .zip(&expected)
        .all(|(a, e)| a.abs_diff(*e) <= 2)
}

/// Runs the program on a 1x1 `color` and returns what comes out.
fn one_pixel(api: Api, color: [u8; 4], values: &MetaValues) -> Result<Option<[u8; 4]>> {
    let Some(headless) = context(api)? else {
        return Ok(None);
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let mut post = Post::new(gl, &caps, [1, 1])?;
    let src = source(gl, [1, 1], |_, _| color)?;
    post.render(&src, values)?;
    pixel(&post.read_output(), 1, 0, 0).map(Some)
}

fn with(f: impl FnOnce(&mut MetaValues)) -> MetaValues {
    let mut values = MetaValues::default();
    f(&mut values);
    values
}

#[test]
fn neutral_values_through_the_program_are_the_input_bit_for_bit() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let size = [37, 11];
        let mut post = Post::new(gl, &caps, size)?;
        // Every red level and a scattering of the others, with a varying
        // alpha: nothing a shader could round the same by luck.
        let src = source(gl, size, |x, y| {
            let n = (x + y * size[0]) as u32;
            [
                (n * 7 % 256) as u8,
                (n * 13 % 256) as u8,
                (255 - n * 5 % 256) as u8,
                (n * 3 % 256) as u8,
            ]
        })?;
        post.render(&src, &MetaValues::default())?;
        assert_eq!(
            post.read_output(),
            read(gl, &src)?,
            "{api:?}: a neutral pass must equal the input exactly"
        );
        assert!(
            !post.apply(&src, &MetaValues::default())?,
            "{api:?}: apply() draws nothing at neutral"
        );
    }
    Ok(())
}

#[test]
fn invert_flips_each_colour_channel_and_keeps_alpha() -> Result<()> {
    for api in BOTH {
        let Some(px) = one_pixel(api, [255, 0, 64, 200], &with(|v| v.invert = true))? else {
            continue;
        };
        assert!(near(px, [0, 255, 191, 200]), "{api:?}: got {px:?}");
    }
    Ok(())
}

#[test]
fn master_scales_to_black_and_halves() -> Result<()> {
    for api in BOTH {
        let white = [255, 255, 255, 255];
        let Some(black) = one_pixel(api, white, &with(|v| v.master = 0.0))? else {
            continue;
        };
        assert!(near(black, [0, 0, 0, 255]), "{api:?}: master 0: {black:?}");
        // Master wins over invert: a blacked-out inverted frame is black.
        let inverted = one_pixel(api, [0; 4], &with(|v| (v.master, v.invert) = (0.0, true)))?;
        assert!(
            inverted.is_some_and(|px| near(px, [0, 0, 0, 0])),
            "{api:?}: master 0 with invert: {inverted:?}"
        );
        let half = one_pixel(api, white, &with(|v| v.master = 0.5))?;
        assert!(
            half.is_some_and(|px| near(px, [128, 128, 128, 255])),
            "{api:?}: master 0.5: {half:?}"
        );
    }
    Ok(())
}

#[test]
fn hue_rotates_and_saturation_greys() -> Result<()> {
    for api in BOTH {
        let red = [255, 0, 0, 255];
        let Some(green) = one_pixel(api, red, &with(|v| v.hue = 1.0 / 3.0))? else {
            continue;
        };
        assert!(
            near(green, [0, 255, 0, 255]),
            "{api:?}: a third turn: {green:?}"
        );
        let blue = one_pixel(api, red, &with(|v| v.hue = 2.0 / 3.0))?;
        assert!(
            blue.is_some_and(|px| near(px, [0, 0, 255, 255])),
            "{api:?}: two thirds: {blue:?}"
        );
        let grey = one_pixel(api, red, &with(|v| v.saturation = 0.0))?;
        // Rec. 709 luma of pure red: 0.2126 * 255 = 54.2.
        assert!(
            grey.is_some_and(|px| near(px, [54, 54, 54, 255])),
            "{api:?}: saturation 0: {grey:?}"
        );
        let same = one_pixel(api, [200, 100, 50, 255], &with(|v| v.saturation = 1.0))?;
        assert!(
            same.is_some_and(|px| near(px, [200, 100, 50, 255])),
            "{api:?}: saturation 1 is the picture: {same:?}"
        );
    }
    Ok(())
}

#[test]
fn brightness_contrast_and_gamma_move_the_tones() -> Result<()> {
    for api in BOTH {
        let mid = [128, 128, 128, 255];
        let Some(dark) = one_pixel(api, mid, &with(|v| v.brightness = -1.0))? else {
            continue;
        };
        assert!(
            near(dark, [0, 0, 0, 255]),
            "{api:?}: brightness -1: {dark:?}"
        );
        let flat = one_pixel(api, [250, 10, 90, 255], &with(|v| v.contrast = 0.0))?;
        assert!(
            flat.is_some_and(|px| near(px, [128, 128, 128, 255])),
            "{api:?}: contrast 0 is mid grey: {flat:?}"
        );
        // gamma 2 raises 0.5 to 1/2: 0.7071 -> 180.
        let lifted = one_pixel(api, [128, 128, 128, 255], &with(|v| v.gamma = 2.0))?;
        assert!(
            lifted.is_some_and(|px| px[0].abs_diff(181) <= 2),
            "{api:?}: gamma 2 lifts the mid-tones: {lifted:?}"
        );
    }
    Ok(())
}

#[test]
fn mirrors_reflect_one_half_onto_the_other_exactly() -> Result<()> {
    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        // Odd and even widths and heights: the centre column of an odd
        // size is its own mirror.
        for size in [[8, 6], [7, 5]] {
            let mut post = Post::new(gl, &caps, size)?;
            // Left half red, right half blue; bottom half red, top blue.
            let sideways = source(
                gl,
                size,
                |x, _| if x * 2 < size[0] + 1 { RED } else { BLUE },
            )?;
            post.render(&sideways, &with(|v| v.horizontal_mirror = true))?;
            let out = post.read_output();
            for y in 0..size[1] {
                for x in 0..size[0] {
                    // The centre column of an odd width keeps its own colour.
                    let expect = RED;
                    assert_eq!(
                        pixel(&out, size[0], x, y)?,
                        expect,
                        "{api:?} {size:?}: horizontal mirror, ({x},{y})"
                    );
                }
            }
            // Without the mirror the right half stays blue.
            post.render(&sideways, &MetaValues::default())?;
            assert_eq!(
                pixel(&post.read_output(), size[0], size[0] - 1, 0)?,
                BLUE,
                "{api:?} {size:?}: unmirrored"
            );

            let upright = source(
                gl,
                size,
                |_, y| if y * 2 < size[1] - 1 { BLUE } else { RED },
            )?;
            post.render(&upright, &with(|v| v.vertical_mirror = true))?;
            let out = post.read_output();
            for y in 0..size[1] {
                assert_eq!(
                    pixel(&out, size[0], 0, y)?,
                    RED,
                    "{api:?} {size:?}: vertical mirror keeps the bottom, row {y}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn a_mirror_is_a_true_reflection_of_a_gradient() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let size = [9, 4];
        let mut post = Post::new(gl, &caps, size)?;
        let src = source(gl, size, |x, _| [(x * 20) as u8, 0, 0, 255])?;
        post.render(&src, &with(|v| v.horizontal_mirror = true))?;
        let out = post.read_output();
        for x in 0..size[0] {
            let from = if x * 2 > size[0] - 1 {
                size[0] - 1 - x
            } else {
                x
            };
            assert_eq!(
                pixel(&out, size[0], x, 1)?[0],
                (from * 20) as u8,
                "{api:?}: column {x} shows column {from}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_size_change_reallocates_the_target_and_apply_runs_only_when_needed() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let mut post = Post::new(gl, &caps, [4, 4])?;
        let src = source(gl, [6, 2], |_, _| [10, 20, 30, 255])?;
        let before = gl.shader_compiles();
        assert!(
            post.apply(&src, &with(|v| v.invert = true))?,
            "{api:?}: a changed picture renders"
        );
        assert_eq!(
            post.size(),
            [6, 2],
            "{api:?}: the target followed the source"
        );
        assert_eq!(
            gl.shader_compiles(),
            before,
            "{api:?}: no shader compiles after the stage is built"
        );
        let px = pixel(&post.read_output(), 6, 5, 1)?;
        assert!(near(px, [245, 235, 225, 255]), "{api:?}: {px:?}");
    }
    Ok(())
}

#[test]
fn limit_colors_maps_luma_onto_the_two_colours() -> Result<()> {
    let values = with(|v| {
        v.limit_colors = true;
        v.limit_low = [0.0, 0.0, 1.0];
        v.limit_high = [1.0, 1.0, 0.0];
    });
    for api in BOTH {
        for (input, expected, what) in [
            ([0, 0, 0, 255], [0, 0, 255, 255], "black becomes low"),
            (
                [255, 255, 255, 255],
                [255, 255, 0, 255],
                "white becomes high",
            ),
            (
                [128, 128, 128, 77],
                [128, 128, 127, 77],
                "grey is halfway, alpha kept",
            ),
        ] {
            let Some(px) = one_pixel(api, input, &values)? else {
                continue;
            };
            assert!(near(px, expected), "{api:?}: {what}: {px:?}");
        }
    }
    Ok(())
}
