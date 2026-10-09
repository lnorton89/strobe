// ============================================================================
// Module:       convert (integration test)
// Description:  The converter's contract without a GPU: golden output,
//               determinism, error cases, warnings, assets and the writer.
//
// Dependencies: anyhow, serde_json; strobe-shadertoy; strobe-scene
// ============================================================================

//! Converter tests that need no GL context.
//!
//! The golden files under `tests/golden/` are the exact bytes the
//! converter produced when they were last reviewed. After an intended
//! change, run with `STROBE_UPDATE_GOLDEN=1` to rewrite them and read the
//! diff before committing.

use anyhow::{Context as _, Result, anyhow};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use strobe_shadertoy::convert::{ConvertError, Warning, write};
use strobe_shadertoy::{Converted, Options, Shader, convert};

const FIXTURES: [&str; 9] = [
    "single_image",
    "common_image",
    "buffer_feedback",
    "chain_four_buffers",
    "gl_traps",
    "legacy",
    "music",
    "missing_texture",
    "name_collisions",
];

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> Result<String> {
    let path = manifest()
        .join("tests/fixtures")
        .join(format!("{name}.json"));
    std::fs::read_to_string(&path).with_context(|| path.display().to_string())
}

fn import(name: &str) -> Result<Converted> {
    let shader = Shader::from_json(&fixture(name)?)?;
    Ok(convert(&shader, &Options::default())?)
}

fn text<'a>(converted: &'a Converted, file: &str) -> Result<&'a str> {
    let bytes = converted
        .files
        .get(file)
        .ok_or_else(|| anyhow!("no file {file}; have {:?}", converted.files.keys()))?;
    Ok(std::str::from_utf8(bytes)?)
}

/// A shader built in the test: `passes` are `renderpass` entries.
fn shader(passes: Value) -> Result<Shader> {
    Ok(Shader::from_value(json!({
        "info": { "id": "Tst200", "name": "t", "username": "u" },
        "renderpass": passes
    }))?)
}

fn pass(kind: &str, name: &str, code: &str, inputs: Value) -> Value {
    json!({ "type": kind, "name": name, "code": code, "inputs": inputs, "outputs": [] })
}

const MAIN: &str = "void mainImage(out vec4 c, in vec2 p) { c = vec4(1.0); }";

fn image(code: &str) -> Value {
    pass("image", "Image", code, json!([]))
}

fn error_of(shader: &Shader) -> String {
    convert(shader, &Options::default())
        .err()
        .map(|e| e.to_string())
        .unwrap_or_else(|| "(converted without error)".to_string())
}

#[test]
fn golden_output_is_unchanged() -> Result<()> {
    let golden = manifest().join("tests/golden");
    let update = std::env::var_os("STROBE_UPDATE_GOLDEN").is_some();
    for (fixture_name, file) in [
        ("single_image", "scene.json"),
        ("single_image", "main.glsl"),
        ("chain_four_buffers", "scene.json"),
        ("chain_four_buffers", "passes/st_clock.glsl"),
        ("chain_four_buffers", "passes/buffer_c.glsl"),
        ("common_image", "main.glsl"),
        ("legacy", "main.glsl"),
        ("music", "main.glsl"),
    ] {
        let converted = import(fixture_name)?;
        let actual = text(&converted, file)?;
        let path = golden.join(fixture_name).join(file);
        if update {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, actual)?;
        }
        let expected = std::fs::read_to_string(&path)
            .with_context(|| format!("{} (STROBE_UPDATE_GOLDEN=1 writes it)", path.display()))?;
        assert_eq!(
            actual, expected,
            "{fixture_name}/{file} differs from its golden file"
        );
    }
    Ok(())
}

#[test]
fn the_same_input_gives_byte_identical_output() -> Result<()> {
    for name in FIXTURES {
        let (first, second) = (import(name)?, import(name)?);
        assert_eq!(first, second, "{name}: two conversions differ");
        // The writer adds nothing of its own either.
        let library = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("determinism");
        let a = write(&first, &library.join("a"), true)?;
        let b = write(&second, &library.join("b"), true)?;
        for file in first.files.keys() {
            assert_eq!(
                std::fs::read(a.join(file))?,
                std::fs::read(b.join(file))?,
                "{name}/{file} differs on disk"
            );
        }
    }
    Ok(())
}

#[test]
fn every_fixture_makes_a_valid_scene_whose_files_exist() -> Result<()> {
    for name in FIXTURES {
        let converted = import(name)?;
        let json = text(&converted, "scene.json")?;
        let def = strobe_scene::SceneDef::parse(json, "t")?;
        let sources = def
            .pass_sources
            .as_ref()
            .ok_or_else(|| anyhow!("{name}: no PASS_SOURCES"))?;
        assert_eq!(sources.len(), def.passes.len(), "{name}: one file per pass");
        for file in sources {
            assert!(converted.files.contains_key(file), "{name}: {file} missing");
        }
        for image in &def.images {
            assert!(
                converted.files.contains_key(&image.path),
                "{name}: {} missing",
                image.path
            );
        }
        assert!(
            converted.files.contains_key("main.glsl"),
            "{name}: main.glsl"
        );
        assert_eq!(
            sources.last().map(String::as_str),
            Some("main.glsl"),
            "{name}: the Image pass is last"
        );
    }
    Ok(())
}

#[test]
fn the_five_strobe_controls_are_always_added_in_their_group() -> Result<()> {
    let converted = import("single_image")?;
    let def = strobe_scene::SceneDef::parse(text(&converted, "scene.json")?, "t")?;
    let names: Vec<&str> = def.controls.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "st_speed",
            "st_audio_time",
            "st_bass_flash",
            "st_zoom",
            "st_hue"
        ],
        "the PLAN 6.3 controls, prefixed"
    );
    let json: Value = serde_json::from_str(text(&converted, "scene.json")?)?;
    let groups: Vec<&str> = json["CONTROLS"]
        .as_array()
        .ok_or_else(|| anyhow!("no CONTROLS"))?
        .iter()
        .filter_map(|c| c["UI_GROUP"].as_str())
        .collect();
    assert_eq!(groups, vec!["Strobe"; 5], "all in the Strobe group");
    Ok(())
}

#[test]
fn the_source_block_keeps_provenance_and_the_tab_line_map() -> Result<()> {
    let converted = import("common_image")?;
    let json: Value = serde_json::from_str(text(&converted, "scene.json")?)?;
    let source = &json["SOURCE"];
    assert_eq!(source["kind"], "shadertoy", "kind");
    assert_eq!(source["id"], "Tst002", "id");
    assert_eq!(
        source["url"], "https://www.shadertoy.com/view/Tst002",
        "url"
    );
    assert_eq!(source["author"], "strobe-tests", "author");
    assert_eq!(source["name"], "Common and image", "name");
    let note = source["license_note"].as_str().unwrap_or_default();
    assert!(
        note.contains("CC BY-NC-SA") && note.contains("commercial"),
        "licence note: {note}"
    );

    // code_line really is where the pass tab's line 1 sits in the file.
    let main = text(&converted, "main.glsl")?;
    let lines: Vec<&str> = main.lines().collect();
    let entry = &source["passes"][0];
    let code_line = entry["code_line"].as_u64().unwrap_or(0) as usize;
    let common_line = entry["common_line"].as_u64().unwrap_or(0) as usize;
    assert!(
        lines
            .get(code_line - 1)
            .is_some_and(|l| l.starts_with("void mainImage")),
        "line {code_line} should open the Image tab, got {:?}",
        lines.get(code_line - 1)
    );
    assert!(
        lines
            .get(common_line - 1)
            .is_some_and(|l| l.starts_with("const float PI")),
        "line {common_line} should open the Common tab, got {:?}",
        lines.get(common_line - 1)
    );
    Ok(())
}

#[test]
fn only_the_names_a_pass_uses_are_in_its_shim() -> Result<()> {
    let converted = import("single_image")?;
    let main = text(&converted, "main.glsl")?;
    for used in ["iResolution", "iTime", "iMouse"] {
        assert!(
            main.contains(&format!("#define {used} strobe_{used}")),
            "{used} defined"
        );
    }
    for unused in [
        "iDate",
        "iFrame",
        "iChannel0",
        "iSampleRate",
        "iChannelResolution",
    ] {
        assert!(
            !main.contains(&format!("#define {unused}")),
            "{unused} not defined"
        );
    }
    // A shader that never reads time gets no clock pass at all.
    let converted = convert(&shader(json!([image(MAIN)]))?, &Options::default())?;
    assert!(
        !converted.files.contains_key("passes/st_clock.glsl"),
        "no clock file"
    );
    let json: Value = serde_json::from_str(text(&converted, "scene.json")?)?;
    assert_eq!(json["PASSES"], json!([]), "and no listed passes");
    Ok(())
}

#[test]
fn author_declarations_of_assembler_names_are_aliased_away() -> Result<()> {
    let converted = import("common_image")?;
    let main = text(&converted, "main.glsl")?;
    assert!(
        main.contains("#define PI strobe_user_PI"),
        "const float PI is renamed"
    );
    // A shader that #defines PI itself needs no alias.
    let code = format!("#define PI 3.14159\n{MAIN}\nfloat f() {{ return PI; }}");
    let converted = convert(&shader(json!([image(&code)]))?, &Options::default())?;
    assert!(
        !text(&converted, "main.glsl")?.contains("strobe_user_PI"),
        "a macro PI is left alone"
    );
    Ok(())
}

#[test]
fn legacy_source_is_normalised_and_reported() -> Result<()> {
    let converted = import("legacy")?;
    let buffer = text(&converted, "passes/buffer_a.glsl")?;
    let image = text(&converted, "main.glsl")?;
    assert!(
        !buffer.contains("precision") && !buffer.contains("#extension"),
        "stripped"
    );
    assert!(
        !buffer.contains("uniform vec3 iResolution"),
        "redeclaration removed"
    );
    assert!(
        image.contains("texture(iChannel0") || image.contains("texture(st_buf_a"),
        "texture2D renamed"
    );
    assert!(
        image.contains("strobe_gl_FragColor = texture"),
        "gl_FragColor renamed"
    );
    assert!(
        image.contains("return strobe_end(strobe_gl_FragColor)"),
        "and returned"
    );
    assert!(
        buffer.contains("#ifdef GL_ES\n#define tanh strobe_user_tanh\n#endif"),
        "a user tanh is aliased for ES"
    );
    assert!(
        converted
            .rewrites
            .iter()
            .any(|(pass, _)| pass == "Buffer A"),
        "rewrites are reported by pass: {:?}",
        converted.rewrites
    );
    Ok(())
}

#[test]
fn unsupported_things_are_errors_that_name_the_pass_and_channel() -> Result<()> {
    let cases = [
        (
            json!([
                image(MAIN),
                pass(
                    "sound",
                    "Sound",
                    "vec2 mainSound(int s, float t){return vec2(0.);}",
                    json!([])
                )
            ]),
            "Sound",
        ),
        (
            json!([image(MAIN), pass("cubemap", "Cube A", MAIN, json!([]))]),
            "Cubemap pass",
        ),
        (
            json!([pass(
                "image",
                "Image",
                MAIN,
                json!([{ "channel": 2, "ctype": "cubemap", "src": "/media/a/x.png" }])
            )]),
            "cubemap input on channel 2",
        ),
        (
            json!([pass(
                "image",
                "Image",
                MAIN,
                json!([{ "channel": 1, "ctype": "volume", "src": "/media/a/x.bin" }])
            )]),
            "volume input on channel 1",
        ),
        (json!([pass("hologram", "H", MAIN, json!([]))]), "hologram"),
    ];
    for (passes, needle) in cases {
        let message = error_of(&shader(passes)?);
        assert!(
            message.contains(needle) && message.contains("not supported"),
            "{needle}: {message}"
        );
    }
    Ok(())
}

#[test]
fn structural_mistakes_have_specific_errors() -> Result<()> {
    let image_with = |inputs: Value| pass("image", "Image", MAIN, inputs);
    let checks = [
        (
            json!([pass("common", "Common", "", json!([]))]),
            "no Image pass",
        ),
        (json!([image(MAIN), image(MAIN)]), "2 Image passes"),
        (json!([image("void notMain() {}")]), "no `mainImage`"),
        (
            json!([image_with(
                json!([{ "channel": 4, "ctype": "texture", "src": "/a.png" }])
            )]),
            "channels 0 to 3",
        ),
        (
            json!([image_with(json!([
                { "channel": 0, "ctype": "keyboard" }, { "channel": 0, "ctype": "keyboard" }
            ]))]),
            "two inputs on channel 0",
        ),
        (
            json!([image_with(
                json!([{ "id": 999, "channel": 0, "ctype": "buffer" }])
            )]),
            "matches no buffer pass",
        ),
        (
            json!([image_with(json!([{ "channel": 0, "ctype": "texture" }]))]),
            "no `src` or `filepath`",
        ),
        (
            json!([image(
                "float st_zoom = 1.0;\nvoid mainImage(out vec4 c, in vec2 p){c=vec4(st_zoom);}"
            )]),
            "reserves",
        ),
    ];
    for (passes, needle) in checks {
        let message = error_of(&shader(passes)?);
        assert!(
            message.contains(needle),
            "expected `{needle}`, got: {message}"
        );
    }
    let five: Vec<Value> = (0..5)
        .map(|i| pass("buffer", &format!("Buffer {i}"), MAIN, json!([])))
        .chain([image(MAIN)])
        .collect();
    assert!(
        error_of(&shader(Value::Array(five))?).contains("only A to D"),
        "five buffers"
    );
    Ok(())
}

#[test]
fn inputs_with_no_equivalent_yet_read_black_with_a_warning() -> Result<()> {
    let inputs = json!([
        { "channel": 0, "ctype": "video", "src": "/media/a/v.webm" },
        { "channel": 1, "ctype": "webcam" },
        { "channel": 2, "ctype": "keyboard" },
    ]);
    let code = "void mainImage(out vec4 c, in vec2 p) { c = texture(iChannel0, p) + texture(iChannel1, p) + texture(iChannel2, p); }";
    let converted = convert(
        &shader(json!([pass("image", "Image", code, inputs)]))?,
        &Options::default(),
    )?;
    let kinds: Vec<&str> = converted
        .warnings
        .iter()
        .filter_map(|w| {
            if let Warning::InputBoundToBlack { kind, .. } = w {
                Some(kind.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["video", "webcam", "keyboard"],
        "one warning each"
    );
    assert!(
        converted.files.contains_key("images/st_black.png"),
        "the black image is shipped"
    );
    let main = text(&converted, "main.glsl")?;
    assert!(
        main.contains("#define iChannel1 st_black"),
        "bound to black: {main}"
    );
    Ok(())
}

#[test]
fn reading_an_unconnected_channel_warns() -> Result<()> {
    let code = "void mainImage(out vec4 c, in vec2 p) { c = texture(iChannel3, p); }";
    let converted = convert(&shader(json!([image(code)]))?, &Options::default())?;
    assert!(
        converted
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::UnboundChannelRead { channel: 3, .. })),
        "{:?}",
        converted.warnings
    );
    Ok(())
}

#[test]
fn a_missing_texture_is_a_warning_with_its_url_or_an_error_when_strict() -> Result<()> {
    let shader = Shader::from_json(&fixture("missing_texture")?)?;
    let converted = convert(&shader, &Options::default())?;
    let message = converted
        .warnings
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        message.contains("0123456789abcdef")
            && message.contains("https://www.shadertoy.com/media/a/")
            && message.contains("reads black"),
        "the warning says which file, from where, and what happens: {message}"
    );
    let pending = text(&converted, "images/pending-assets.json")?;
    let list: Value = serde_json::from_str(pending)?;
    assert_eq!(list[0]["pass"], "Image", "pass");
    assert_eq!(list[0]["channel"], 0, "channel");
    assert!(
        list[0]["file"]
            .as_str()
            .unwrap_or("")
            .starts_with("images/0123"),
        "file"
    );

    let strict = convert(
        &shader,
        &Options {
            strict_assets: true,
            ..Options::default()
        },
    );
    assert!(
        matches!(&strict, Err(ConvertError::MissingAsset(a)) if a.channel == 0),
        "strict mode refuses: {strict:?}"
    );
    Ok(())
}

#[test]
fn a_supplied_texture_is_copied_in_with_its_sampler_settings() -> Result<()> {
    let name = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.jpg";
    let mut value: Value = serde_json::from_str(&fixture("missing_texture")?)?;
    value["renderpass"][0]["inputs"][0]["sampler"] = json!({
        "filter": "nearest", "wrap": "clamp", "vflip": "false", "srgb": "true", "internal": "byte"
    });
    let shader = Shader::from_value(value)?;
    let mut options = Options::default();
    options.assets.insert(name.to_string(), vec![1, 2, 3]);
    let converted = convert(&shader, &options)?;
    assert_eq!(
        converted.files.get(&format!("images/{name}")),
        Some(&vec![1, 2, 3]),
        "copied"
    );
    assert!(converted.pending_assets.is_empty(), "nothing pending");
    let def = strobe_scene::SceneDef::parse(text(&converted, "scene.json")?, "t")?;
    let image = &def.images[0];
    assert_eq!(image.path, format!("images/{name}"), "path");
    assert_eq!(
        (image.filter, image.wrap, image.vflip),
        (
            strobe_scene::scene::Filter::Nearest,
            strobe_scene::scene::Wrap::Clamp,
            false
        ),
        "filter, wrap and vflip survive"
    );
    assert!(
        converted
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::SrgbNotDecoded { .. })),
        "sRGB is not decoded, and the import says so: {:?}",
        converted.warnings
    );
    assert!(
        !converted.files.contains_key("images/st_black.png"),
        "no black needed"
    );
    Ok(())
}

#[test]
fn the_original_json_can_travel_with_the_scene() -> Result<()> {
    let shader = Shader::from_json(&fixture("single_image")?)?;
    let original = fixture("single_image")?;
    let converted = convert(
        &shader,
        &Options {
            original_json: Some(original.clone()),
            name: Some("My title".to_string()),
            ..Options::default()
        },
    )?;
    assert_eq!(text(&converted, "shadertoy.json")?, original, "verbatim");
    assert_eq!(converted.title, "My title", "--name wins");
    assert_eq!(
        converted.folder_name, "my-title-Tst001.synScene",
        "and names the folder"
    );
    Ok(())
}

#[test]
fn the_writer_is_atomic_and_refuses_to_clobber() -> Result<()> {
    let converted = import("chain_four_buffers")?;
    let library = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("writer");
    let _ = std::fs::remove_dir_all(&library);

    let dir = write(&converted, &library, false)?;
    assert!(
        dir.join("main.glsl").is_file() && dir.join("scene.json").is_file(),
        "written"
    );
    let leftovers: Vec<String> = std::fs::read_dir(&library)?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        leftovers,
        vec![converted.folder_name.clone()],
        "no staging folder left behind"
    );
    assert_eq!(
        strobe_scene::load::scan(&library)?,
        vec![dir.clone()],
        "the scene library lists the new scene"
    );

    let again = write(&converted, &library, false);
    assert!(
        again
            .err()
            .is_some_and(|e| e.to_string().contains("already exists")),
        "a second import without --force is refused"
    );
    std::fs::write(dir.join("stale.txt"), "x")?;
    write(&converted, &library, true)?;
    assert!(
        !dir.join("stale.txt").exists(),
        "--force replaces the whole folder"
    );
    assert!(
        Path::new(&dir).join("passes/buffer_d.glsl").is_file(),
        "and the new one is complete"
    );
    let entries = std::fs::read_dir(&library)?.count();
    assert_eq!(entries, 1, "no .replaced or .importing leftovers");
    Ok(())
}

#[test]
fn buffer_order_comes_from_names_then_ids_then_position() -> Result<()> {
    let mk = |name: &str, id: Value| {
        json!({ "type": "buffer", "name": name, "code": MAIN, "inputs": [],
                "outputs": [{ "id": id, "channel": 0 }] })
    };
    // Names missing: output ids 258 and 257 decide.
    let by_id = convert(
        &shader(json!([image(MAIN), mk("", json!(258)), mk("", json!(257))]))?,
        &Options::default(),
    )?;
    let sources = text(&by_id, "scene.json")?;
    assert!(
        sources.find("passes/buffer_a.glsl") < sources.find("passes/buffer_b.glsl"),
        "A before B: {sources}"
    );
    // Nothing usable: listed order is A, B.
    let by_position = convert(
        &shader(json!([
            image(MAIN),
            mk("x", json!("p")),
            mk("y", json!("q"))
        ]))?,
        &Options::default(),
    )?;
    assert!(
        by_position.files.contains_key("passes/buffer_b.glsl"),
        "two buffers, A and B"
    );
    Ok(())
}
