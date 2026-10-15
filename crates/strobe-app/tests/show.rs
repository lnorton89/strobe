// ============================================================================
// Module:       show (integration test)
// Description:  A playlist played end to end on a headless GL context:
//               precompile, a failed scene skipped, warm-up, a crossfade
//               seen in the mixed frame, hold and next, and zero shader
//               compiles after the load, in both GL dialects.
//
// Dependencies: anyhow; strobe-app (Show, Session, AudioInput, Library);
//               strobe-gfx (headless context, compositor); strobe-playlist
// ============================================================================

//! The show against a real GLSL compiler.
//!
//! Four tiny scenes (red, green, blue, and one that does not compile) are
//! written to a temp library and a playlist over them is driven by hand at
//! 60 Hz. The point is the promise PLAN section 7.4 makes: **zero shader
//! compiles during playback**. The context's own counter
//! (`Gl::shader_compiles`) is read after the load and again after the whole
//! show has played, transitions and all.
//!
//! Without EGL these tests report a skip and pass, unless
//! `STROBE_REQUIRE_HEADLESS_GL` is set (CI's Linux job).

use anyhow::{Result, anyhow};
use std::path::PathBuf;
use strobe_app::audio::AudioInput;
use strobe_app::library::{Entry, Library};
use strobe_app::session::Session;
use strobe_app::show::{Env, Phase, Show};
use strobe_gfx::{Api, Caps, Compositor, Headless, Mouse};
use strobe_playlist::{Command, Cue, DurationSpec, Playlist, SceneRef, Section, TransitionSpec};

const SIZE: [i32; 2] = [32, 18];
const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];
const DT: f32 = 1.0 / 60.0;

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

/// A library of solid-colour scenes in the temp directory, removed on drop.
struct TempLibrary {
    root: PathBuf,
    library: Library,
}

impl TempLibrary {
    fn new(tag: &str) -> Result<Self> {
        let root = std::env::temp_dir().join(format!("strobe-show-{}-{tag}", std::process::id()));
        let scenes: [(&str, &str); 4] = [
            (
                "red",
                "vec4 renderMain() { return vec4(1.0, 0.0, 0.0, 1.0); }",
            ),
            (
                "green",
                "vec4 renderMain() { return vec4(0.0, 1.0, 0.0, 1.0); }",
            ),
            (
                "blue",
                "vec4 renderMain() { return vec4(0.0, 0.0, 1.0, 1.0); }",
            ),
            ("broken", "vec4 renderMain() { return nonsense; }"),
        ];
        let mut entries = Vec::new();
        for (name, shader) in scenes {
            let dir = root.join(format!("{name}.synScene"));
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("scene.json"), "{}")?;
            std::fs::write(dir.join("main.glsl"), shader)?;
            entries.push(Entry::new(name.to_string(), dir));
        }
        Ok(Self {
            library: Library {
                dir: root.clone(),
                entries,
            },
            root,
        })
    }
}

impl Drop for TempLibrary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// red, broken, green (crossfade 1 s), blue (cut); 2 s each, play once.
fn playlist() -> Playlist {
    let mut playlist = Playlist::new("test show");
    playlist.defaults.duration = Some(DurationSpec::Seconds(2.0));
    let mut section = Section::new("all");
    let mut cue = |name: &str, transition: Option<TransitionSpec>| {
        let mut cue = Cue::new(SceneRef::Library(name.to_string()));
        cue.settings.transition = transition;
        section.cues.push(cue);
    };
    cue("red", None);
    cue("broken", None);
    cue("green", Some(TransitionSpec::crossfade(1.0)));
    cue("blue", Some(TransitionSpec::cut()));
    playlist.sections.push(section);
    playlist
}

/// Everything a show needs, and the way to run a frame of it.
struct Rig<'a> {
    gl: &'a strobe_gfx::gl::Gl,
    caps: Caps,
    active: Option<Session>,
    compositor: Compositor,
    audio: AudioInput,
}

impl<'a> Rig<'a> {
    fn new(headless: &'a Headless) -> Result<Self> {
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let compositor = Compositor::new(gl, &caps, SIZE)?;
        Ok(Self {
            gl,
            caps,
            active: None,
            compositor,
            audio: AudioInput::new(false),
        })
    }

    fn env(&mut self) -> Env<'_> {
        Env {
            gl: self.gl,
            caps: &self.caps,
            active: &mut self.active,
            compositor: &mut self.compositor,
            audio: &mut self.audio,
            full_size: SIZE,
            start_time: 0.0,
            mouse: Mouse::default(),
            date: [2026.0, 9.0, 10.0, 0.0],
            media: None,
        }
    }

    /// Compiles everything, one step per call, as the app does.
    fn load(&mut self, show: &mut Show) -> Result<()> {
        for _ in 0..100 {
            if show.compile_step(&mut self.env()).is_none() {
                return Ok(());
            }
        }
        Err(anyhow!("the precompile did not finish in 100 steps"))
    }

    fn frame(&mut self, show: &mut Show) {
        let _ = show.frame(&mut self.env(), DT, DT);
    }

    fn centre_of_output(&self, show: &Show) -> Result<[u8; 3]> {
        let (pixels, [w, h]) = if show.composited() {
            (self.compositor.read_output(), self.compositor.size())
        } else {
            let session = self
                .active
                .as_ref()
                .ok_or_else(|| anyhow!("nothing is playing"))?;
            (session.player.read_output(), session.player.render_size())
        };
        let offset = ((h / 2 * w + w / 2) * 4) as usize;
        let px = pixels
            .get(offset..offset + 3)
            .ok_or_else(|| anyhow!("readback too short"))?;
        Ok([px[0], px[1], px[2]])
    }
}

#[test]
fn a_playlist_plays_through_and_compiles_nothing_after_its_load() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let library = TempLibrary::new(&format!("play-{api:?}"))?;
        let mut rig = Rig::new(&headless)?;
        let mut show = Show::new(None, playlist(), &library.library, None, None);

        // Loading: nothing plays until every scene has been tried.
        assert_eq!(show.phase(), Phase::Compiling, "{api:?}: starts compiling");
        rig.load(&mut show)?;
        assert_eq!(show.phase(), Phase::Ready, "{api:?}: ready after the load");
        let progress = show.progress();
        assert_eq!(
            (progress.total, progress.failed),
            (4, 1),
            "{api:?}: four scenes to compile, the broken one failed"
        );
        assert!(
            show.cue_failure(1).is_some(),
            "{api:?}: the broken scene's cue knows why it cannot play"
        );
        let after_load = rig.gl.shader_compiles();

        // Play the whole show: 2 s + (skipped) + 2 s + 2 s = 6 s, plus slack.
        let mut order = Vec::new();
        let mut mixed = None;
        let mut warmed = false;
        for _ in 0..(8 * 60) {
            rig.frame(&mut show);
            let current = show.clock().current();
            if current.is_some() && order.last() != Some(&current) {
                order.push(current);
            }
            warmed |= show.warming_cue().is_some();
            if show.composited() && mixed.is_none() {
                let progress = show.clock().transition().map_or(0.0, |t| t.progress);
                if progress > 0.3 {
                    mixed = Some(rig.centre_of_output(&show)?);
                }
            }
        }
        assert_eq!(
            order,
            vec![Some(0), Some(2), Some(3)],
            "{api:?}: the broken cue was passed over"
        );
        assert!(
            warmed,
            "{api:?}: the next scene was warmed up before its cue"
        );
        let mix = mixed.ok_or_else(|| anyhow!("{api:?}: no transition frame was composited"))?;
        assert!(
            mix[0] > 20 && mix[1] > 20 && mix[2] == 0,
            "{api:?}: mid-crossfade the frame holds both red and green, got {mix:?}"
        );
        assert!(show.ended(), "{api:?}: the playlist ended");
        assert_eq!(
            rig.gl.shader_compiles(),
            after_load,
            "{api:?}: shader compiles happened during playback"
        );
        assert_eq!(show.runtime_compiles(), 0, "{api:?}: the show's own count");
        // The last cue (a cut to blue) is what stays on screen.
        let last = rig.centre_of_output(&show)?;
        assert!(
            last[2] > 250 && last[0] == 0 && last[1] == 0,
            "{api:?}: the last scene stays up, got {last:?}"
        );
        // Red and green went back to the cache, parked.
        assert_eq!(show.parked_count(), 2, "{api:?}: two scenes parked");
        if let Some(code) = rig.gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
        show.finish();
    }
    Ok(())
}

#[test]
fn hold_freezes_the_cue_and_next_goes_to_the_next_playable_one() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let library = TempLibrary::new("commands")?;
    let mut rig = Rig::new(&headless)?;
    let mut show = Show::new(None, playlist(), &library.library, None, None);
    rig.load(&mut show)?;
    for _ in 0..30 {
        rig.frame(&mut show);
    }
    assert_eq!(show.clock().current(), Some(0), "playing the first cue");

    show.command(Command::ToggleHold);
    rig.frame(&mut show);
    let frozen = show.clock().elapsed();
    for _ in 0..120 {
        rig.frame(&mut show);
    }
    assert!(show.clock().is_held(), "held");
    assert!(
        (show.clock().elapsed() - frozen).abs() < 1e-9,
        "a held cue's clock does not move"
    );
    show.command(Command::ToggleHold);

    show.command(Command::Next);
    rig.frame(&mut show);
    assert_eq!(
        show.clock().current(),
        Some(2),
        "next skips the cue whose scene failed"
    );
    let compiles = rig.gl.shader_compiles();
    show.command(Command::Extend(30.0));
    show.command(Command::RestartCue);
    show.command(Command::Previous);
    for _ in 0..60 {
        rig.frame(&mut show);
    }
    assert_eq!(
        rig.gl.shader_compiles(),
        compiles,
        "commands compile nothing either"
    );
    show.finish();
    Ok(())
}

#[test]
fn a_reloaded_playlist_compiles_only_what_is_new_and_keeps_its_place() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let library = TempLibrary::new("reload")?;
    let mut rig = Rig::new(&headless)?;
    let mut show = Show::new(None, playlist(), &library.library, None, None);
    rig.load(&mut show)?;
    for _ in 0..30 {
        rig.frame(&mut show);
    }
    let before = rig.gl.shader_compiles();

    // The same show with one more cue at the end, using a scene it has.
    let mut edited = playlist();
    if let Some(section) = edited.sections.first_mut() {
        section
            .cues
            .push(Cue::new(SceneRef::Library("red".to_string())));
    }
    show.replace_playlist(edited, &library.library);
    assert_eq!(show.phase(), Phase::Compiling, "back to loading");
    // Frames while it loads still advance the live scene (the app does that).
    rig.load(&mut show)?;
    assert_eq!(
        rig.gl.shader_compiles(),
        before,
        "nothing was compiled again: every scene was already there"
    );
    rig.frame(&mut show);
    assert_eq!(
        show.clock().current(),
        Some(0),
        "the show is back on the cue it was on"
    );
    show.finish();
    Ok(())
}

/// One scene, `tint`, whose red channel is its `amount` control, with a
/// `default` preset (0.5) and a `full` preset (1.0).
fn preset_library(tag: &str) -> Result<TempLibrary> {
    let root = std::env::temp_dir().join(format!("strobe-show-{}-{tag}", std::process::id()));
    let dir = root.join("tint.synScene");
    std::fs::create_dir_all(dir.join("presets"))?;
    std::fs::write(
        dir.join("scene.json"),
        r#"{ "CONTROLS": [ { "NAME": "amount", "TYPE": "slider", "MIN": 0.0, "MAX": 1.0, "DEFAULT": 0.0 } ] }"#,
    )?;
    std::fs::write(
        dir.join("main.glsl"),
        "vec4 renderMain() { return vec4(amount, 0.0, 0.0, 1.0); }",
    )?;
    std::fs::write(
        dir.join("presets/default.json"),
        r#"{ "name": "default", "scene": { "amount": 0.5 } }"#,
    )?;
    std::fs::write(
        dir.join("presets/full.json"),
        r#"{ "name": "full", "scene": { "amount": 1.0 } }"#,
    )?;
    Ok(TempLibrary {
        library: Library {
            dir: root.clone(),
            entries: vec![Entry::new("tint".to_string(), dir)],
        },
        root,
    })
}

fn amount(rig: &Rig<'_>) -> Result<f32> {
    let session = rig
        .active
        .as_ref()
        .ok_or_else(|| anyhow!("nothing is playing"))?;
    let index = session
        .controls
        .find("amount")
        .ok_or_else(|| anyhow!("no amount control"))?;
    session
        .controls
        .target(index)
        .map(|values| values[0])
        .ok_or_else(|| anyhow!("no target"))
}

#[test]
fn a_cues_preset_is_applied_when_the_cue_starts_and_an_override_beats_it() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let library = preset_library(&format!("presets-{api:?}"))?;
        let mut rig = Rig::new(&headless)?;
        let mut playlist = Playlist::new("presets");
        playlist.defaults.duration = Some(DurationSpec::Seconds(1.0));
        let mut section = Section::new("all");
        // 1: the scene's own default preset (no preset named);
        // 2: the cue's `full` preset;
        // 3: `full`, but the cue's explicit override says 0.25;
        // 4: a preset that does not exist (warned, plays on the defaults).
        let mut cue = Cue::new(SceneRef::Library("tint".to_string()));
        section.cues.push(cue.clone());
        cue.preset = Some("full".to_string());
        section.cues.push(cue.clone());
        cue.overrides.insert(
            "amount".to_string(),
            strobe_playlist::ControlValue::Float(0.25),
        );
        section.cues.push(cue.clone());
        cue.preset = Some("missing".to_string());
        cue.overrides.clear();
        section.cues.push(cue);
        playlist.sections.push(section);

        let mut show = Show::new(None, playlist, &library.library, None, None);
        rig.load(&mut show)?;
        let after_load = rig.gl.shader_compiles();
        let mut seen: Vec<(usize, f32)> = Vec::new();
        for _ in 0..(5 * 60) {
            rig.frame(&mut show);
            if let Some(current) = show.clock().current()
                && !seen.iter().any(|(cue, _)| *cue == current)
            {
                // The first frame of a cue, after its settings were applied.
                seen.push((current, amount(&rig)?));
            }
        }
        let by_cue = |index: usize| seen.iter().find(|(cue, _)| *cue == index).map(|(_, a)| *a);
        let close = |a: Option<f32>, b: f32| a.is_some_and(|a| (a - b).abs() < 1e-4);
        assert!(
            close(by_cue(0), 0.5),
            "{api:?}: a cue with no preset starts from the scene's default preset: {seen:?}"
        );
        assert!(
            close(by_cue(1), 1.0),
            "{api:?}: the cue's preset is applied: {seen:?}"
        );
        assert!(
            close(by_cue(2), 0.25),
            "{api:?}: an explicit override beats the preset: {seen:?}"
        );
        assert!(
            close(by_cue(3), 0.5),
            "{api:?}: a preset that is not there falls back to the default preset: {seen:?}"
        );
        assert_eq!(
            rig.gl.shader_compiles(),
            after_load,
            "{api:?}: applying presets compiles nothing"
        );
        show.finish();
    }
    Ok(())
}

/// The live session's meta value `name`, as a target.
fn meta(rig: &Rig<'_>, name: &str) -> Result<f32> {
    let session = rig
        .active
        .as_ref()
        .ok_or_else(|| anyhow!("nothing is playing"))?;
    let index = session
        .meta
        .find(name)
        .ok_or_else(|| anyhow!("no meta `{name}`"))?;
    session
        .meta
        .target(index)
        .map(|values| values[0])
        .ok_or_else(|| anyhow!("no target"))
}

#[test]
fn the_operators_output_settings_carry_across_cues_under_a_cues_own_meta() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let library = TempLibrary::new(&format!("meta-{api:?}"))?;
        let mut rig = Rig::new(&headless)?;
        let mut playlist = Playlist::new("meta");
        playlist.defaults.duration = Some(DurationSpec::Seconds(1.0));
        let mut section = Section::new("all");
        section
            .cues
            .push(Cue::new(SceneRef::Library("red".to_string())));
        let mut green = Cue::new(SceneRef::Library("green".to_string()));
        green.settings.transition = Some(TransitionSpec::cut());
        section.cues.push(green);
        // The third cue sets its own hue; master still carries.
        let mut blue = Cue::new(SceneRef::Library("blue".to_string()));
        blue.settings.transition = Some(TransitionSpec::cut());
        let mut values = std::collections::BTreeMap::new();
        values.insert("hue".to_string(), strobe_playlist::ControlValue::Float(0.5));
        blue.settings.meta = Some(strobe_playlist::MetaOverrides { values });
        section.cues.push(blue);
        playlist.sections.push(section);

        let mut show = Show::new(None, playlist, &library.library, None, None);
        rig.load(&mut show)?;
        rig.frame(&mut show);
        // The operator fades the output out and shifts the hue during cue 1.
        if let Some(session) = rig.active.as_mut() {
            for (name, value) in [("master", 0.0), ("hue", 0.25)] {
                let index = session
                    .meta
                    .find(name)
                    .ok_or_else(|| anyhow!("no meta `{name}`"))?;
                session.meta.set(index, None, &[value]);
            }
        }
        let mut seen: Vec<(usize, f32, f32)> = Vec::new();
        for _ in 0..(4 * 60) {
            rig.frame(&mut show);
            if let Some(current) = show.clock().current()
                && !seen.iter().any(|(cue, ..)| *cue == current)
            {
                seen.push((current, meta(&rig, "master")?, meta(&rig, "hue")?));
            }
        }
        let cue = |index: usize| seen.iter().find(|(c, ..)| *c == index).copied();
        assert_eq!(
            cue(1),
            Some((1, 0.0, 0.25)),
            "{api:?}: the next scene keeps master and hue: {seen:?}"
        );
        assert_eq!(
            cue(2),
            Some((2, 0.0, 0.5)),
            "{api:?}: a cue's own meta wins, the rest carries: {seen:?}"
        );
        show.finish();
    }
    Ok(())
}

/// A library with one scene, `pic`, that shows `syn_Media` and names
/// `own.png` as its own; `chosen.png` sits beside it for a cue to name.
fn media_library(tag: &str) -> Result<TempLibrary> {
    let root = std::env::temp_dir().join(format!("strobe-show-{}-{tag}", std::process::id()));
    let dir = root.join("pic.synScene");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("scene.json"), r#"{ "MEDIA": "own.png" }"#)?;
    std::fs::write(
        dir.join("main.glsl"),
        "vec4 renderMain() { return texture(syn_Media, _uv); }",
    )?;
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])).save(dir.join("own.png"))?;
    image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 255, 0, 255]))
        .save(root.join("chosen.png"))?;
    // A preset whose media channel names a picture inside the scene (D223).
    image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 255, 255])).save(dir.join("alt.png"))?;
    std::fs::create_dir_all(dir.join("presets"))?;
    std::fs::write(
        dir.join("presets/alt.json"),
        r#"{ "name": "alt", "channels": { "scene": true, "meta": true, "media": true }, "media": "alt.png" }"#,
    )?;
    Ok(TempLibrary {
        library: Library {
            dir: root.clone(),
            entries: vec![Entry::new("pic".to_string(), dir)],
        },
        root,
    })
}

#[test]
fn a_cues_media_replaces_the_scenes_own_and_the_next_cue_gets_it_back() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let library = media_library(&format!("media-{api:?}"))?;
        let chosen = library.root.join("chosen.png");
        let mut rig = Rig::new(&headless)?;
        let mut playlist = Playlist::new("media");
        playlist.defaults.duration = Some(DurationSpec::Seconds(1.0));
        let mut section = Section::new("all");
        // 1: the scene's own; 2: the cue's; 3: the scene's own again; 4: the
        // cue's preset's (D223); 5: the cue's media beats its preset's. The
        // same scene throughout, so later cues restart the live session.
        let cue = Cue::new(SceneRef::Library("pic".to_string()));
        section.cues.push(cue.clone());
        let mut named = cue.clone();
        named.media = Some(chosen.clone());
        section.cues.push(named.clone());
        section.cues.push(cue.clone());
        let mut preset = cue;
        preset.preset = Some("alt".to_string());
        section.cues.push(preset.clone());
        preset.media = Some(chosen.clone());
        section.cues.push(preset);
        playlist.sections.push(section);

        let mut show = Show::new(None, playlist, &library.library, None, None);
        rig.load(&mut show)?;
        let after_load = rig.gl.shader_compiles();
        let mut seen: Vec<(usize, Option<PathBuf>)> = Vec::new();
        for _ in 0..(6 * 60) {
            rig.frame(&mut show);
            if let Some(current) = show.clock().current()
                && !seen.iter().any(|(cue, _)| *cue == current)
            {
                let showing = rig
                    .active
                    .as_ref()
                    .and_then(|session| session.player.media_request())
                    .map(|request| request.path.clone());
                seen.push((current, showing));
            }
        }
        let by_cue = |index: usize| {
            seen.iter()
                .find(|(cue, _)| *cue == index)
                .and_then(|(_, path)| path.clone())
        };
        let own = library.library.entries[0].dir.join("own.png");
        assert_eq!(
            by_cue(0),
            Some(own.clone()),
            "{api:?}: the scene's own first: {seen:?}"
        );
        assert_eq!(
            by_cue(1),
            Some(chosen.clone()),
            "{api:?}: then the cue's: {seen:?}"
        );
        assert_eq!(
            by_cue(2),
            Some(own),
            "{api:?}: then the scene's own again: {seen:?}"
        );
        let alt = library.library.entries[0].dir.join("alt.png");
        assert_eq!(by_cue(3), Some(alt), "{api:?}: then the preset's: {seen:?}");
        assert_eq!(
            by_cue(4),
            Some(chosen),
            "{api:?}: and the cue's beats its preset's: {seen:?}"
        );
        assert_eq!(
            rig.gl.shader_compiles(),
            after_load,
            "{api:?}: changing media compiles nothing"
        );
        show.finish();
    }
    Ok(())
}

/// A library of the given `(name, scene.json, main.glsl)` scenes.
fn library_of(tag: &str, scenes: &[(&str, &str, &str)]) -> Result<TempLibrary> {
    let root = std::env::temp_dir().join(format!("strobe-show-{}-{tag}", std::process::id()));
    let mut entries = Vec::new();
    for (name, json, shader) in scenes {
        let dir = root.join(format!("{name}.synScene"));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("scene.json"), json)?;
        std::fs::write(dir.join("main.glsl"), shader)?;
        entries.push(Entry::new((*name).to_string(), dir));
    }
    Ok(TempLibrary {
        library: Library {
            dir: root.clone(),
            entries,
        },
        root,
    })
}

/// A 1x1 double-buffered counter (a literal-size pass, which a resize
/// keeps, D287) shown in red as the count over [`COUNT_SCALE`].
const COUNTER_JSON: &str = r#"{ "PASSES": [
    {"TARGET": "count", "DOUBLE_BUFFER": true, "WIDTH": 1, "HEIGHT": 1,
     "FLOAT": true, "FULL_PRECISION": true, "FILTER": "nearest"} ] }"#;
const COUNTER_SHADER: &str = "vec4 renderMain() {\n\
    if (PASSINDEX == 0) { return texture(count, vec2(0.5)) + vec4(1.0, 0.0, 0.0, 0.0); }\n\
    return vec4(texture(count, vec2(0.5)).r / 400.0, 0.0, 0.0, 1.0); }";
const COUNT_SCALE: f32 = 400.0;

/// One slider, `amount`, shown as the red channel.
const AMOUNT_JSON: &str = r#"{ "CONTROLS": [ { "NAME": "amount", "TYPE": "slider", "MIN": 0.0, "MAX": 1.0, "DEFAULT": 0.0 } ] }"#;
const AMOUNT_SHADER: &str = "vec4 renderMain() { return vec4(amount, 0.0, 0.0, 1.0); }";
const RED_SHADER: &str = "vec4 renderMain() { return vec4(1.0, 0.0, 0.0, 1.0); }";
const GREEN_SHADER: &str = "vec4 renderMain() { return vec4(0.0, 1.0, 0.0, 1.0); }";

/// The live scene's own frame (not the mix), centre pixel.
fn live_centre(rig: &Rig<'_>) -> Result<[u8; 3]> {
    let session = rig
        .active
        .as_ref()
        .ok_or_else(|| anyhow!("nothing is playing"))?;
    let pixels = session.player.read_output();
    let [w, h] = session.player.render_size();
    let offset = ((h / 2 * w + w / 2) * 4) as usize;
    let px = pixels
        .get(offset..offset + 3)
        .ok_or_else(|| anyhow!("readback too short"))?;
    Ok([px[0], px[1], px[2]])
}

fn cue_of(scene: &str, seconds: f64, transition: TransitionSpec) -> Cue {
    let mut cue = Cue::new(SceneRef::Library(scene.to_string()));
    cue.settings.duration = Some(DurationSpec::Seconds(seconds));
    cue.settings.transition = Some(transition);
    cue
}

fn with_amount(mut cue: Cue, amount: f32) -> Cue {
    cue.overrides.insert(
        "amount".to_string(),
        strobe_playlist::ControlValue::Float(amount),
    );
    cue
}

#[test]
fn a_scene_cued_again_starts_fresh_rather_than_from_its_last_cues_buffers() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let library = library_of(
            &format!("recue-{api:?}"),
            &[
                ("counter", COUNTER_JSON, COUNTER_SHADER),
                ("red", "{}", RED_SHADER),
            ],
        )?;
        let mut rig = Rig::new(&headless)?;
        let mut playlist = Playlist::new("recue");
        let mut section = Section::new("all");
        section
            .cues
            .push(cue_of("counter", 3.0, TransitionSpec::cut()));
        section.cues.push(cue_of("red", 3.0, TransitionSpec::cut()));
        section
            .cues
            .push(cue_of("counter", 3.0, TransitionSpec::cut()));
        playlist.sections.push(section);
        let mut show = Show::new(None, playlist, &library.library, None, None);
        rig.load(&mut show)?;
        let count = |rig: &Rig<'_>| -> Result<f32> {
            Ok(f32::from(live_centre(rig)?[0]) / 255.0 * COUNT_SCALE)
        };
        let mut end_of_first = None;
        let mut start_of_third = None;
        for _ in 0..(8 * 60) {
            rig.frame(&mut show);
            match show.clock().current() {
                Some(0) => end_of_first = Some(count(&rig)?),
                Some(2) if start_of_third.is_none() => start_of_third = Some(count(&rig)?),
                _ => {}
            }
        }
        let (Some(first), Some(third)) = (end_of_first, start_of_third) else {
            return Err(anyhow!("{api:?}: the counter cues did not both play"));
        };
        // The third cue's counter has run only through its warm-up (the
        // two seconds before it starts, about 120 frames); one that carried
        // the first cue's buffers would read about 180 more.
        assert!(
            third < first,
            "{api:?}: a re-cued scene starts its passes over: the first cue ended at \
             {first:.0}, the third began at {third:.0}"
        );
        show.finish();
    }
    Ok(())
}

#[test]
fn a_cues_overrides_reach_the_picture_every_time_its_scene_plays() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let library = library_of(
            &format!("repeat-{api:?}"),
            &[
                ("tint", AMOUNT_JSON, AMOUNT_SHADER),
                ("red", "{}", RED_SHADER),
            ],
        )?;
        let mut rig = Rig::new(&headless)?;
        let mut playlist = Playlist::new("repeats");
        let mut section = Section::new("all");
        let fade = TransitionSpec::crossfade(0.5);
        // The same scene three times, once after another scene and once
        // straight after itself, each with its own value.
        section
            .cues
            .push(with_amount(cue_of("tint", 2.0, fade.clone()), 0.2));
        section.cues.push(cue_of("red", 2.0, fade.clone()));
        section
            .cues
            .push(with_amount(cue_of("tint", 2.0, fade.clone()), 0.8));
        section
            .cues
            .push(with_amount(cue_of("tint", 2.0, fade), 0.4));
        playlist.sections.push(section);
        let mut show = Show::new(None, playlist, &library.library, None, None);
        rig.load(&mut show)?;
        let mut last = [None; 4];
        for _ in 0..(9 * 60) {
            rig.frame(&mut show);
            if let Some(current) = show.clock().current()
                && let Some(slot) = last.get_mut(current)
            {
                *slot = Some(live_centre(&rig)?[0]);
            }
        }
        let expected = [51_u8, 255, 204, 102];
        for (index, (seen, want)) in last.iter().zip(expected).enumerate() {
            let Some(seen) = seen else {
                return Err(anyhow!("{api:?}: cue {index} never played"));
            };
            assert!(
                seen.abs_diff(want) <= 2,
                "{api:?}: cue {index} ends drawing red {seen}, its override says {want}: {last:?}"
            );
        }
        show.finish();
    }
    Ok(())
}

#[test]
fn jumping_to_a_cue_whose_scene_is_warming_for_another_cue_applies_its_own_overrides() -> Result<()>
{
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let library = library_of(
        "jump-warm",
        &[
            ("tint", AMOUNT_JSON, AMOUNT_SHADER),
            ("red", "{}", RED_SHADER),
            ("green", "{}", GREEN_SHADER),
        ],
    )?;
    let mut rig = Rig::new(&headless)?;
    let mut playlist = Playlist::new("jump");
    let mut section = Section::new("all");
    let cut = TransitionSpec::cut();
    section.cues.push(cue_of("red", 3.0, cut.clone()));
    section
        .cues
        .push(with_amount(cue_of("tint", 3.0, cut.clone()), 0.8));
    section.cues.push(cue_of("green", 3.0, cut.clone()));
    section
        .cues
        .push(with_amount(cue_of("tint", 3.0, cut), 0.2));
    playlist.sections.push(section);
    let mut show = Show::new(None, playlist, &library.library, None, None);
    rig.load(&mut show)?;
    // 1.5 s into the first cue the second (tint at 0.8) is warming.
    for _ in 0..90 {
        rig.frame(&mut show);
    }
    assert_eq!(show.warming_cue(), Some(1), "cue 2 is warming");
    // The operator jumps to cue 4: the same scene, another value.
    show.command(Command::JumpTo(3));
    for _ in 0..3 {
        rig.frame(&mut show);
    }
    assert_eq!(show.clock().current(), Some(3), "on cue 4");
    let value = amount(&rig)?;
    assert!(
        (value - 0.2).abs() < 1e-4,
        "cue 4 plays with its own override (0.2), not the warmed cue's: {value}"
    );
    show.finish();
    Ok(())
}

#[test]
fn saving_the_playlist_leaves_the_live_scene_alone_unless_its_cue_changed() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let library = library_of(
        "reload-live",
        &[
            ("tint", AMOUNT_JSON, AMOUNT_SHADER),
            ("red", "{}", RED_SHADER),
        ],
    )?;
    let mut rig = Rig::new(&headless)?;
    let edited = |live_amount: f32, last: &str| {
        let mut playlist = Playlist::new("reload");
        let mut section = Section::new("all");
        section.cues.push(with_amount(
            cue_of("tint", 30.0, TransitionSpec::cut()),
            live_amount,
        ));
        section.cues.push(cue_of(last, 30.0, TransitionSpec::cut()));
        playlist.sections.push(section);
        playlist
    };
    let mut show = Show::new(None, edited(0.2, "red"), &library.library, None, None);
    rig.load(&mut show)?;
    for _ in 0..120 {
        rig.frame(&mut show);
    }
    // The operator nudges the live scene.
    if let Some(session) = rig.active.as_mut() {
        let index = session
            .controls
            .find("amount")
            .ok_or_else(|| anyhow!("no amount control"))?;
        session.controls.set(index, None, &[0.6]);
    }
    let time = |rig: &Rig<'_>| rig.active.as_ref().map_or(0.0, |s| s.time());
    let before = time(&rig);

    // A save that changes another cue (the LIST pane's edits go the same
    // way, D213).
    show.replace_playlist(edited(0.2, "tint"), &library.library);
    rig.load(&mut show)?;
    rig.frame(&mut show);
    assert_eq!(show.clock().current(), Some(0), "still on the first cue");
    assert!(
        time(&rig) >= before,
        "the live scene's clock runs on through the save: {before} then {}",
        time(&rig)
    );
    let nudged = amount(&rig)?;
    assert!(
        (nudged - 0.6).abs() < 1e-4,
        "the operator's change survives a save that did not touch the live cue: {nudged}"
    );

    // A save that changes the live cue itself applies it.
    show.replace_playlist(edited(0.9, "tint"), &library.library);
    rig.load(&mut show)?;
    rig.frame(&mut show);
    let edited_amount = amount(&rig)?;
    assert!(
        (edited_amount - 0.9).abs() < 1e-4,
        "an edit to the live cue takes effect: {edited_amount}"
    );
    show.finish();
    Ok(())
}

#[test]
fn a_show_that_ends_a_second_time_clears_its_resume_point_again() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let library = library_of("ends-twice", &[("red", "{}", RED_SHADER)])?;
    let file = library.root.join("resume.toml");
    let mut rig = Rig::new(&headless)?;
    let mut playlist = Playlist::new("twice");
    let mut section = Section::new("all");
    section.cues.push(cue_of("red", 3.0, TransitionSpec::cut()));
    section.cues.push(cue_of("red", 3.0, TransitionSpec::cut()));
    playlist.sections.push(section);
    let writer = strobe_app::show::resume::ResumeWriter::start(file.clone());
    let mut show = Show::new(None, playlist, &library.library, None, Some(writer));
    rig.load(&mut show)?;
    for _ in 0..(7 * 60) {
        rig.frame(&mut show);
    }
    assert!(show.ended(), "the playlist ended");
    // The operator brings it back and lets it run out again; on the way the
    // position is saved (every five seconds).
    show.command(Command::JumpTo(0));
    for _ in 0..(7 * 60) {
        rig.frame(&mut show);
    }
    assert!(show.ended(), "the playlist ended again");
    show.finish();
    assert!(
        !file.exists(),
        "an ended show leaves no resume point, the second time too"
    );
    Ok(())
}

#[test]
fn a_slow_scene_does_not_stretch_its_cue_past_real_time() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let library = library_of(
        "slow",
        &[("red", "{}", RED_SHADER), ("green", "{}", GREEN_SHADER)],
    )?;
    let mut rig = Rig::new(&headless)?;
    let mut playlist = Playlist::new("slow");
    let mut section = Section::new("all");
    section.cues.push(cue_of("red", 2.0, TransitionSpec::cut()));
    section
        .cues
        .push(cue_of("green", 2.0, TransitionSpec::cut()));
    playlist.sections.push(section);
    let mut show = Show::new(None, playlist, &library.library, None, None);
    rig.load(&mut show)?;
    // A scene at 2.5 fps: 0.4 s between frames, which the app clamps to
    // 0.1 s for the scene's own clock (`stats::MAX_DT`). The mini PC's
    // 10-volume-clouds played a 2-minute cue for 490 s that way.
    let (scene_dt, wall_dt) = (0.1, 0.4);
    for _ in 0..6 {
        let _ = show.frame(&mut rig.env(), scene_dt, wall_dt);
    }
    assert_eq!(
        show.clock().current(),
        Some(1),
        "2.4 s of real time is past the first cue's 2 s"
    );
    let scene_time = rig.active.as_ref().map_or(0.0, |s| s.time());
    assert!(
        scene_time < 1.0,
        "the scene's own clock still advances by the clamped dt (real time would be 2.4 s): {scene_time}"
    );
    show.finish();
    Ok(())
}
