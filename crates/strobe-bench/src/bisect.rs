// ============================================================================
// Module:       strobe_bench::bisect
// Description:  D296 experiment (throwaway): `--bisect`. Why do scene draws
//               into targets over 16384 pixels never land on the mini PC's
//               Vega 3 when the probe's draw does? GL state at each draw,
//               then one variable at a time: shader, vertex stage, target,
//               player path.
//
// Dependencies: anyhow; strobe-gfx (Gl, ScenePlayer, state capture);
//               strobe-scene (loading, assembly, controls)
// ============================================================================

//! The D296 bisect.
//!
//! The mini PC (M11) showed: clears land in every target; the scene's draws
//! never write a pixel into a target over 16384 pixels (alpha 0
//! everywhere, and a target cleared red stays red); the probe's trivial
//! draw lands at every size in the same context; the app's draws land at
//! 1920x1080 in its own context. This run takes the two draws apart:
//!
//! - **A. State.** The full draw-time GL state of the probe's draw and of
//!   the player's first draw, at the failing size, captured inside the draw
//!   wrapper itself ([`Gl::arm_state_capture`]); the differences print first.
//!   Also the player at a good size against the failing one.
//! - **B. Shader.** The probe's recipe (fresh target, clear red, read, draw,
//!   read) with one fragment shader changed per row, from the probe's own
//!   green to the scene's assembled program.
//! - **C. Vertex stage.** The player's vertex shader, and an in-range quad
//!   (six vertices inside clip space) instead of the oversized triangle.
//! - **D. Target.** The probe's draw into the player's own output texture.
//! - **E. Player path.** Trivial original scenes (solid green; `_uv`) through
//!   the full `ScenePlayer`, against the real scene.
//!
//! A row "lands" when the pixels it read back changed from what was there
//! before the draw. It writes the trivial scenes to `<out>/d296-scenes` so a
//! normal bench run can use them as a library too.

use anyhow::{Context as _, Result, anyhow};
use std::fmt::Write as _;
use std::path::Path;
use strobe_gfx::gl::{Format, Framebuffer, Gl, Program, Sampling, Stage, VertexArray};
use strobe_gfx::{Caps, FrameInputs, Mouse, Options, ScenePlayer};
use strobe_scene::SceneFolder;
use strobe_scene::assemble::{self, Dialect, PassIndex};
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;

/// The largest size seen to work and the smallest seen to fail (M11).
const GOOD: [i32; 2] = [128, 128];
const BAD: [i32; 2] = [129, 128];

/// The trivial scenes for the player-path rows: (folder, `renderMain` body).
const TRIVIAL: [(&str, &str); 2] = [
    ("01-d296-solid", "return vec4(0.0, 1.0, 0.0, 1.0);"),
    ("02-d296-uv", "return vec4(_uv, 0.5, 1.0);"),
];

/// How a row's vertex stage places the geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Geometry {
    /// The probe's own full-screen triangle shader.
    ProbeTriangle,
    /// `assemble::vertex`: the player's full-screen triangle shader.
    PlayerTriangle,
    /// Two triangles exactly covering clip space.
    Quad,
}

/// One shader row of section B/C.
struct Variant {
    name: String,
    geometry: Geometry,
    fragment: String,
}

/// Runs the bisect; returns the report.
pub(crate) fn run(
    gl: &Gl,
    caps: &Caps,
    context: &str,
    size: [i32; 2],
    scene_dir: &Path,
    out: &Path,
) -> Result<String> {
    let mut report = String::new();
    let _ = writeln!(report, "# D296 bisect\n");
    let _ = writeln!(
        report,
        "- Renderer: {} ({})",
        caps.info.renderer, caps.info.version
    );
    let _ = writeln!(report, "- Context: {context}");
    let _ = writeln!(report, "- Scene: {}", scene_dir.display());
    let _ = writeln!(report, "- Failing size used: {}x{}\n", size[0], size[1]);

    let library = out.join("d296-scenes");
    let trivial = write_trivial_scenes(&library)?;
    let real = SceneFolder::load(scene_dir)
        .map_err(|e| anyhow!("loading {}: {e}", scene_dir.display()))?;

    // D296 experiment (throwaway): the player path before anything else has
    // drawn in this process, failing size first, as the bench loop does.
    if std::env::var_os("STROBE_D296_E_FIRST").is_some() {
        let _ = writeln!(report, "## E0. The player path first, before A-D\n");
        for scene in trivial.iter().chain(std::iter::once(&real)) {
            for at in [size, GOOD, size] {
                let line = player_frames(gl, caps, scene, at, 3)?;
                let _ = writeln!(
                    report,
                    "- {} at {}x{}: {line}",
                    scene.def.title, at[0], at[1]
                );
            }
        }
        let _ = writeln!(report);
    }
    let painter = painter_fragment(caps.dialect);
    let vao = gl
        .vertex_array()
        .map_err(|e| anyhow!("vertex array: {e}"))?;

    // ---------------------------------------------------------------- A
    let _ = writeln!(report, "## A. GL state at the draw\n");
    let probe_state = {
        let program = link(gl, caps.dialect, Geometry::ProbeTriangle, &painter)?;
        let (row, state) = probe_draw(gl, &program, &vao, Geometry::ProbeTriangle, size, true)?;
        let _ = writeln!(report, "Probe draw at {}x{}: {row}", size[0], size[1]);
        state
    };
    let (player_bad, landed_bad) = player_state(gl, caps, &real, size)?;
    let _ = writeln!(
        report,
        "Player first draw at {}x{}: {landed_bad}",
        size[0], size[1]
    );
    let (player_good, landed_good) = player_state(gl, caps, &real, GOOD)?;
    let _ = writeln!(
        report,
        "Player first draw at {}x{}: {landed_good}\n",
        GOOD[0], GOOD[1]
    );
    match (&probe_state, &player_bad) {
        (Some(probe), Some(player)) => {
            let _ = writeln!(report, "### Differences, probe | player (failing size)\n");
            let _ = writeln!(report, "```");
            for line in probe.diff(player) {
                let _ = writeln!(report, "{line}");
            }
            let _ = writeln!(report, "```\n");
        }
        (None, _) | (_, None) => {
            let _ = writeln!(report, "A capture did not happen (no draw?)\n");
        }
    }
    if let (Some(good), Some(bad)) = (&player_good, &player_bad) {
        let _ = writeln!(
            report,
            "### Differences, player good size | player failing size\n"
        );
        let _ = writeln!(report, "```");
        for line in good.diff(bad) {
            let _ = writeln!(report, "{line}");
        }
        let _ = writeln!(report, "```\n");
    }
    for (label, state) in [("probe", &probe_state), ("player", &player_bad)] {
        if let Some(state) = state {
            let _ = writeln!(
                report,
                "### Full state, {label} (failing size)\n\n```\n{state}```\n"
            );
        }
    }

    // ---------------------------------------------------------- B and C
    let _ = writeln!(
        report,
        "## B/C. One shader or vertex change at a time (probe recipe)\n"
    );
    let _ = writeln!(
        report,
        "| Size | Variant | Landed % | Centre RGBA | Frag location | GL error |"
    );
    let _ = writeln!(report, "|---|---|---:|---|---:|---|");
    let variants = variants(caps.dialect, &painter, &trivial, &real);
    let mut sizes = vec![GOOD, BAD];
    if !sizes.contains(&size) {
        sizes.push(size);
    }
    for variant in &variants {
        let program = match link(gl, caps.dialect, variant.geometry, &variant.fragment) {
            Ok(program) => program,
            Err(error) => {
                let _ = writeln!(
                    report,
                    "| - | {} | could not build: {} | | | |",
                    variant.name,
                    error.to_string().replace('\n', " ")
                );
                continue;
            }
        };
        let location = gl
            .frag_data_location(&program, "strobe_FragColor")
            .max(gl.frag_data_location(&program, "color"));
        for &at in &sizes {
            match probe_draw(gl, &program, &vao, variant.geometry, at, false) {
                Ok((row, _)) => {
                    let _ = writeln!(
                        report,
                        "| {}x{} | {} | {:.1} | {:?} | {location} | {} |",
                        at[0], at[1], variant.name, row.landed, row.centre, row.error
                    );
                }
                Err(error) => {
                    let _ = writeln!(
                        report,
                        "| {}x{} | {} | failed: {error} | | | |",
                        at[0], at[1], variant.name
                    );
                }
            }
        }
    }

    // ---------------------------------------------------------------- D
    let _ = writeln!(
        report,
        "\n## D. The probe's draw into the player's own output texture\n"
    );
    let program = link(gl, caps.dialect, Geometry::ProbeTriangle, &painter)?;
    for at in [GOOD, size] {
        for own_fbo in [true, false] {
            let line = painter_into_player_target(gl, caps, &real, &program, &vao, at, own_fbo)?;
            let _ = writeln!(report, "- {}x{}: {line}", at[0], at[1]);
        }
    }

    // ---------------------------------------------------------------- E
    let _ = writeln!(report, "\n## E. The full player path, three frames\n");
    for scene in trivial.iter().chain(std::iter::once(&real)) {
        for at in [GOOD, size] {
            let line = player_frames(gl, caps, scene, at, 3)?;
            let _ = writeln!(
                report,
                "- {} at {}x{}: {line}",
                scene.def.title, at[0], at[1]
            );
        }
    }
    Ok(report)
}

/// What one probe-recipe draw did.
struct DrawRow {
    /// Share of pixels that changed from the clear, 0 to 100.
    landed: f64,
    /// The centre pixel after the draw.
    centre: [u8; 4],
    /// The GL error after the draw, if any.
    error: String,
}

impl std::fmt::Display for DrawRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "landed {:.1} %, centre {:?}, GL error {}",
            self.landed, self.centre, self.error
        )
    }
}

/// The probe's recipe with `program`: a fresh RGBA8 target, cleared red and
/// read, then one draw and a read. Optionally captures the draw-time state.
fn probe_draw(
    gl: &Gl,
    program: &Program,
    vao: &VertexArray,
    geometry: Geometry,
    size: [i32; 2],
    capture: bool,
) -> Result<(DrawRow, Option<strobe_gfx::gl::state::GlState>)> {
    let [w, h] = size;
    let texture = gl
        .texture_2d(w, h, Format::Rgba8, Sampling::LINEAR_CLAMP, None)
        .map_err(|e| anyhow!("target: {e}"))?;
    let fbo = gl.framebuffer(&texture).map_err(|e| anyhow!("fbo: {e}"))?;
    gl.clear_errors();
    gl.plain_state();
    gl.bind_framebuffer(Some(&fbo));
    gl.viewport(w, h);
    gl.clear([1.0, 0.0, 0.0, 1.0]);
    gl.finish();
    let before = gl.read_rgba8(&fbo, w, h);
    gl.bind_framebuffer(Some(&fbo));
    gl.viewport(w, h);
    gl.use_program(program);
    if let Some(location) = gl.uniform_location(program, "RENDERSIZE") {
        gl.uniform_vec2(&location, [w as f32, h as f32]);
    }
    if let Some(location) = gl.uniform_location(program, "d296_one") {
        gl.uniform_f32(&location, 1.0);
    }
    gl.bind_vertex_array(vao);
    if capture {
        gl.arm_state_capture();
    }
    match geometry {
        Geometry::Quad => gl.draw_triangles(6),
        Geometry::ProbeTriangle | Geometry::PlayerTriangle => gl.draw_fullscreen_triangle(),
    }
    let error = gl
        .error()
        .map_or_else(|| "none".to_owned(), |code| format!("0x{code:x}"));
    gl.finish();
    let after = gl.read_rgba8(&fbo, w, h);
    gl.bind_framebuffer(None);
    let state = if capture {
        gl.take_state_capture()
    } else {
        None
    };
    Ok((
        DrawRow {
            landed: changed(&before, &after),
            centre: centre(&after, size),
            error,
        },
        state,
    ))
}

/// Share of RGBA pixels that differ between `before` and `after`.
fn changed(before: &[u8], after: &[u8]) -> f64 {
    let (a, _) = before.as_chunks::<4>();
    let (b, _) = after.as_chunks::<4>();
    let total = a.len().min(b.len());
    if total == 0 {
        return 0.0;
    }
    let hits = a.iter().zip(b).filter(|(x, y)| x != y).count();
    hits as f64 * 100.0 / total as f64
}

/// The pixel at the middle of a bottom-up RGBA8 read.
fn centre(pixels: &[u8], [w, h]: [i32; 2]) -> [u8; 4] {
    let index = ((h / 2).max(0) as usize * w.max(0) as usize + (w / 2).max(0) as usize) * 4;
    pixels
        .get(index..index + 4)
        .and_then(|px| <[u8; 4]>::try_from(px).ok())
        .unwrap_or_default()
}

/// Share of pixels in a read with alpha other than 0 (the player's final
/// pass writes alpha 1; its targets start at alpha 0).
fn written(pixels: &[u8]) -> f64 {
    let (px, _) = pixels.as_chunks::<4>();
    if px.is_empty() {
        return 0.0;
    }
    px.iter().filter(|p| p[3] != 0).count() as f64 * 100.0 / px.len() as f64
}

/// Builds the player for `scene` at `size`, captures the state at its first
/// draw, renders one frame and says how much of the output was written.
fn player_state(
    gl: &Gl,
    caps: &Caps,
    scene: &SceneFolder,
    size: [i32; 2],
) -> Result<(Option<strobe_gfx::gl::state::GlState>, String)> {
    let mut player = ScenePlayer::new(gl, caps, scene, size, Options::default())
        .map_err(|e| anyhow!("player: {e}"))?;
    gl.finish();
    gl.arm_state_capture();
    render(&mut player, scene, 1)?;
    gl.finish();
    let state = gl.take_state_capture();
    let pixels = player.read_output();
    Ok((
        state,
        format!(
            "written (alpha != 0) {:.1} %, GL error {}",
            written(&pixels),
            gl.error()
                .map_or_else(|| "none".to_owned(), |code| format!("0x{code:x}"))
        ),
    ))
}

/// Renders `frames` frames of `scene` with the bench's inputs.
fn render(player: &mut ScenePlayer, scene: &SceneFolder, frames: u64) -> Result<()> {
    let mut controls =
        ControlBank::new(scene.def.controls.clone()).map_err(|e| anyhow!("controls: {e}"))?;
    let transitions: Vec<f32> = scene
        .def
        .transition_uniforms
        .iter()
        .map(|t| t.initial)
        .collect();
    let mut audio = vec![0.0; SCALARS.len()];
    let dt = 1.0 / 60.0;
    for frame in 0..frames {
        let time = 2.0 + frame as f32 * dt;
        strobe_scene::synthetic::fill(time, &mut audio);
        controls.tick(dt);
        player.render(&FrameInputs {
            time,
            frame,
            mouse: Mouse::default(),
            audio: &audio,
            controls: &controls,
            transitions: &transitions,
        });
    }
    Ok(())
}

/// The full player path: `frames` frames, then how much was written.
fn player_frames(
    gl: &Gl,
    caps: &Caps,
    scene: &SceneFolder,
    size: [i32; 2],
    frames: u64,
) -> Result<String> {
    let mut player = ScenePlayer::new(gl, caps, scene, size, Options::default())
        .map_err(|e| anyhow!("player: {e}"))?;
    render(&mut player, scene, frames)?;
    gl.finish();
    let pixels = player.read_output();
    Ok(format!(
        "written {:.1} %, centre {:?}",
        written(&pixels),
        centre(&pixels, player.render_size())
    ))
}

/// One frame of the real scene, then the probe's green draw into the
/// player's own output texture through a framebuffer of the bisect's own.
fn painter_into_player_target(
    gl: &Gl,
    caps: &Caps,
    scene: &SceneFolder,
    painter: &Program,
    vao: &VertexArray,
    size: [i32; 2],
    own_fbo: bool,
) -> Result<String> {
    let mut player = ScenePlayer::new(gl, caps, scene, size, Options::default())
        .map_err(|e| anyhow!("player: {e}"))?;
    render(&mut player, scene, 1)?;
    gl.finish();
    let [w, h] = player.render_size();
    let separate: Option<Framebuffer> = if own_fbo {
        None
    } else {
        Some(
            gl.framebuffer(player.output_texture())
                .map_err(|e| anyhow!("fbo on the player's texture: {e}"))?,
        )
    };
    let fbo = separate
        .as_ref()
        .unwrap_or_else(|| player.output_framebuffer());
    let before = gl.read_rgba8(fbo, w, h);
    gl.plain_state();
    gl.bind_framebuffer(Some(fbo));
    gl.viewport(w, h);
    gl.use_program(painter);
    gl.bind_vertex_array(vao);
    gl.draw_fullscreen_triangle();
    gl.finish();
    let after = gl.read_rgba8(fbo, w, h);
    gl.bind_framebuffer(None);
    Ok(format!(
        "through {}: player's frame written {:.1} %; probe draw into it changed {:.1} %, centre {:?}",
        if own_fbo {
            "the player's own framebuffer"
        } else {
            "a new framebuffer on the player's texture"
        },
        written(&before),
        changed(&before, &after),
        centre(&after, [w, h])
    ))
}

/// The `#version` (and precision) line for hand-written shaders.
fn header(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Es300 => "#version 300 es\nprecision highp float;\n",
        Dialect::Glsl330 => "#version 330 core\n",
        Dialect::Glsl410 => "#version 410 core\n",
    }
}

/// The probe's green fragment shader.
fn painter_fragment(dialect: Dialect) -> String {
    format!(
        "{}out vec4 color; void main() {{ color = vec4(0.0, 1.0, 0.0, 1.0); }}\n",
        header(dialect)
    )
}

/// Every section B/C row, one change from the painter (or from the row
/// above) each.
fn variants(
    dialect: Dialect,
    painter: &str,
    trivial: &[SceneFolder],
    real: &SceneFolder,
) -> Vec<Variant> {
    let h = header(dialect);
    let mut rows = vec![
        Variant {
            name: "B1 probe green (baseline)".to_owned(),
            geometry: Geometry::ProbeTriangle,
            fragment: painter.to_owned(),
        },
        Variant {
            name: "B2 green, output named strobe_FragColor".to_owned(),
            geometry: Geometry::ProbeTriangle,
            fragment: format!(
                "{h}out vec4 strobe_FragColor; void main() {{ strobe_FragColor = vec4(0.0, 1.0, 0.0, 1.0); }}\n"
            ),
        },
        Variant {
            name: "B3 green from a uniform".to_owned(),
            geometry: Geometry::ProbeTriangle,
            fragment: format!(
                "{h}uniform float d296_one; out vec4 color; void main() {{ color = vec4(0.0, d296_one, 0.0, 1.0); }}\n"
            ),
        },
        Variant {
            name: "B4 green, reads gl_FragCoord".to_owned(),
            geometry: Geometry::ProbeTriangle,
            fragment: format!(
                "{h}out vec4 color; void main() {{ color = vec4(0.0, gl_FragCoord.x >= 0.0 ? 1.0 : 0.5, 0.0, 1.0); }}\n"
            ),
        },
        Variant {
            name: "B5 gl_FragCoord / RENDERSIZE as colour".to_owned(),
            geometry: Geometry::ProbeTriangle,
            fragment: format!(
                "{h}uniform vec2 RENDERSIZE; out vec4 color; void main() {{ color = vec4(gl_FragCoord.xy / RENDERSIZE, 0.5, 1.0); }}\n"
            ),
        },
    ];
    for (index, scene) in trivial.iter().chain(std::iter::once(real)).enumerate() {
        rows.push(Variant {
            name: format!("B{} assembled: {}", 6 + index, scene.def.title),
            geometry: Geometry::ProbeTriangle,
            fragment: assemble::fragment(&assemble::Request {
                scene: &scene.def,
                user_code: scene.shader_for(scene.def.final_pass()),
                dialect,
                pass_index: PassIndex::Constant(scene.def.final_pass()),
                extra: &[],
            }),
        });
    }
    rows.push(Variant {
        name: "C1 probe green, player's vertex shader".to_owned(),
        geometry: Geometry::PlayerTriangle,
        fragment: painter.to_owned(),
    });
    rows.push(Variant {
        name: "C2 probe green, in-range quad".to_owned(),
        geometry: Geometry::Quad,
        fragment: painter.to_owned(),
    });
    rows.push(Variant {
        name: format!("C3 assembled {}, in-range quad", real.def.title),
        geometry: Geometry::Quad,
        fragment: assemble::fragment(&assemble::Request {
            scene: &real.def,
            user_code: real.shader_for(real.def.final_pass()),
            dialect,
            pass_index: PassIndex::Constant(real.def.final_pass()),
            extra: &[],
        }),
    });
    rows
}

/// Compiles and links one row's program.
fn link(gl: &Gl, dialect: Dialect, geometry: Geometry, fragment: &str) -> Result<Program> {
    let h = header(dialect);
    let vertex = match geometry {
        Geometry::ProbeTriangle => format!(
            "{h}void main() {{ vec2 p = vec2(float((gl_VertexID << 1) & 2), \
             float(gl_VertexID & 2)); gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0); }}\n"
        ),
        Geometry::PlayerTriangle => assemble::vertex(dialect),
        Geometry::Quad => format!(
            "{h}void main() {{ vec2 c[6] = vec2[6](vec2(-1.0, -1.0), vec2(1.0, -1.0), \
             vec2(-1.0, 1.0), vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0)); \
             gl_Position = vec4(c[gl_VertexID], 0.0, 1.0); }}\n"
        ),
    };
    let vertex = gl
        .compile(Stage::Vertex, &vertex)
        .map_err(|log| anyhow!("vertex: {log}"))?;
    let fragment = gl
        .compile(Stage::Fragment, fragment)
        .map_err(|log| anyhow!("fragment: {log}"))?;
    gl.link(&vertex, &fragment)
        .map_err(|log| anyhow!("link: {log}"))
}

/// Writes the trivial original scenes and loads them.
fn write_trivial_scenes(library: &Path) -> Result<Vec<SceneFolder>> {
    let mut scenes = Vec::new();
    for (folder, body) in TRIVIAL {
        let dir = library.join(format!("{folder}.synScene"));
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let json = format!(
            "{{\n  \"TITLE\": \"{folder}\",\n  \"CREDIT\": \"Strobe (D296 bisect)\",\n  \"CONTROLS\": []\n}}\n"
        );
        std::fs::write(dir.join("scene.json"), json)?;
        std::fs::write(
            dir.join("main.glsl"),
            format!("vec4 renderMain(void) {{\n    {body}\n}}\n"),
        )?;
        scenes
            .push(SceneFolder::load(&dir).map_err(|e| anyhow!("loading {}: {e}", dir.display()))?);
    }
    Ok(scenes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_counts_differing_pixels() -> Result<()> {
        let a = [1, 2, 3, 4, 5, 6, 7, 8];
        let b = [1, 2, 3, 4, 5, 6, 7, 9];
        let share = changed(&a, &b);
        assert!(
            (share - 50.0).abs() < 1e-9,
            "one of two changed, got {share}"
        );
        Ok(())
    }
}
