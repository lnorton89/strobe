// ============================================================================
// Module:       render (integration test)
// Description:  Compiles and renders scenes on headless GL in both dialects:
//               desktop 3.3 core (native path) and GLES 3.0 (ANGLE path).
//
// Dependencies: anyhow; strobe-gfx (headless context, player), strobe-scene
// ============================================================================

//! Renderer tests against a real GLSL compiler.
//!
//! Each test opens a headless EGL context — Mesa's llvmpipe on a Linux CI
//! runner, no GPU and no display needed — and asserts on pixels read back
//! from the scene's output. Where no headless EGL exists (a Windows
//! runner), the tests report a skip and pass, unless
//! `STROBE_REQUIRE_HEADLESS_GL` is set, which CI's Linux job does so a
//! missing driver cannot pass silently there.

use anyhow::{Result, anyhow};
use std::path::PathBuf;
use strobe_gfx::gl::trace::Event;
use strobe_gfx::gl::{Bindings, prime};
use strobe_gfx::{
    Api, AudioTextures, Caps, FrameInputs, GpuTimer, Headless, Mouse, Options, ScenePlayer,
};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;
use strobe_scene::scene::SceneDef;

/// A headless context, or `None` (after saying so) when none exists here.
fn context(api: Api) -> Result<Option<Headless>> {
    match Headless::new(api) {
        Ok(headless) => Ok(Some(headless)),
        Err(error) if std::env::var_os("STROBE_REQUIRE_HEADLESS_GL").is_none() => {
            strobe_gfx::skip::note(format_args!("no headless {api:?} context ({error})"));
            Ok(None)
        }
        Err(error) => Err(anyhow!("headless {api:?} context required: {error}")),
    }
}

fn folder(json: &str, shader: &str) -> Result<SceneFolder> {
    Ok(SceneFolder {
        dir: PathBuf::from("."),
        def: SceneDef::parse(json, "test")?,
        shader: shader.to_string(),
        pass_shaders: Vec::new(),
        script: None,
    })
}

/// Renders `frames` frames of a scene and returns the centre pixel.
fn render_center(
    api: Api,
    options: Options,
    json: &str,
    shader: &str,
    frames: u64,
) -> Result<Option<[u8; 4]>> {
    let Some(headless) = context(api)? else {
        return Ok(None);
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let scene = folder(json, shader)?;
    let controls = ControlBank::new(scene.def.controls.clone())?;
    let transitions: Vec<f32> = scene
        .def
        .transition_uniforms
        .iter()
        .map(|t| t.initial)
        .collect();
    let audio = vec![0.0; SCALARS.len()];
    let mut player = ScenePlayer::new(gl, &caps, &scene, [32, 18], options)?;
    for frame in 0..frames {
        player.render(&FrameInputs {
            time: frame as f32 / 60.0,
            frame,
            mouse: Mouse::default(),
            audio: &audio,
            controls: &controls,
            transitions: &transitions,
        });
    }
    if let Some(code) = gl.error() {
        return Err(anyhow!("GL error 0x{code:x}"));
    }
    let pixels = player.read_output();
    let offset = (9 * 32 + 16) * 4;
    let px = pixels
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow!("readback too short"))?;
    Ok(Some([px[0], px[1], px[2], px[3]]))
}

fn near(actual: u8, expected: u8) -> bool {
    actual.abs_diff(expected) <= 2
}

const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];

#[test]
fn a_control_value_reaches_the_shader() -> Result<()> {
    for api in BOTH {
        let Some(px) = render_center(
            api,
            Options::default(),
            r#"{ "CONTROLS": [ {"NAME": "amount", "TYPE": "slider", "DEFAULT": 0.5} ] }"#,
            "vec4 renderMain() { return vec4(amount, 0.0, 0.0, 1.0); }",
            1,
        )?
        else {
            continue;
        };
        assert!(near(px[0], 128) && px[1] == 0, "{api:?}: got {px:?}");
    }
    Ok(())
}

#[test]
fn the_ssf_library_compiles_in_both_dialects() -> Result<()> {
    // Touch a function from every family, so the driver cannot skip
    // compiling any of them.
    let shader = r#"
        vec4 renderMain() {
            vec2 p = _uvc;
            float n = _noise(p * 3.0) + _fbm(vec3(p, TIME), 4.0) + _noise(TIME);
            vec3 c = _hsv2rgb(vec3(n, 0.8, 0.9));
            c = _hueRotate(_contrast(_saturation(c, 1.2), 1.1), 30.0);
            c = _palette(n, vec3(0.5), vec3(0.5), vec3(1.0), vec3(0.0, 0.33, 0.67)) * c;
            vec2 q = _rotate(_toRect(_toPolarTrue(p)), 45.0) + _hash22(p) * 0.0;
            float m = _mix3(0.0, 0.5, 1.0, _nsin(TIME)) * _pulse(_uv.x, 0.5, 0.5) + _smin(q.x, q.y, 8.0) * 0.0;
            vec4 e = _edgeDetectSobel(syn_FinalPass) * 0.0 + _hBlurTiltShift(syn_FinalPass, 2.0, 0.5) * 0.0;
            vec4 media = _isMediaActive() ? _loadMedia() : vec4(0.0);
            return vec4(_gamma(c, 1.0) * m + e.rgb + media.rgb + _hash44(vec4(1.0)).rgb * 0.0, 1.0);
        }"#;
    for api in BOTH {
        render_center(api, Options::default(), "{}", shader, 2)?;
    }
    Ok(())
}

#[test]
fn a_buffer_pass_feeds_the_final_pass() -> Result<()> {
    let json = r#"{ "PASSES": [ {"TARGET": "buf"} ] }"#;
    let shader = r#"
        vec4 renderMain() {
            if (PASSINDEX == 0) { return vec4(0.25, 0.5, 0.75, 1.0); }
            return texture(buf, _uv);
        }"#;
    for api in BOTH {
        for specialise_passes in [true, false] {
            let options = Options {
                specialise_passes,
                ..Options::default()
            };
            let Some(px) = render_center(api, options, json, shader, 1)? else {
                continue;
            };
            assert!(
                near(px[0], 64) && near(px[1], 128) && near(px[2], 191),
                "{api:?} specialised={specialise_passes}: got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_double_buffered_pass_reads_its_previous_frame() -> Result<()> {
    let json = r#"{ "PASSES": [ {"TARGET": "acc", "DOUBLE_BUFFER": true} ] }"#;
    let shader = r#"
        vec4 renderMain() {
            if (PASSINDEX == 0) { return texture(acc, _uv) + vec4(0.1, 0.0, 0.0, 0.0); }
            return vec4(texture(acc, _uv).rgb, 1.0);
        }"#;
    for api in BOTH {
        let Some(px) = render_center(api, Options::default(), json, shader, 3)? else {
            continue;
        };
        assert!(near(px[0], 77), "{api:?}: three frames of +0.1, got {px:?}");
    }
    Ok(())
}

#[test]
fn syn_final_pass_is_last_frames_output() -> Result<()> {
    let shader = "vec4 renderMain() { return vec4(texture(syn_FinalPass, _uv).rgb + 0.1, 1.0); }";
    for api in BOTH {
        let Some(px) = render_center(api, Options::default(), "{}", shader, 4)? else {
            continue;
        };
        assert!(near(px[0], 102), "{api:?}: four frames of +0.1, got {px:?}");
    }
    Ok(())
}

#[test]
fn real_scene_idioms_compile_on_desktop_gl() -> Result<()> {
    // Real scenes compare PASSINDEX with a float literal, rely on implicit
    // int-to-float conversion, and use texture2D. Desktop GLSL accepts all
    // three, in either PASSINDEX mode.
    let json = r#"{ "PASSES": [ {"TARGET": "firstBuffer"} ] }"#;
    let shader = r#"
        vec4 renderMain(void) {
            float scale = 2;
            if (PASSINDEX == 0.0) { return vec4(0.5 * scale * 0.5); }
            else if (PASSINDEX == 1.0) { return texture2D(firstBuffer, _uv); }
        }"#;
    for specialise_passes in [true, false] {
        let options = Options {
            specialise_passes,
            ..Options::default()
        };
        let Some(px) = render_center(Api::DesktopCore33, options, json, shader, 1)? else {
            continue;
        };
        assert!(
            near(px[0], 128),
            "specialised={specialise_passes}: got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn passindex_compared_with_a_float_literal_compiles_in_both_dialects() -> Result<()> {
    // Q8: real scenes write `PASSINDEX == 0.0`, either way round, with `<`
    // and `!=` as well. Desktop GLSL converts the int; GLSL ES needs the
    // assembler's cast. Both PASSINDEX modes, both dialects, and the right
    // branch must run (a wrong cast would pick the wrong pass).
    let json = r#"{ "PASSES": [ {"TARGET": "firstBuffer"} ] }"#;
    let shader = r#"
        vec4 renderMain(void) {
            if (PASSINDEX == 0.0) { return vec4(0.25, 0.0, 0.0, 1.0); }
            if (1.0 == PASSINDEX && PASSINDEX != 0.0 && PASSINDEX > 0.5 && 0.5 < PASSINDEX) {
                return vec4(texture(firstBuffer, _uv).r * 2.0, 0.5, 0.0, 1.0);
            }
            return vec4(0.0, 0.0, 1.0, 1.0);
        }"#;
    for api in BOTH {
        for specialise_passes in [true, false] {
            let options = Options {
                specialise_passes,
                ..Options::default()
            };
            let Some(px) = render_center(api, options, json, shader, 1)? else {
                continue;
            };
            assert!(
                near(px[0], 128) && near(px[1], 128) && px[2] == 0,
                "{api:?} specialised={specialise_passes}: pass 0 wrote 0.25, the final pass doubled it; got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn implicit_int_to_float_and_non_macro_if_compile_in_both_dialects() -> Result<()> {
    // D199: desktop GLSL converts int to float implicitly and real scenes
    // rely on it everywhere; GLSL ES needs every conversion written out.
    // The values check that the rewrite keeps desktop's meaning: `1 / 2` is
    // an integer division (0), and `#if` on a global reads it as 0.
    let shader = "#define STEPS 4\n\
                  float TUBE = 3.0;\n\
                  float half_of(float x) { return x / 2; }\n\
                  float sgn(float x) { return (x < 0) ? -1 : 1; }\n\
                  vec4 renderMain() {\n\
                      float s = 0;\n\
                      for (int i = 0; i < STEPS; i++) { s += i * 0.0625; }\n\
                      float r = s + 1 / 2;\n\
                      float g = clamp(half_of(1), 0, 1) * sgn(3);\n\
                  #if TUBE == 0\n\
                      float b = max(0, 0.25);\n\
                  #else\n\
                      float b = 1;\n\
                  #endif\n\
                      return vec4(r, g, b, 1);\n\
                  }\n";
    for api in BOTH {
        let Some(px) = render_center(api, Options::default(), "{}", shader, 1)? else {
            continue;
        };
        assert!(
            near(px[0], 96) && near(px[1], 128) && near(px[2], 64),
            "{api:?}: 0.375, 0.5 and 0.25; got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn function_like_macros_mixing_int_and_float_compile_in_both_dialects() -> Result<()> {
    // C-02, D261: a function-like macro hides the int from the D199 pass,
    // so GLSL ES rejected `S(0, 1, x)`. ES types each call through its
    // expansion: an argument edit (`S`, `H`, `ROT`), or the call inlined
    // when the body needs one (`sat`) or a parameter is both an index and a
    // factor (`TAP`). Desktop is left as written. The values check the
    // meaning is desktop's.
    let shader = "#define S(a, b, t) smoothstep(a, b, t)\n\
                  #define ROT(a) mat2(cos(a), -sin(a), sin(a), cos(a))\n\
                  #define sat(x) clamp(x, 0, 1)\n\
                  #define H(n) fract(n * 0.5)\n\
                  const float w[2] = float[2](0.25, 0.375);\n\
                  #define TAP(i) (w[i] * i)\n\
                  vec4 renderMain() {\n\
                      float r = S(0, 1, 0.5);\n\
                      float g = sat(2.5) * TAP(1);\n\
                      float b = (ROT(0) * vec2(H(1), 0.0)).x * 0.5;\n\
                      return vec4(r, g, b, 1.0);\n\
                  }\n";
    for api in BOTH {
        let Some(px) = render_center(api, Options::default(), "{}", shader, 1)? else {
            continue;
        };
        assert!(
            near(px[0], 128) && near(px[1], 96) && near(px[2], 64),
            "{api:?}: 0.5, 0.375 and 0.25; got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn a_sampler_chosen_with_a_ternary_compiles_in_both_dialects() -> Result<()> {
    // C-02, D261: GLSL forbids a sampler as an operand of `?:`, and Mesa
    // and ANGLE enforce it on both dialects. The call moves inside the
    // choice. Each pass writes one colour, and the final pass must read the
    // buffer the condition picks.
    let json = r#"{ "PASSES": [ {"TARGET": "bufA"}, {"TARGET": "bufB"} ] }"#;
    let shader = "#define PICK(i, p) texture(i == 0 ? bufA : bufB, p)\n\
                  vec4 renderMain() {\n\
                      if (PASSINDEX == 0) { return vec4(0.25, 0.0, 0.0, 1.0); }\n\
                      if (PASSINDEX == 1) { return vec4(0.0, 0.5, 0.0, 1.0); }\n\
                      int pick = int(TIME) + 1;\n\
                      vec4 a = texture(pick == 0 ? bufA : bufB, _uv);\n\
                      vec4 b = textureLod((pick == 1) ? bufA\n\
                                                      : bufB, _uv, 0);\n\
                  #ifdef GL_ES\n\
                      vec4 c = PICK(pick - 1, _uv);\n\
                  #else\n\
                      vec4 c = texture(bufA, _uv);\n\
                  #endif\n\
                      return vec4(b.r, a.g, c.r, 1.0);\n\
                  }\n";
    for api in BOTH {
        let Some(px) = render_center(api, Options::default(), json, shader, 1)? else {
            continue;
        };
        assert!(
            near(px[0], 64) && near(px[1], 128) && near(px[2], 64),
            "{api:?}: bufA's 0.25, bufB's 0.5, and bufA's 0.25 through a macro; got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn a_sampler_choice_that_cannot_be_rewritten_fails_with_strobes_message_on_es() -> Result<()> {
    // Two choices in one call are not rewritten. GLSL ES rejects them
    // everywhere, so the ES program fails with Strobe's explanation at the
    // user's line, not only the driver's (D261).
    let Some(headless) = context(Api::Gles30)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let scene = SceneFolder {
        dir: PathBuf::from("."),
        def: SceneDef::parse(
            r#"{ "PASSES": [ {"TARGET": "bufA"}, {"TARGET": "bufB"} ] }"#,
            "test",
        )?,
        shader:
            "vec4 mixed(sampler2D a, sampler2D b) { return texture(a, _uv) + texture(b, _uv); }\n\
                 vec4 renderMain() {\n\
                     bool c = TIME > 1.0;\n\
                     return mixed(c ? bufA : bufB, c ? bufB : bufA);\n\
                 }\n"
            .to_string(),
        pass_shaders: Vec::new(),
        script: None,
    };
    let error = ScenePlayer::new(gl, &caps, &scene, [8, 8], Options::default())
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        error.contains("main.glsl:4") && error.contains("strobe: a sampler is chosen with ?:"),
        "expected Strobe's message at main.glsl:4, got: {error}"
    );
    Ok(())
}

#[test]
fn a_pass_that_reads_itself_sees_its_previous_frame_in_both_dialects() -> Result<()> {
    // D203: a single-buffered pass sampling its own target is a feedback
    // loop GL leaves undefined (NVIDIA: last frame; ANGLE/D3D11: zeros;
    // TORNADO T7). The player double-buffers it, so four frames of "+0.25"
    // accumulate to 1.0 everywhere.
    let json = r#"{ "PASSES": [ {"TARGET": "acc", "FLOAT": true, "WIDTH": 4, "HEIGHT": 4} ] }"#;
    let shader = "vec4 renderMain() {\n\
                      if (PASSINDEX == 0) { return texture(acc, _uv) + vec4(0.25); }\n\
                      return vec4(texture(acc, _uv).rgb * 0.5, 1.0);\n\
                  }\n";
    for api in BOTH {
        let Some(px) = render_center(api, Options::default(), json, shader, 4)? else {
            continue;
        };
        assert!(
            near(px[0], 128),
            "{api:?}: four frames of +0.25 make 1.0, halved to 0.5; got {px:?}"
        );
        // llvmpipe happens to read the old contents through a feedback loop,
        // so the value alone cannot tell; the buffering can.
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder(json, shader)?;
        let player = ScenePlayer::new(gl, &caps, &scene, [32, 18], Options::default())?;
        assert!(
            player.is_double_buffered(0),
            "{api:?}: the self-reading pass got a second target"
        );
        let plain = folder(json, "vec4 renderMain() { return vec4(0.5); }")?;
        let player = ScenePlayer::new(gl, &caps, &plain, [32, 18], Options::default())?;
        assert!(
            !player.is_double_buffered(0),
            "{api:?}: a pass that does not read itself keeps one"
        );
    }
    Ok(())
}

#[test]
fn a_non_constant_global_initializer_compiles_in_both_dialects() -> Result<()> {
    // D192: real scenes open with `vec4 iMouse = vec4(MouseXY*RENDERSIZE,
    // ...)`. GLSL ES rejects a global initializer that is not constant; the
    // assembler moves it into main, in order, so a later global can use an
    // earlier one. MouseXY 0.5 gives (0.5, 0.5), halved to (0.25, 0.25).
    let json = r#"{ "CONTROLS": [ {"NAME": "MouseXY", "TYPE": "xy", "DEFAULT": [0.5, 0.5]} ] }"#;
    let shader = "vec4 iMouse = vec4(MouseXY * RENDERSIZE, 0.0, 0.0);\n\
                  vec2 halved = iMouse.xy / RENDERSIZE * 0.5;\n\
                  vec4 renderMain() { return vec4(halved, 0.0, 1.0); }\n";
    for api in BOTH {
        for specialise_passes in [true, false] {
            let options = Options {
                specialise_passes,
                ..Options::default()
            };
            let Some(px) = render_center(api, options, json, shader, 1)? else {
                continue;
            };
            assert!(
                near(px[0], 64) && near(px[1], 64),
                "{api:?} specialised={specialise_passes}: 0.5 * 0.5 in both channels; got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_global_initialized_from_the_coordinates_sees_them_in_both_dialects() -> Result<()> {
    // D293: a global set from `_uv` was read before main set `_uv`, so on
    // desktop it was (0, 0) on every pixel and the scene rendered black.
    // Near the centre `_uv` is about (0.5, 0.5), so red is about 128 and
    // green about 64 (the 32x18 target's centre pixel is half a texel off
    // either way); broken, both are 0. `t0` is TIME, 0.
    let shader = "vec2 uv = _uv;\nvec2 half_uv = uv * 0.5;\nfloat t0 = TIME;\n\
                  vec4 renderMain() { return vec4(uv.x, half_uv.y, t0, 1.0); }\n";
    for api in BOTH {
        for specialise_passes in [true, false] {
            let options = Options {
                specialise_passes,
                ..Options::default()
            };
            let Some(px) = render_center(api, options, "{}", shader, 1)? else {
                continue;
            };
            assert!(
                (112..=144).contains(&px[0]) && (48..=80).contains(&px[1]),
                "{api:?} specialised={specialise_passes}: uv is the pixel's, not (0, 0); got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn the_output_is_opaque_and_buffer_passes_keep_their_alpha() -> Result<()> {
    // D192: Synesthesia's `hello_world_start` returns vec4(1, 0, 0, 0) and is
    // meant to be red. The final pass is made opaque; a buffer pass keeps
    // its alpha, which scenes use as data (here 0.25, read back as red).
    let json = r#"{ "PASSES": [ {"TARGET": "data"} ] }"#;
    let shader = "vec4 renderMain() {\n\
                  if (PASSINDEX == 0) { return vec4(0.0, 0.0, 0.0, 0.25); }\n\
                  return vec4(texture(data, _uv).a, 0.0, 0.0, 0.0); }\n";
    for api in BOTH {
        for specialise_passes in [true, false] {
            let options = Options {
                specialise_passes,
                ..Options::default()
            };
            let Some(px) = render_center(api, options, json, shader, 1)? else {
                continue;
            };
            assert!(
                near(px[0], 64) && px[3] == 255,
                "{api:?} specialised={specialise_passes}: buffer alpha 0.25 reaches the final pass, which is opaque; got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn int_passindex_comparisons_still_compile_in_both_dialects() -> Result<()> {
    // The fix-up must not disturb the int idiom (R4) or other uses.
    let json = r#"{ "PASSES": [ {"TARGET": "firstBuffer"} ] }"#;
    let shader = r#"
        vec4 renderMain(void) {
            if (PASSINDEX == 0) { return vec4(0.25, 0.0, 0.0, 1.0); }
            float p = float(PASSINDEX);
            return vec4(texture(firstBuffer, _uv).r * 2.0, p * 0.5, 0.0, 1.0);
        }"#;
    for api in BOTH {
        let Some(px) = render_center(api, Options::default(), json, shader, 1)? else {
            continue;
        };
        assert!(near(px[0], 128) && near(px[1], 128), "{api:?}: got {px:?}");
    }
    Ok(())
}

#[test]
fn a_compile_error_points_at_main_glsl() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let scene = folder("{}", "vec4 renderMain() {\n    return undefinedThing;\n}\n")?;
    let error = ScenePlayer::new(gl, &caps, &scene, [8, 8], Options::default())
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        error.contains("main.glsl:2"),
        "expected a main.glsl:2 location, got: {error}"
    );
    Ok(())
}

#[test]
fn a_compile_error_names_the_users_column_when_the_driver_gives_one() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    // The assembler writes `1` as `1.0` before the error, so the driver's
    // column is two bytes to the right of the user's.
    let line = "    float x = 1; return vec4(x) + undefinedThing;";
    let shader = format!("vec4 renderMain() {{\n{line}\n}}\n");
    let scene = folder("{}", &shader)?;
    let error = ScenePlayer::new(gl, &caps, &scene, [8, 8], Options::default())
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    let wanted = line.find("undefinedThing").map_or(0, |at| at + 1);
    // Mesa's `1:2(C)` gives a column; other drivers name the line only.
    if error.contains(" 1:2(") {
        assert!(
            error.contains(&format!("main.glsl:2:{wanted}: ")),
            "expected main.glsl:2:{wanted}, the column on the user's line, got: {error}"
        );
    } else {
        assert!(
            error.contains("main.glsl:2: "),
            "a driver without columns: the line alone, got: {error}"
        );
    }
    Ok(())
}

/// Reads both audio textures at the middle of the frame: spectrum in red,
/// level trail in green. Written for both dialects with one `#ifdef`,
/// because GLES has no `sampler1D`.
const AUDIO_PROBE: &str = r#"
    #ifdef GL_ES
    #define SPEC(x) texture(syn_Spectrum, vec2(x, 0.5))
    #define TRAIL(x) texture(syn_LevelTrail, vec2(x, 0.5))
    #else
    #define SPEC(x) texture(syn_Spectrum, x)
    #define TRAIL(x) texture(syn_LevelTrail, x)
    #endif
    vec4 renderMain() { return vec4(SPEC(0.5).r, TRAIL(0.5).g, SPEC(0.5).a, 1.0); }
"#;

#[test]
fn uploaded_audio_texels_reach_the_spectrum_and_trail_samplers() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder("{}", AUDIO_PROBE)?;
        let controls = ControlBank::new(scene.def.controls.clone())?;
        let audio = vec![0.0; SCALARS.len()];
        let mut player = ScenePlayer::new(gl, &caps, &scene, [32, 18], Options::default())?;
        // A width other than the 256 the player starts with, to cover the
        // reallocation path as well.
        for width in [256_usize, 64] {
            let spectrum = vec![[0.5, 0.0, 0.0, 1.0]; width];
            let trail = vec![[0.0, 0.25, 0.0, 0.0]; width];
            player.upload_audio(&AudioTextures {
                spectrum: &spectrum,
                level_trail: &trail,
                shadertoy: None,
            });
            player.render(&FrameInputs {
                time: 0.0,
                frame: 0,
                mouse: Mouse::default(),
                audio: &audio,
                controls: &controls,
                transitions: &[],
            });
            if let Some(code) = gl.error() {
                return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
            }
            let pixels = player.read_output();
            let offset = (9 * 32 + 16) * 4;
            let px = pixels
                .get(offset..offset + 3)
                .ok_or_else(|| anyhow!("readback too short"))?;
            assert!(
                near(px[0], 128) && near(px[1], 64) && near(px[2], 255),
                "{api:?} width {width}: spectrum r 0.5, trail g 0.25, wave a 1.0; got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_scene_that_ignores_the_audio_textures_never_uploads_them() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let scene = folder("{}", "vec4 renderMain() { return vec4(1.0); }")?;
    let mut player = ScenePlayer::new(gl, &caps, &scene, [32, 18], Options::default())?;
    // Nothing to assert on pixels: the point is that a mismatched width
    // (which would reallocate) is harmless and raises no GL error.
    player.upload_audio(&AudioTextures {
        spectrum: &[[1.0; 4]; 7],
        level_trail: &[[1.0; 4]; 7],
        shadertoy: None,
    });
    assert!(gl.error().is_none(), "an unused upload must be a no-op");
    Ok(())
}

#[test]
fn the_gpu_timer_reports_results_without_ever_waiting() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let Some(mut timer) = GpuTimer::new(gl, &caps) else {
            strobe_gfx::skip::note(format_args!("{api:?} has no GPU timer queries"));
            continue;
        };
        let scene = folder("{}", "vec4 renderMain() { return vec4(TIME); }")?;
        let controls = ControlBank::new(scene.def.controls.clone())?;
        let audio = vec![0.0; SCALARS.len()];
        let mut player = ScenePlayer::new(gl, &caps, &scene, [64, 36], Options::default())?;
        let mut samples = Vec::new();
        for frame in 0..12_u64 {
            timer.begin();
            player.render(&FrameInputs {
                time: frame as f32 / 60.0,
                frame,
                mouse: Mouse::default(),
                audio: &audio,
                controls: &controls,
                transitions: &[],
            });
            timer.end();
            timer.poll(|ms| samples.push(ms));
            assert!(
                timer.in_flight() <= strobe_gfx::timing::SLOTS,
                "{api:?}: never more than the ring holds"
            );
        }
        // Let the GPU finish, then every outstanding result is readable.
        gl.finish();
        timer.poll(|ms| samples.push(ms));
        assert!(
            !samples.is_empty(),
            "{api:?}: at least one measurement must come back"
        );
        assert!(
            samples.iter().all(|ms| ms.is_finite() && *ms >= 0.0),
            "{api:?}: samples are finite and non-negative: {samples:?}"
        );
        assert_eq!(timer.in_flight(), 0, "{api:?}: everything was drained");
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

/// Renders `frames` frames of `scene` and returns the centre pixel, with
/// the player available to `before` for per-frame setup.
fn render_folder(
    api: Api,
    options: Options,
    scene: &SceneFolder,
    frames: u64,
    mut before: impl FnMut(&mut ScenePlayer),
) -> Result<Option<[u8; 4]>> {
    let Some(headless) = context(api)? else {
        return Ok(None);
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let controls = ControlBank::new(scene.def.controls.clone())?;
    let audio = vec![0.0; SCALARS.len()];
    let mut player = ScenePlayer::new(gl, &caps, scene, [32, 18], options)?;
    for frame in 0..frames {
        before(&mut player);
        player.render(&FrameInputs {
            time: frame as f32 / 60.0,
            frame,
            mouse: Mouse::default(),
            audio: &audio,
            controls: &controls,
            transitions: &[],
        });
    }
    if let Some(code) = gl.error() {
        return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
    }
    let pixels = player.read_output();
    let offset = (9 * 32 + 16) * 4;
    let px = pixels
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow!("readback too short"))?;
    Ok(Some([px[0], px[1], px[2], px[3]]))
}

/// A scene whose passes each have their own source file.
fn per_pass_folder(json: &str, sources: &[&str]) -> Result<SceneFolder> {
    Ok(SceneFolder {
        dir: PathBuf::from("."),
        def: SceneDef::parse(json, "test")?,
        shader: sources.last().copied().unwrap_or_default().to_string(),
        pass_shaders: sources.iter().map(|s| (*s).to_string()).collect(),
        script: None,
    })
}

const TWO_PASSES: &str = r#"{ "PASSES": [ {"TARGET": "buf"} ],
    "PASS_SOURCES": ["passes/buf.glsl", "main.glsl"] }"#;

#[test]
fn pass_sources_give_each_pass_its_own_program_even_with_colliding_names() -> Result<()> {
    // Both files define `tint` and `K` differently. One shared program could
    // not hold both; asking for one anyway must fall back to per-pass.
    let scene = per_pass_folder(
        TWO_PASSES,
        &[
            "const float K = 0.25;\nvec3 tint() { return vec3(K, 0.0, 0.0); }\n\
             vec4 renderMain() { return vec4(tint(), 1.0); }",
            "const float K = 0.5;\nvec3 tint() { return vec3(0.0, K, 0.0); }\n\
             vec4 renderMain() { return vec4(texture(buf, _uv).rgb + tint(), 1.0); }",
        ],
    )?;
    for api in BOTH {
        for specialise_passes in [true, false] {
            let options = Options {
                specialise_passes,
                ..Options::default()
            };
            let Some(px) = render_folder(api, options, &scene, 1, |_| {})? else {
                continue;
            };
            assert!(
                near(px[0], 64) && near(px[1], 128) && px[2] == 0,
                "{api:?} specialise_passes={specialise_passes}: red from the buffer's K, \
                 green from the final pass's K; got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_compile_error_names_the_pass_file_and_its_own_line() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());

    let scene = per_pass_folder(
        TWO_PASSES,
        &[
            "vec4 renderMain() {\n    return vec4(1.0);\n}\n",
            "// line 1\n// line 2\nvec4 renderMain() {\n    return missingThing;\n}\n",
        ],
    )?;
    let error = ScenePlayer::new(gl, &caps, &scene, [8, 8], Options::default())
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    // The scene's PASS_SOURCES entry for the final pass is main.glsl, and
    // the error is on that file's line 4.
    assert!(
        error.contains("pass 1") && error.contains("main.glsl:4"),
        "expected pass 1, main.glsl:4, got: {error}"
    );

    let scene = per_pass_folder(
        TWO_PASSES,
        &[
            "vec4 renderMain() {\n    return nope;\n}\n",
            "vec4 renderMain() { return vec4(1.0); }",
        ],
    )?;
    let error = ScenePlayer::new(gl, &caps, &scene, [8, 8], Options::default())
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        error.contains("pass 0") && error.contains("passes/buf.glsl:2"),
        "expected pass 0, passes/buf.glsl:2, got: {error}"
    );
    Ok(())
}

#[test]
fn strobe_date_is_the_date_the_host_set() -> Result<()> {
    let shader = "vec4 renderMain() { return vec4(strobe_Date.x / 4096.0, strobe_Date.y / 16.0, \
                  strobe_Date.z / 32.0, 1.0); }";
    for api in BOTH {
        let scene = folder("{}", shader)?;
        let Some(px) = render_folder(api, Options::default(), &scene, 1, |player| {
            player.set_date([2048.0, 8.0, 16.0, 0.0]);
        })?
        else {
            continue;
        };
        assert!(
            near(px[0], 128) && near(px[1], 128) && near(px[2], 128),
            "{api:?}: year 2048/4096, month 8/16, day 16/32 should all be half; got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn a_full_precision_pass_keeps_counting_where_half_floats_stall() -> Result<()> {
    // 0.25 a frame for 2400 frames reaches 600. A 16-bit float cannot hold
    // 600.25 (its spacing there is 0.5), so it would stall near 512.
    let shader = "vec4 renderMain() {\n\
        if (PASSINDEX == 0) { return texture(acc, vec2(0.5)) + vec4(0.25, 0.0, 0.0, 0.0); }\n\
        return vec4(texture(acc, vec2(0.5)).r / 1024.0, 0.0, 0.0, 1.0); }";
    for (full, expected) in [(true, 149_u8), (false, 128)] {
        let json = format!(
            r#"{{ "PASSES": [ {{"TARGET": "acc", "DOUBLE_BUFFER": true, "WIDTH": 1,
                 "HEIGHT": 1, "FILTER": "nearest", "FULL_PRECISION": {full}}} ] }}"#
        );
        let scene = folder(&json, shader)?;
        let Some(px) = render_folder(Api::DesktopCore33, Options::default(), &scene, 2400, |_| {})?
        else {
            return Ok(());
        };
        // 600 / 1024 is 149.4 of 255; stalled at 512 it is 127.5.
        assert!(
            px[0].abs_diff(expected) <= 2,
            "full_precision={full}: expected about {expected}, got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn a_fixed_size_pass_keeps_its_grid_when_the_output_resizes() -> Result<()> {
    // The pass writes its own RENDERSIZE; the final pass shows it. Design
    // 64x36 with a 16x8 pass: followed in proportion (D11) it is 12x6 at the
    // 48x27 the player is resized to, and FIXED_SIZE keeps it 16x8 (D191).
    let shader = "vec4 renderMain() {\n\
        if (PASSINDEX == 0) { return vec4(RENDERSIZE / 255.0, 0.0, 1.0); }\n\
        return vec4(texture(grid, vec2(0.5)).rg, 0.0, 1.0); }";
    for api in BOTH {
        for (fixed, expected) in [(true, [16_u8, 8]), (false, [12, 6])] {
            let json = format!(
                r#"{{ "WIDTH": 64, "HEIGHT": 36, "PASSES": [ {{"TARGET": "grid",
                     "WIDTH": 16, "HEIGHT": 8, "FILTER": "nearest", "FIXED_SIZE": {fixed}}} ] }}"#
            );
            let scene = folder(&json, shader)?;
            let Some(px) = render_folder(api, Options::default(), &scene, 3, |player| {
                if let Err(error) = player.resize_keeping([48, 27]) {
                    eprintln!("resize failed: {error}");
                }
            })?
            else {
                return Ok(());
            };
            assert_eq!(
                [px[0], px[1]],
                expected,
                "{api:?} FIXED_SIZE={fixed}: the pass's RENDERSIZE"
            );
        }
    }
    Ok(())
}

/// A scene that is original to this repository and shaped like the ones
/// D280 is about: a root `WIDTH`/`HEIGHT`, a FLOAT nearest-filtered buffer
/// pass with the same literal size (absolute pixels, D194), and a final pass
/// that reads it with `texelFetch(grad, ivec2(_xy), 0)`. The buffer holds
/// `_xy / (1920, 1080)`, so the picture is a gradient from 0 to 1 only if the
/// final pass addresses the whole buffer.
const PINNED_GRADIENT_JSON: &str = r#"{ "WIDTH": 1920, "HEIGHT": 1080,
    "PASSES": [ {"TARGET": "grad", "WIDTH": 1920, "HEIGHT": 1080, "FLOAT": true,
                 "FILTER": "nearest", "WRAP": "clamp"} ] }"#;

const PINNED_GRADIENT_SHADER: &str = "vec4 renderMain() {\n\
    if (PASSINDEX == 0) { return vec4(_xy / vec2(1920.0, 1080.0), 0.0, 1.0); }\n\
    return vec4(texelFetch(grad, ivec2(_xy), 0).rg, 0.0, 1.0); }";

/// Checks the output of the gradient scene covers the whole gradient: the
/// pixel at fraction `(fx, fy)` across and up the picture holds roughly
/// `(fx, fy)`.
fn assert_covers_the_gradient(player: &ScenePlayer, size: [i32; 2], label: &str) -> Result<()> {
    let [width, height] = player.render_size();
    assert_eq!(
        [width, height],
        size,
        "{label}: the final pass renders at {size:?}"
    );
    let pixels = player.read_output();
    for (fx, fy) in [
        (0.01, 0.01),
        (0.99, 0.01),
        (0.01, 0.99),
        (0.99, 0.99),
        (0.5, 0.5),
        (0.25, 0.75),
    ] {
        let x = ((width as f32 * fx) as usize).min(width as usize - 1);
        // The readback is top row first, the gradient grows upward.
        let y = ((height as f32 * (1.0 - fy)) as usize).min(height as usize - 1);
        let offset = (y * width as usize + x) * 4;
        let px = pixels
            .get(offset..offset + 4)
            .ok_or_else(|| anyhow!("{label}: readback too short"))?;
        let expect = |fraction: f32| (fraction * 255.0).round() as i32;
        assert!(
            (i32::from(px[0]) - expect(fx)).abs() <= 3
                && (i32::from(px[1]) - expect(fy)).abs() <= 3,
            "{label}: at ({fx}, {fy}) of the picture expected about ({}, {}), got {px:?}; \
             a final pass smaller than the buffer would show only its corner",
            expect(fx),
            expect(fy)
        );
    }
    Ok(())
}

#[test]
fn a_final_pass_reading_a_root_sized_literal_pass_covers_it_at_any_render_scale() -> Result<()> {
    // D280. The window is 960x540; render scale 1.0 asks for that, 0.5 for
    // 480x270, and a resize between them must not change the picture.
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder(PINNED_GRADIENT_JSON, PINNED_GRADIENT_SHADER)?;
        assert_eq!(
            strobe_scene::pin::pinned_size(&scene),
            Some([1920, 1080]),
            "{api:?}: the scene pins its final pass"
        );
        let controls = ControlBank::new(scene.def.controls.clone())?;
        let audio = vec![0.0; SCALARS.len()];
        let mut player = ScenePlayer::new(gl, &caps, &scene, [960, 540], Options::default())?;
        assert_eq!(
            player.requested_size(),
            [960, 540],
            "{api:?}: the request is remembered"
        );
        let mut frame = 0_u64;
        let mut draw = |player: &mut ScenePlayer| {
            player.render(&FrameInputs {
                time: frame as f32 / 60.0,
                frame,
                mouse: Mouse::default(),
                audio: &audio,
                controls: &controls,
                transitions: &[],
            });
            frame += 1;
        };
        draw(&mut player);
        draw(&mut player);
        assert_covers_the_gradient(
            &player,
            [1920, 1080],
            &format!("{api:?} at render scale 1.0"),
        )?;

        player.resize([480, 270])?;
        assert_eq!(
            player.requested_size(),
            [480, 270],
            "{api:?}: the new request is remembered"
        );
        draw(&mut player);
        draw(&mut player);
        assert_covers_the_gradient(
            &player,
            [1920, 1080],
            &format!("{api:?} at render scale 0.5"),
        )?;

        player.resize_keeping([240, 135])?;
        draw(&mut player);
        assert_covers_the_gradient(
            &player,
            [1920, 1080],
            &format!("{api:?} at render scale 0.25"),
        )?;
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

/// The gradient scene's final pass sampling the buffer by uv instead, as
/// most root-sized-pass scenes do: it reads the whole buffer at any size.
const UV_GRADIENT_SHADER: &str = "vec4 renderMain() {\n\
    if (PASSINDEX == 0) { return vec4(_xy / vec2(1920.0, 1080.0), 0.0, 1.0); }\n\
    return vec4(texture(grad, _uv).rg, 0.0, 1.0); }";

#[test]
fn a_scene_without_a_texel_read_root_sized_literal_pass_still_follows_the_render_scale()
-> Result<()> {
    // The other side of D280: the pin is for scenes whose passes demand it.
    // A 1920x1080 literal pass in a scene that never wrote a root size, a
    // root-sized one that is an expression, or a root-sized literal one the
    // final pass samples by uv, renders at the size it is asked.
    for (json, shader) in [
        (
            r#"{ "PASSES": [ {"TARGET": "grad", "WIDTH": 1920, "HEIGHT": 1080} ] }"#,
            PINNED_GRADIENT_SHADER,
        ),
        (
            r#"{ "WIDTH": 1920, "HEIGHT": 1080,
                 "PASSES": [ {"TARGET": "grad", "WIDTH": "$WIDTH", "HEIGHT": "$HEIGHT"} ] }"#,
            PINNED_GRADIENT_SHADER,
        ),
        (PINNED_GRADIENT_JSON, UV_GRADIENT_SHADER),
    ] {
        for api in BOTH {
            let Some(headless) = context(api)? else {
                continue;
            };
            let gl = headless.gl();
            let caps = Caps::from_info(gl.info());
            let scene = folder(json, shader)?;
            let mut player = ScenePlayer::new(gl, &caps, &scene, [64, 36], Options::default())?;
            assert_eq!(player.render_size(), [64, 36], "{api:?}: as asked");
            player.resize([32, 18])?;
            assert_eq!(player.render_size(), [32, 18], "{api:?}: follows a resize");
            assert_eq!(player.pinned_size(), None, "{api:?}: nothing pinned");
            if shader == UV_GRADIENT_SHADER {
                // Sampled by uv, the buffer shows whole at a smaller size.
                player.resize([480, 270])?;
                let controls = ControlBank::new(scene.def.controls.clone())?;
                let audio = vec![0.0; SCALARS.len()];
                for frame in 0..2_u64 {
                    player.render(&FrameInputs {
                        time: frame as f32 / 60.0,
                        frame,
                        mouse: Mouse::default(),
                        audio: &audio,
                        controls: &controls,
                        transitions: &[],
                    });
                }
                assert_covers_the_gradient(&player, [480, 270], &format!("{api:?} by uv"))?;
            }
        }
    }
    Ok(())
}

#[test]
fn image_wrap_filter_and_vflip_options_are_honoured() -> Result<()> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("image-options");
    std::fs::create_dir_all(&dir)?;
    // One column, two rows: red on top, blue underneath.
    let mut picture = image::RgbaImage::new(1, 2);
    picture.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
    picture.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
    picture.save(dir.join("two.png"))?;
    let shader = "vec4 renderMain() { return vec4(texture(pic, vec2(0.5, 0.9)).rgb, 1.0); }";
    for (vflip, expected) in [(true, [255_u8, 0]), (false, [0, 255])] {
        let json = format!(
            r#"{{ "IMAGES": [ {{"NAME": "pic", "PATH": "two.png", "WRAP": "clamp",
                 "FILTER": "nearest", "VFLIP": {vflip}}} ] }}"#
        );
        let mut scene = folder(&json, shader)?;
        scene.dir = dir.clone();
        let Some(px) = render_folder(Api::DesktopCore33, Options::default(), &scene, 1, |_| {})?
        else {
            return Ok(());
        };
        assert!(
            px[0] == expected[0] && px[2] == expected[1],
            "vflip={vflip}: near the top of uv the picture's top row is red when flipped \
             upright and blue when not; got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn the_shadertoy_audio_texture_is_512_by_2_and_resampled_from_the_spectrum() -> Result<()> {
    // Row 0 (v = 0.25) is the spectrum, row 1 (v = 0.75) the waveform;
    // blue reports the texture's width through textureSize.
    let shader = "vec4 renderMain() {\n\
        vec4 spec = texture(strobe_ShadertoyAudio, vec2(0.5, 0.25));\n\
        vec4 wave = texture(strobe_ShadertoyAudio, vec2(0.5, 0.75));\n\
        float wide = float(textureSize(strobe_ShadertoyAudio, 0).x) / 512.0;\n\
        return vec4(spec.r, wave.r, wide * 0.5, 1.0); }";
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder("{}", shader)?;
        let controls = ControlBank::new(scene.def.controls.clone())?;
        let audio = vec![0.0; SCALARS.len()];
        let mut player = ScenePlayer::new(gl, &caps, &scene, [32, 18], Options::default())?;
        let centre = |player: &mut ScenePlayer| -> Result<[u8; 3]> {
            player.render(&FrameInputs {
                time: 0.0,
                frame: 0,
                mouse: Mouse::default(),
                audio: &audio,
                controls: &controls,
                transitions: &[],
            });
            let pixels = player.read_output();
            let offset = (9 * 32 + 16) * 4;
            let px = pixels
                .get(offset..offset + 3)
                .ok_or_else(|| anyhow!("readback too short"))?;
            Ok([px[0], px[1], px[2]])
        };
        // Before any upload: a dark spectrum and a centred waveform.
        let silent = centre(&mut player)?;
        assert!(
            silent[0] == 0 && near(silent[1], 128) && near(silent[2], 128),
            "{api:?}: silence is zero spectrum, waveform at 0.5, 512 wide; got {silent:?}"
        );
        // 256 texels whose raw spectrum is 0.75 and whose waveform is 0.25.
        let texels = vec![[0.75, 0.0, 0.0, 0.25]; 256];
        player.upload_audio(&AudioTextures {
            spectrum: &texels,
            level_trail: &texels,
            shadertoy: None,
        });
        let live = centre(&mut player)?;
        // (0.75 * 80 - 6.0206) / 70 = 0.771 -> 197 of 255.
        assert!(
            near(live[0], 197) && near(live[1], 64) && near(live[2], 128),
            "{api:?}: resampled spectrum 197, waveform 64; got {live:?}"
        );
        // Browser-matched bytes are uploaded as given, not resampled: row 0
        // all 40, row 1 all 200.
        let exact: Vec<u8> = [vec![40_u8; 512], vec![200_u8; 512]].concat();
        player.upload_audio(&AudioTextures {
            spectrum: &texels,
            level_trail: &texels,
            shadertoy: Some(&exact),
        });
        let direct = centre(&mut player)?;
        assert!(
            near(direct[0], 40) && near(direct[1], 200) && near(direct[2], 128),
            "{api:?}: exact bytes pass through untouched; got {direct:?}"
        );
        // A wrong-sized slice is ignored and the resampled path is used.
        player.upload_audio(&AudioTextures {
            spectrum: &texels,
            level_trail: &texels,
            shadertoy: Some(&exact[..100]),
        });
        let fallback = centre(&mut player)?;
        assert!(
            near(fallback[0], 197) && near(fallback[1], 64),
            "{api:?}: a wrong-sized slice falls back to resampling; got {fallback:?}"
        );
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

#[test]
fn a_healthy_context_reports_no_reset_and_a_lost_one_is_never_asked() -> Result<()> {
    for api in [Api::DesktopCore33, Api::Gles30] {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        assert!(!gl.is_lost(), "{api:?}: a new context is not lost");
        let status = gl.reset_status();
        assert!(
            !status.is_reset(),
            "{api:?}: a fresh context has not been reset, got {status}"
        );
        let clone = gl.clone();
        gl.mark_lost();
        assert!(clone.is_lost(), "{api:?}: the flag is shared by clones");
        assert_eq!(
            clone.reset_status(),
            strobe_gfx::gl::ResetStatus::Unknown,
            "{api:?}: a lost context is reported without calling the driver"
        );
        // Dropping an object on a lost context skips the delete (a smoke
        // test: the skip itself cannot be observed from outside, but the
        // drop must be harmless and leave no GL error behind).
        let texture = gl
            .texture_2d(
                4,
                4,
                strobe_gfx::gl::Format::Rgba8,
                strobe_gfx::gl::Sampling::LINEAR_CLAMP,
                None,
            )
            .map_err(|e| anyhow!("{api:?}: texture: {e}"))?;
        drop(texture);
        let error = gl.error();
        assert!(error.is_none(), "{api:?}: unexpected GL error {error:?}");
    }
    Ok(())
}

// ---- textureFinalPass: the asynchronous readback ---------------------------------

/// One 64x36 pixel of the readback, or `None` when it has not arrived.
fn readback_pixel(player: &ScenePlayer, x: usize, y: usize) -> Option<[u8; 3]> {
    let pixels = player.final_pass_pixels()?;
    let offset = (y * 64 + x) * 3;
    let px = pixels.get(offset..offset + 3)?;
    Some([px[0], px[1], px[2]])
}

/// Renders frame `frame` of `player` with a silent room.
fn render_frame(player: &mut ScenePlayer, controls: &ControlBank, frame: u64) {
    let audio = vec![0.0; SCALARS.len()];
    player.render(&FrameInputs {
        time: frame as f32 / 60.0,
        frame,
        mouse: Mouse::default(),
        audio: &audio,
        controls,
        transitions: &[],
    });
}

#[test]
fn the_final_pass_readback_arrives_two_frames_late_and_is_black_before() -> Result<()> {
    // Red encodes the frame number, so the age of what arrives is readable.
    let shader = "vec4 renderMain() { return vec4(FRAMECOUNT / 32.0, 0.25, 0.5, 1.0); }";
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder("{}", shader)?;
        let controls = ControlBank::new(Vec::new())?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [320, 180], Options::default())?;
        assert!(
            player.final_pass_pixels().is_none(),
            "{api:?}: off by default, nothing to read"
        );
        player.enable_final_pass_readback()?;
        for frame in 0..10_u64 {
            render_frame(&mut player, &controls, frame);
            // The test waits for the GPU so every fence has signalled: it
            // pins the *documented* latency. The player itself never does.
            gl.finish();
            let latest = readback_pixel(&player, 5, 5);
            if frame < 2 {
                assert!(
                    latest.is_none(),
                    "{api:?}: frame {frame}: the ring has not filled, so the script reads black"
                );
                continue;
            }
            let px = latest.ok_or_else(|| anyhow!("{api:?}: frame {frame}: nothing arrived"))?;
            let expected = ((frame - 2) as f32 / 32.0 * 255.0).round() as u8;
            assert!(
                near(px[0], expected) && near(px[1], 64) && near(px[2], 128),
                "{api:?}: after frame {frame} the readback shows frame {}: want red {expected}, got {px:?}",
                frame - 2
            );
        }
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
        assert_eq!(
            player.final_pass_dropped(),
            0,
            "{api:?}: nothing was dropped"
        );
    }
    Ok(())
}

#[test]
fn the_downsample_keeps_the_pictures_orientation_and_average() -> Result<()> {
    // Red rises up the picture, green rises to the right.
    let shader = "vec4 renderMain() { return vec4(_uv.y, _uv.x, 0.0, 1.0); }";
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder("{}", shader)?;
        let controls = ControlBank::new(Vec::new())?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [640, 360], Options::default())?;
        player.enable_final_pass_readback()?;
        for frame in 0..4_u64 {
            render_frame(&mut player, &controls, frame);
            gl.finish();
        }
        let pixel = |x, y| readback_pixel(&player, x, y).ok_or_else(|| anyhow!("{api:?}: no data"));
        let (bottom_left, top_right, middle) = (pixel(0, 0)?, pixel(63, 35)?, pixel(32, 18)?);
        // Row 0 is the bottom (rows run bottom-up, like _uv), and each texel
        // is the average of its block, so it sits at (index + 0.5) / size.
        let within =
            |actual: u8, expected: f32| (f32::from(actual) - expected * 255.0).abs() <= 5.0;
        assert!(
            within(bottom_left[0], 0.5 / 36.0) && within(bottom_left[1], 0.5 / 64.0),
            "{api:?}: bottom-left is dark in both channels; got {bottom_left:?}"
        );
        assert!(
            within(top_right[0], 35.5 / 36.0) && within(top_right[1], 63.5 / 64.0),
            "{api:?}: top-right is bright in both channels; got {top_right:?}"
        );
        assert!(
            within(middle[0], 18.5 / 36.0) && within(middle[1], 32.5 / 64.0),
            "{api:?}: the middle is the mean; got {middle:?}"
        );
    }
    Ok(())
}

#[test]
fn a_busy_gpu_costs_freshness_never_a_stall_and_the_ring_recovers() -> Result<()> {
    let shader = "vec4 renderMain() { return vec4(FRAMECOUNT / 64.0, 0.0, 0.0, 1.0); }";
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder("{}", shader)?;
        let controls = ControlBank::new(Vec::new())?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [64, 36], Options::default())?;
        player.enable_final_pass_readback()?;
        // No gl.finish(): fences signal whenever the driver gets there. The
        // picture may be older than two frames but never newer, never
        // garbage, and it never goes backwards.
        let mut newest_seen = 0_i32;
        for frame in 0..40_u64 {
            render_frame(&mut player, &controls, frame);
            if let Some(px) = readback_pixel(&player, 0, 0) {
                let shown = (f32::from(px[0]) / 255.0 * 64.0).round() as i32;
                assert!(
                    shown <= frame as i32 - 2,
                    "{api:?}: frame {frame} shows frame {shown}, newer than the ring allows"
                );
                assert!(
                    shown >= newest_seen,
                    "{api:?}: the picture went backwards ({shown} after {newest_seen})"
                );
                newest_seen = shown;
            }
        }
        gl.finish();
        for frame in 40..44_u64 {
            render_frame(&mut player, &controls, frame);
            gl.finish();
        }
        let px = readback_pixel(&player, 0, 0).ok_or_else(|| anyhow!("{api:?}: never arrived"))?;
        let shown = (f32::from(px[0]) / 255.0 * 64.0).round() as i32;
        assert_eq!(
            shown, 41,
            "{api:?}: once the GPU catches up it is two frames old again"
        );
    }
    Ok(())
}

#[test]
fn resizing_restarts_the_readback_from_black() -> Result<()> {
    let shader = "vec4 renderMain() { return vec4(0.5, 0.5, 0.5, 1.0); }";
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder("{}", shader)?;
        let controls = ControlBank::new(Vec::new())?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [128, 72], Options::default())?;
        player.enable_final_pass_readback()?;
        for frame in 0..4_u64 {
            render_frame(&mut player, &controls, frame);
            gl.finish();
        }
        assert!(
            readback_pixel(&player, 1, 1).is_some(),
            "{api:?}: data before resize"
        );
        player.resize([200, 100])?;
        assert!(
            player.final_pass_pixels().is_none(),
            "{api:?}: a resize drops the old picture"
        );
        for frame in 4..8_u64 {
            render_frame(&mut player, &controls, frame);
            gl.finish();
        }
        let px = readback_pixel(&player, 1, 1)
            .ok_or_else(|| anyhow!("{api:?}: no data after resize"))?;
        assert!(
            near(px[0], 128) && near(px[1], 128),
            "{api:?}: the new size reads back too; got {px:?}"
        );
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

#[test]
fn a_lost_context_makes_the_readback_inert_and_its_drops_harmless() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder("{}", "vec4 renderMain() { return vec4(1.0); }")?;
        let controls = ControlBank::new(Vec::new())?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [64, 36], Options::default())?;
        player.enable_final_pass_readback()?;
        render_frame(&mut player, &controls, 0);
        gl.mark_lost();
        // With the flag set, no GL call may be made for the ring's buffers
        // and fences: dropping the player frees nothing and touches no GL.
        drop(player);
    }
    Ok(())
}

/// A frame counter that must survive resizes (D287): a 1x1 double-buffered
/// float pass that adds 1 per frame, a `$WIDTH` pass that records its own
/// width, and a final pass that shows both. Red is the count over 50 and
/// green is the `$WIDTH` pass's width over 255. The counter is a literal
/// size (absolute, D194), so no resize changes its size.
const COUNTER_JSON: &str = r#"{ "WIDTH": 64, "HEIGHT": 36,
    "PASSES": [
        {"TARGET": "count", "DOUBLE_BUFFER": true, "WIDTH": 1, "HEIGHT": 1,
         "FLOAT": true, "FULL_PRECISION": true, "FILTER": "nearest"},
        {"TARGET": "wide", "WIDTH": "$WIDTH", "HEIGHT": "$HEIGHT", "FILTER": "nearest"} ] }"#;

const COUNTER_SHADER: &str = "vec4 renderMain() {\n\
    if (PASSINDEX == 0) { return texture(count, vec2(0.5)) + vec4(1.0, 0.0, 0.0, 0.0); }\n\
    if (PASSINDEX == 1) { return vec4(RENDERSIZE.x / 255.0, 0.0, 0.0, 1.0); }\n\
    return vec4(texture(count, vec2(0.5)).r / 50.0, texture(wide, vec2(0.5)).r, 0.0, 1.0); }";

/// The centre pixel of the player's output.
fn centre_pixel(player: &ScenePlayer) -> Result<[u8; 4]> {
    let [width, height] = player.render_size();
    let pixels = player.read_output();
    let offset = ((height as usize / 2) * width as usize + width as usize / 2) * 4;
    let px = pixels
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow!("readback too short"))?;
    Ok([px[0], px[1], px[2], px[3]])
}

/// What the counter scene shows after `count` frames: the count scaled to
/// 8 bits, as the final pass writes it.
fn counter_byte(count: u32) -> u8 {
    (count as f32 / 50.0 * 255.0).round() as u8
}

#[test]
fn a_resize_keeps_the_passes_whose_size_it_does_not_change() -> Result<()> {
    for api in BOTH {
        for keeping in [false, true] {
            let Some(headless) = context(api)? else {
                continue;
            };
            let gl = headless.gl();
            let caps = Caps::from_info(gl.info());
            let scene = folder(COUNTER_JSON, COUNTER_SHADER)?;
            let controls = ControlBank::new(scene.def.controls.clone())?;
            let label = format!("{api:?} keeping={keeping}");
            let mut player = ScenePlayer::new(gl, &caps, &scene, [64, 36], Options::default())?;
            // An odd count leaves the second of the two buffers current, so
            // a resize that reset which one is current shows too.
            let mut frame = 0_u64;
            let mut counted = 0_u32;
            for _ in 0..5 {
                render_frame(&mut player, &controls, frame);
                frame += 1;
                counted += 1;
            }
            let px = centre_pixel(&player)?;
            assert!(
                px[0].abs_diff(counter_byte(counted)) <= 1 && px[1] == 64,
                "{label}: before any resize expected count {counted} and width 64, got {px:?}"
            );
            // Resize down, up and down again, as the adaptive probe does,
            // with a frame between each.
            for (size, width) in [([32, 18], 32_u8), ([64, 36], 64), ([48, 27], 48)] {
                if keeping {
                    player.resize_keeping(size)?;
                } else {
                    player.resize(size)?;
                }
                render_frame(&mut player, &controls, frame);
                frame += 1;
                counted += 1;
                let px = centre_pixel(&player)?;
                assert_eq!(player.render_size(), size, "{label}: resized to {size:?}");
                assert!(
                    px[0].abs_diff(counter_byte(counted)) <= 1,
                    "{label}: after resizing to {size:?} the counter should read {counted} \
                     (byte {}), not restart; got {px:?}",
                    counter_byte(counted)
                );
                assert_eq!(
                    px[1], width,
                    "{label}: the $WIDTH pass follows the resize to {size:?}; got {px:?}"
                );
            }
            // Asking for the size it already has changes nothing either.
            player.resize([48, 27])?;
            render_frame(&mut player, &controls, frame);
            counted += 1;
            let px = centre_pixel(&player)?;
            assert!(
                px[0].abs_diff(counter_byte(counted)) <= 1,
                "{label}: a no-op resize keeps counting; got {px:?}"
            );
            if let Some(code) = gl.error() {
                return Err(anyhow!("{label}: GL error 0x{code:x}"));
            }
        }
    }
    Ok(())
}

/// The counter of [`COUNTER_JSON`], read this time by the `$WIDTH` pass,
/// which the final pass shows: red is the count over 50.
const READ_COUNTER_SHADER: &str = "vec4 renderMain() {\n\
    if (PASSINDEX == 0) { return texture(count, vec2(0.5)) + vec4(1.0, 0.0, 0.0, 0.0); }\n\
    if (PASSINDEX == 1) { return vec4(texture(count, vec2(0.5)).r / 50.0, 0.0, 0.0, 1.0); }\n\
    return vec4(texture(wide, vec2(0.5)).rgb, 1.0); }";

#[test]
fn a_resize_restarts_an_unchanged_pass_that_a_restarted_pass_reads() -> Result<()> {
    // The `$WIDTH` pass restarts from black and reads the counter, so the
    // counter restarts with it (D287), as a Shadertoy buffer's `iFrame`
    // clock must (D91). `resize_keeping` restarts nothing, so it keeps it.
    for api in BOTH {
        for keeping in [false, true] {
            let Some(headless) = context(api)? else {
                continue;
            };
            let gl = headless.gl();
            let caps = Caps::from_info(gl.info());
            let scene = folder(COUNTER_JSON, READ_COUNTER_SHADER)?;
            let controls = ControlBank::new(scene.def.controls.clone())?;
            let label = format!("{api:?} keeping={keeping}");
            let mut player = ScenePlayer::new(gl, &caps, &scene, [64, 36], Options::default())?;
            for frame in 0..5 {
                render_frame(&mut player, &controls, frame);
            }
            if keeping {
                player.resize_keeping([32, 18])?;
            } else {
                player.resize([32, 18])?;
            }
            render_frame(&mut player, &controls, 5);
            let expected = counter_byte(if keeping { 6 } else { 1 });
            let px = centre_pixel(&player)?;
            assert!(
                px[0].abs_diff(expected) <= 1,
                "{label}: the counter the $WIDTH pass reads should show byte {expected}; got {px:?}"
            );
            if let Some(code) = gl.error() {
                return Err(anyhow!("{label}: GL error 0x{code:x}"));
            }
        }
    }
    Ok(())
}

#[test]
fn restart_clears_every_pass_a_resize_would_keep() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder(COUNTER_JSON, COUNTER_SHADER)?;
        let controls = ControlBank::new(scene.def.controls.clone())?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [64, 36], Options::default())?;
        for frame in 0..5 {
            render_frame(&mut player, &controls, frame);
        }
        player.restart()?;
        render_frame(&mut player, &controls, 0);
        let px = centre_pixel(&player)?;
        assert!(
            px[0].abs_diff(counter_byte(1)) <= 1 && px[1] == 64,
            "{api:?}: after a restart the literal-size counter starts again at 1; got {px:?}"
        );
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

#[test]
fn a_one_byte_texture_whose_rows_are_not_four_bytes_uploads_tightly_packed() -> Result<()> {
    // `texture_2d` and `upload` take tightly packed rows. GL's default
    // unpack alignment is 4, so without setting it a 3-texel R8 row is read
    // as 4 bytes: the second row starts one byte late and its last texel
    // comes from past the end of the slice. The player's own silent 1x2
    // Shadertoy audio texture is such a case.
    use strobe_gfx::gl::{Format, Sampling, Texture};
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let red = |texture: &Texture| -> Result<Vec<u8>> {
            let fbo = gl
                .framebuffer(texture)
                .map_err(|e| anyhow!("{api:?}: framebuffer: {e}"))?;
            let pixels = gl.read_rgba8(&fbo, 3, 2);
            Ok(pixels.as_chunks::<4>().0.iter().map(|px| px[0]).collect())
        };
        let texture = gl
            .texture_2d(
                3,
                2,
                Format::R8,
                Sampling::LINEAR_CLAMP,
                Some(&[10, 20, 30, 40, 50, 60]),
            )
            .map_err(|e| anyhow!("{api:?}: texture: {e}"))?;
        let allocated = red(&texture)?;
        assert_eq!(
            allocated,
            vec![10, 20, 30, 40, 50, 60],
            "{api:?}: the allocation reads its rows tightly packed"
        );
        gl.upload(&texture, &[70, 80, 90, 100, 110, 120])
            .map_err(|e| anyhow!("{api:?}: upload: {e}"))?;
        let uploaded = red(&texture)?;
        assert_eq!(
            uploaded,
            vec![70, 80, 90, 100, 110, 120],
            "{api:?}: an upload reads its rows tightly packed"
        );
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

/// D296: offscreen rendering never depends on the window's drawable.
///
/// On AMD's native WGL a hidden window's context rendered black above about
/// 160x90. A driver can only behave differently from llvmpipe here if the
/// renderer lets the default framebuffer (0, the window's drawable) into the
/// picture, so the invariant is that it does not: from `ScenePlayer::new`
/// through every frame and the read-back, no clear, draw or blit lands in
/// framebuffer 0 and no read-back comes from it, and each draw's viewport is
/// its own target's size, including a literal pass larger than the output
/// (the shape of the `neon_city` scene). Presenting to a window is the one
/// operation that does touch the default framebuffer; the last assertion
/// checks the trace sees it, so the invariant is not vacuously true.
#[test]
fn offscreen_rendering_never_touches_the_default_framebuffer() -> Result<()> {
    const OUTPUT: [i32; 2] = [64, 36];
    const BIG: [i32; 2] = [128, 72];
    let json = r#"{ "PASSES": [
        {"TARGET": "big", "WIDTH": 128, "HEIGHT": 72},
        {"TARGET": "trail", "DOUBLE_BUFFER": true}
    ] }"#;
    let shader = "vec4 renderMain() {\n\
                      if (PASSINDEX == 0) { return vec4(0.0, 1.0, 0.0, 1.0); }\n\
                      if (PASSINDEX == 1) { return texture(trail, _uv) * 0.5 + vec4(0.25, 0.0, 0.0, 0.0); }\n\
                      return texture(big, _uv) + texture(trail, _uv);\n\
                  }\n";
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        gl.trace_capture();
        let caps = Caps::from_info(gl.info());
        let scene = folder(json, shader)?;
        let controls = ControlBank::new(scene.def.controls.clone())?;
        let audio = vec![0.0; SCALARS.len()];
        let mut player = ScenePlayer::new(gl, &caps, &scene, OUTPUT, Options::default())?;
        for frame in 0..3_u64 {
            player.render(&FrameInputs {
                time: frame as f32 / 60.0,
                frame,
                mouse: Mouse::default(),
                audio: &audio,
                controls: &controls,
                transitions: &[],
            });
        }
        let pixels = player.read_output();
        assert_eq!(
            pixels.len(),
            (OUTPUT[0] * OUTPUT[1] * 4) as usize,
            "{api:?}: the read-back is the requested size"
        );
        let events = gl.trace_take();

        let strays: Vec<String> = events
            .iter()
            .filter(|event| event.touches_default_framebuffer())
            .map(ToString::to_string)
            .collect();
        assert!(
            strays.is_empty(),
            "{api:?}: offscreen work touched the default framebuffer: {strays:?}"
        );

        let draws: Vec<[i32; 2]> = events
            .iter()
            .filter_map(|event| match event {
                Event::Draw { viewport, .. } => Some(*viewport),
                Event::Bind { .. }
                | Event::Viewport { .. }
                | Event::Clear { .. }
                | Event::Blit { .. }
                | Event::ReadPixels { .. }
                | Event::Texture { .. } => None,
            })
            .collect();
        let expected: Vec<[i32; 2]> = (0..3).flat_map(|_| [BIG, OUTPUT, OUTPUT]).collect();
        assert_eq!(
            draws, expected,
            "{api:?}: three frames of three passes, each at its own target's size"
        );

        let reads: Vec<&Event> = events
            .iter()
            .filter(|event| matches!(event, Event::ReadPixels { .. }))
            .collect();
        assert!(
            matches!(
                reads.as_slice(),
                [Event::ReadPixels { read_fbo, size }] if *read_fbo != 0 && *size == OUTPUT
            ),
            "{api:?}: one read-back, from an FBO, at the output size: {reads:?}"
        );

        // The control: presenting is the one thing that draws to the window.
        gl.trace_capture();
        player.present(OUTPUT);
        let presented = gl.trace_take();
        assert!(
            presented.iter().any(Event::touches_default_framebuffer),
            "{api:?}: the trace must see present() write the default framebuffer: {presented:?}"
        );
    }
    Ok(())
}

/// D311: every context is primed when it is built (`gl::prime`), with one
/// draw into a 16x16 target, because the mini PC's AMD WGL driver drops
/// draws into targets over 16384 pixels until the context has drawn into a
/// small one. llvmpipe never had that bug, so this cannot show that the
/// prime fixes it (only the mini PC can, MINIPC.md Task M15). What it pins
/// is that the prime is clean: a new context looks untouched (nothing
/// bound, the GL default unpack alignment, no pending error), a second
/// prime lands and changes neither bindings nor viewport, and a 640x360
/// offscreen render afterwards, above the AMD threshold, is right.
#[test]
fn the_context_prime_leaves_the_state_clean() -> Result<()> {
    const SIZE: [i32; 2] = [640, 360];
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        assert_eq!(
            gl.bindings(),
            Bindings::FRESH,
            "{api:?}: a primed context must look like a fresh one"
        );
        assert_eq!(gl.error(), None, "{api:?}: the prime left a GL error");

        let viewport = gl.viewport_box();
        let primed = prime::run(gl).map_err(|e| anyhow!("{api:?}: prime: {e}"))?;
        assert!(
            primed.landed && primed.errors == 0,
            "{api:?}: the prime's own draw must land cleanly: {primed:?}"
        );
        assert_eq!(
            gl.bindings(),
            Bindings::FRESH,
            "{api:?}: the prime must unbind everything it bound"
        );
        assert_eq!(
            gl.viewport_box(),
            viewport,
            "{api:?}: the prime must put the viewport back"
        );

        let caps = Caps::from_info(gl.info());
        let scene = folder(
            "{}",
            "vec4 renderMain() { return vec4(0.0, 1.0, 0.0, 1.0); }",
        )?;
        let controls = ControlBank::new(scene.def.controls.clone())?;
        let audio = vec![0.0; SCALARS.len()];
        let mut player = ScenePlayer::new(gl, &caps, &scene, SIZE, Options::default())?;
        player.render(&FrameInputs {
            time: 0.0,
            frame: 0,
            mouse: Mouse::default(),
            audio: &audio,
            controls: &controls,
            transitions: &[],
        });
        let pixels = player.read_output();
        assert_eq!(
            pixels.len(),
            (SIZE[0] * SIZE[1] * 4) as usize,
            "{api:?}: the read-back is the requested size"
        );
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| *px == [0, 255, 0, 255]),
            "{api:?}: every pixel of a 640x360 render after the prime is green"
        );
        assert_eq!(gl.error(), None, "{api:?}: GL error after the render");
    }
    Ok(())
}
