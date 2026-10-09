// ============================================================================
// Module:       example (integration test)
// Description:  The shipped playlists/example.toml parses, refers only to
//               scenes that exist in scenes/, round-trips and plays.
//
// Dependencies: strobe_playlist (public API only), anyhow, std::fs
// ============================================================================

//! The example playlist is documentation, so it is tested like code: a
//! playlist that stopped parsing, named a scene that was renamed, or no
//! longer survived a write/read cycle would mislead everyone who copied it.

use anyhow::{Context as _, Result, anyhow};
use std::path::PathBuf;
use strobe_playlist::{
    BeatInfo, ClockConfig, ClockState, LoopMode, Playlist, PlaylistClock, SceneRef, Timeline,
    TimelineConfig, distinct_scenes, file, parse::parse, write::to_toml,
};

/// The workspace root, from this crate's manifest directory.
fn workspace() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| anyhow!("{} has no workspace root above it", manifest.display()))
}

fn example() -> Result<Playlist> {
    let path = workspace()?.join("playlists").join("example.toml");
    file::load(&path).with_context(|| format!("loading {}", path.display()))
}

#[test]
fn the_example_playlist_parses_and_has_the_documented_shape() -> Result<()> {
    let playlist = example()?;
    assert_eq!(playlist.loop_mode, LoopMode::Loop, "loops");
    assert_eq!(playlist.sections.len(), 3, "three sections");
    assert_eq!(playlist.cue_count(), 12, "twelve cues");
    assert!(playlist.anchor.is_some(), "anchored to a time of day");
    assert_eq!(playlist.seed(), 1337, "seeded");
    Ok(())
}

#[test]
fn every_scene_it_names_exists_in_the_scene_library() -> Result<()> {
    let scenes = workspace()?.join("scenes");
    let playlist = example()?;
    let mut checked = 0;
    for scene in distinct_scenes(&playlist) {
        let SceneRef::Library(name) = &scene else {
            return Err(anyhow!(
                "the example should use library names, found {scene}"
            ));
        };
        let folder = scenes.join(format!("{name}.synScene"));
        assert!(
            folder.join("scene.json").is_file(),
            "the example refers to `{name}`, but {} has no scene.json",
            folder.display()
        );
        checked += 1;
    }
    assert_eq!(
        checked, 10,
        "ten distinct scenes (one is used twice, one is disabled)"
    );
    Ok(())
}

#[test]
fn it_survives_a_write_and_a_read() -> Result<()> {
    let playlist = example()?;
    let text = to_toml(&playlist);
    let back = parse(&text).map_err(|e| anyhow!("{e}\n{text}"))?;
    assert_eq!(
        back, playlist,
        "the example changed through a write/read cycle"
    );
    assert_eq!(to_toml(&back), text, "and writing is stable");
    Ok(())
}

#[test]
fn it_lays_out_on_the_timeline_and_plays() -> Result<()> {
    let playlist = example()?;
    let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
    assert_eq!(timeline.len(), 11, "the disabled cue is not scheduled");
    assert!(
        timeline.total_runtime() > 5400.0,
        "the 1h30m cue alone is that long"
    );
    assert!(
        timeline.has_open_ended(),
        "the last cue is manual, so the runtime is nominal"
    );
    assert_eq!(
        timeline.wall_time(0).map(|w| w.to_string()).as_deref(),
        Some("09:00:00"),
        "the first cue starts at the anchor"
    );

    let mut clock = PlaylistClock::new(timeline, ClockConfig::default());
    for _ in 0..60 * 60 {
        clock.tick(1.0 / 60.0, &BeatInfo::SILENT);
        clock.drain_events().for_each(drop);
    }
    assert_eq!(
        clock.state(),
        ClockState::Playing,
        "a minute in, it is playing"
    );
    Ok(())
}
