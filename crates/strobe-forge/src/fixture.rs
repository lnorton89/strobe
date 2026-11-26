// ============================================================================
// Module:       strobe_forge::fixture
// Description:  A small original scene and canned model replies for the
//               tests: one that compiles and moves, one that does not.
//
// Dependencies: anyhow, std::fs
// ============================================================================

//! Test fixtures. Original work, written for these tests.

use anyhow::Result;
use std::path::{Path, PathBuf};

/// Soft rings that drift and breathe with the bass.
pub(crate) const GLSL: &str = "\
vec4 renderMain(void) {
    vec2 p = _uvc / size;
    float r = length(p - 0.2 * vec2(sin(TIME * 0.7), cos(TIME * 0.5)));
    float rings = 0.5 + 0.5 * sin(r * 18.0 - TIME * 2.0);
    vec3 col = ink * rings * (0.4 + 0.6 * smoothstep(1.2, 0.0, r));
    col += flash * 0.3;
    return vec4(col, 1.0);
}
";

/// Its `scene.json`: a slider, a colour and a bang.
pub(crate) const SCENE: &str = r#"{
  "TITLE": "Rings",
  "CREDIT": "Strobe interpretation of a test pattern",
  "CONTROLS": [
    { "NAME": "size", "TYPE": "slider smooth", "MIN": 0.5, "MAX": 1.5, "DEFAULT": 1.0, "PARAMS": 0.03 },
    { "NAME": "ink", "TYPE": "color smooth", "DEF_COLOR": [0.9, 0.8, 0.5] },
    { "NAME": "flash", "TYPE": "bang smooth", "PARAMS": 0.1 }
  ],
  "TAGS": ["test"]
}"#;

/// Writes a scene folder.
pub(crate) fn write_scene(dir: &Path, glsl: &str, json: &str) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("main.glsl"), glsl)?;
    std::fs::write(dir.join("scene.json"), json)?;
    Ok(())
}

/// A model reply carrying `glsl` and [`SCENE`] in the requested format.
pub(crate) fn reply(glsl: &str) -> String {
    format!(
        "Here is the scene.\n\n<file name=\"main.glsl\">\n{glsl}</file>\n\n<file name=\"scene.json\">\n{SCENE}\n</file>\n"
    )
}

/// A fresh, empty folder for one test under the system's temporary folder.
pub(crate) fn tempdir(name: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir()
        .join("strobe-forge-tests")
        .join(format!("{name}-{}", std::process::id()));
    // Absent the first time; create_dir_all below reports a real problem.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
