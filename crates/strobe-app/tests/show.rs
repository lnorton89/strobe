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
            eprintln!("skipping: no headless {api:?} context ({error})");
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
        let _ = show.frame(&mut self.env(), DT);
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
