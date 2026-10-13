// ============================================================================
// Module:       strobe_audio::behaviour
// Description:  End-to-end tests of the analyser on synthetic signals: tempo,
//               beats, band isolation, silence, ranges, frame-rate
//               independence, texture shapes, manual controls.
//
// Dependencies: crate::fixtures (signals), crate::analyzer, crate::offline
// ============================================================================

//! Behaviour tests.
//!
//! These run the whole analyser, not its parts: a synthesised signal goes
//! in through [`Analyzer::push`] and the SSF uniforms come out of
//! [`Analyzer::analyze`], exactly as a render loop would use them.

use anyhow::{Result, anyhow};
use strobe_scene::audio::{SCALARS, silent_value};

use crate::analyzer::Analyzer;
use crate::fixtures::{self, FPS, RATE};
use crate::frame::{AudioFrame, Layout};
use crate::offline;

/// The value of `name` in `frame`, as a test error if there is none.
fn get(frame: &AudioFrame, name: &str) -> Result<f32> {
    frame
        .scalar(name)
        .ok_or_else(|| anyhow!("no scalar called {name}"))
}

/// Beats a kick pattern contains, matching [`fixtures::kicks`].
fn beats_in(bpm: f32, seconds: f32) -> usize {
    let period = 60.0 / bpm;
    let mut count = 0;
    while 0.2 + count as f32 * period < seconds {
        count += 1;
    }
    count
}

/// Rising edges of an impulse trace: frames that reach full height after
/// being well below it.
fn impulses(trace: &[f32]) -> usize {
    trace
        .windows(2)
        .filter(|pair| pair[1] > 0.99 && pair[0] < 0.9)
        .count()
}

/// Runs a 20 s kick pattern and checks tempo, confidence and beat counts.
/// Returns the BPM error so the numbers can be read with `--nocapture`.
fn check_tempo(bpm: f32, accepted: &[f32]) -> Result<f32> {
    let seconds = 20.0;
    let signal = fixtures::kicks(bpm, seconds, RATE);
    let mut analyzer = Analyzer::new(RATE);
    let mut on_beat = Vec::new();
    offline::run(&mut analyzer, &signal, FPS, |_, frame| {
        on_beat.push(frame.scalar("syn_OnBeat").unwrap_or(f32::NAN));
    });
    let frame = analyzer.frame();
    let measured = get(frame, "syn_BPM")?;
    let error = accepted
        .iter()
        .map(|target| (measured - target).abs())
        .fold(f32::MAX, f32::min);
    eprintln!("kick {bpm} BPM: measured {measured:.2}, error {error:.2}");
    assert!(
        error <= 2.0,
        "{bpm} BPM kick measured {measured}, expected one of {accepted:?} within 2"
    );
    let confidence = get(frame, "syn_BPMConfidence")?;
    assert!(
        confidence > 0.5,
        "{bpm} BPM kick confidence {confidence} should exceed 0.5"
    );

    let expected = beats_in(bpm, seconds);
    let fired = impulses(&on_beat);
    assert!(
        fired.abs_diff(expected) <= 1,
        "OnBeat fired {fired} times for {expected} beats at {bpm} BPM"
    );
    let counted = get(frame, "syn_BeatTime")? as usize;
    assert!(
        counted.abs_diff(expected) <= 1,
        "BeatTime counted {counted} for {expected} beats at {bpm} BPM"
    );
    Ok(error)
}

#[test]
fn tempo_128_bpm() -> Result<()> {
    check_tempo(128.0, &[128.0])?;
    Ok(())
}

#[test]
fn tempo_100_bpm() -> Result<()> {
    check_tempo(100.0, &[100.0])?;
    Ok(())
}

#[test]
fn tempo_174_bpm() -> Result<()> {
    // Octave choice: 174 BPM is also 87 BPM at half time. The prior around
    // 120 BPM and the biased autocorrelation are meant to resolve to the
    // faster, true tempo, but half time is musically defensible, so it is
    // accepted here; the report states which one the analyser picks.
    check_tempo(174.0, &[174.0, 87.0])?;
    Ok(())
}

#[test]
fn the_beat_grid_is_phase_locked_to_the_kicks() -> Result<()> {
    // BPMTri is 0 on the beat and 1 half a beat later, so on frames where
    // a kick has just fired it should be near 0.
    let signal = fixtures::kicks(120.0, 20.0, RATE);
    let mut analyzer = Analyzer::new(RATE);
    let mut samples = Vec::new();
    offline::run(&mut analyzer, &signal, FPS, |time, frame| {
        if time > 10.0 && frame.scalar("syn_OnBeat").unwrap_or(0.0) > 0.99 {
            samples.push(frame.scalar("syn_BPMTri").unwrap_or(f32::NAN));
        }
    });
    assert!(samples.len() >= 10, "expected ten or more beats after 10 s");
    for tri in &samples {
        assert!(
            *tri < 0.25 || *tri > 0.75,
            "BPMTri should sit on a beat boundary when a kick fires, got {tri}"
        );
    }
    Ok(())
}

#[test]
fn bass_tone_raises_bass_not_high() -> Result<()> {
    let frame = fixtures::last_frame(&fixtures::tone(60.0, 0.5, 3.0, RATE), RATE, FPS);
    let bass = get(&frame, "syn_BassLevel")?;
    let high = get(&frame, "syn_HighLevel")?;
    assert!(bass > 0.8, "a 60 Hz tone should fill BassLevel, got {bass}");
    assert!(high < 0.01, "and leave HighLevel alone, got {high}");
    Ok(())
}

#[test]
fn ten_khz_tone_raises_high_not_bass() -> Result<()> {
    let frame = fixtures::last_frame(&fixtures::tone(10_000.0, 0.5, 3.0, RATE), RATE, FPS);
    let bass = get(&frame, "syn_BassLevel")?;
    let high = get(&frame, "syn_HighLevel")?;
    let mid = get(&frame, "syn_MidLevel")?;
    assert!(
        high > 0.8,
        "a 10 kHz tone should fill HighLevel, got {high}"
    );
    assert!(bass < 0.01, "and leave BassLevel alone, got {bass}");
    assert!(mid < 0.01, "and MidLevel too, got {mid}");
    Ok(())
}

#[test]
fn silence_gives_the_silent_values_and_never_nan() -> Result<()> {
    let signal = vec![0.0; RATE as usize * 10];
    let frame = fixtures::last_frame(&signal, RATE, FPS);
    for (name, value) in SCALARS.iter().zip(&frame.scalars) {
        assert!(value.is_finite(), "{name} is {value} after 10 s of silence");
        assert!(
            (value - silent_value(name)).abs() < 1e-6,
            "{name} should hold its silent value {} but is {value}",
            silent_value(name)
        );
    }
    for texel in &frame.spectrum {
        assert!(
            texel.iter().all(|v| v.is_finite()),
            "spectrum texels stay finite in silence"
        );
    }
    Ok(())
}

#[test]
fn hostile_input_never_produces_nan() -> Result<()> {
    let mut signal = fixtures::noise(1.0, 2.0, RATE);
    for (i, sample) in signal.iter_mut().enumerate() {
        match i % 1000 {
            0 => *sample = f32::NAN,
            1 => *sample = f32::INFINITY,
            2 => *sample = 1.0e30,
            _ => {}
        }
    }
    let frame = fixtures::last_frame(&signal, RATE, FPS);
    for (name, value) in SCALARS.iter().zip(&frame.scalars) {
        assert!(value.is_finite(), "{name} is {value} after hostile input");
    }
    Ok(())
}

#[test]
fn levels_stay_in_range_under_a_clipped_square_wave() -> Result<()> {
    let signal = fixtures::square(100.0, 6.0, RATE);
    let mut analyzer = Analyzer::new(RATE);
    let mut worst: (f32, &str) = (0.0, "");
    let mut violations = Vec::new();
    offline::run(&mut analyzer, &signal, FPS, |_, frame| {
        for (name, value) in SCALARS.iter().zip(&frame.scalars) {
            let bounded = name.ends_with("Level")
                || name.ends_with("Hits")
                || name.ends_with("Presence")
                || *name == "syn_OnBeat"
                || *name == "syn_BPMConfidence"
                || *name == "syn_FadeInOut"
                || *name == "syn_Intensity";
            if bounded && !(0.0..=1.0).contains(value) {
                violations.push(format!("{name}={value}"));
            }
            if value.abs() > worst.0 {
                worst = (value.abs(), name);
            }
        }
    });
    assert!(
        violations.is_empty(),
        "bounded uniforms left 0..1: {:?}",
        violations.iter().take(5).collect::<Vec<_>>()
    );
    assert!(worst.0.is_finite(), "{} went non-finite", worst.1);
    Ok(())
}

#[test]
fn reactivity_scales_levels_and_stays_clamped() -> Result<()> {
    let signal = fixtures::tone(60.0, 0.05, 3.0, RATE);
    let level_with = |gain: f32| -> Result<f32> {
        let mut analyzer = Analyzer::new(RATE);
        analyzer.set_reactivity(gain);
        offline::run(&mut analyzer, &signal, FPS, |_, _| {});
        get(analyzer.frame(), "syn_BassHits")?;
        get(analyzer.frame(), "syn_Level")
    };
    let half = level_with(0.5)?;
    let full = level_with(1.0)?;
    let boosted = level_with(4.0)?;
    assert!(
        half < full,
        "half reactivity ({half}) is below neutral ({full})"
    );
    assert!(boosted <= 1.0, "gain is clamped to 1.0, got {boosted}");
    Ok(())
}

#[test]
fn audio_speed_scales_the_clocks_not_the_beat_counter() -> Result<()> {
    let signal = fixtures::kicks(120.0, 8.0, RATE);
    let run = |speed: f32| -> Result<(f32, f32)> {
        let mut analyzer = Analyzer::new(RATE);
        analyzer.set_audio_speed(speed);
        offline::run(&mut analyzer, &signal, FPS, |_, _| {});
        let frame = analyzer.frame();
        Ok((get(frame, "syn_Time")?, get(frame, "syn_BeatTime")?))
    };
    let (slow_time, slow_beats) = run(1.0)?;
    let (fast_time, fast_beats) = run(2.0)?;
    assert!(
        (fast_time / slow_time - 2.0).abs() < 0.05,
        "doubling audio speed doubles the clock: {slow_time} vs {fast_time}"
    );
    assert_eq!(slow_beats, fast_beats, "beats are counted, not scaled");
    Ok(())
}

#[test]
fn clocks_advance_faster_when_louder() -> Result<()> {
    // Levels are normalised to the track's own recent peak, so loudness is
    // relative: the same tone drops to a tenth of its amplitude halfway
    // through, and its clocks must slow down accordingly.
    let mut signal = fixtures::tone(440.0, 0.5, 2.0, RATE);
    signal.extend(fixtures::tone(440.0, 0.05, 2.0, RATE));
    let mut analyzer = Analyzer::new(RATE);
    let mut mid = Vec::new();
    let mut curved = Vec::new();
    offline::run(&mut analyzer, &signal, FPS, |time, frame| {
        mid.push((time, frame.scalar("syn_MidTime").unwrap_or(f32::NAN)));
        curved.push((time, frame.scalar("syn_CurvedTime").unwrap_or(f32::NAN)));
    });
    let gain = |series: &[(f32, f32)], from: f32, to: f32| -> f32 {
        let at = |t: f32| {
            series
                .iter()
                .min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs()))
                .map_or(0.0, |p| p.1)
        };
        at(to) - at(from)
    };
    let loud = gain(&mid, 1.0, 2.0);
    let quiet = gain(&mid, 3.0, 4.0);
    assert!(
        loud > 3.0 * quiet,
        "MidTime advanced {loud} in the loud second and {quiet} in the quiet one"
    );
    let loud = gain(&curved, 1.0, 2.0);
    let quiet = gain(&curved, 3.0, 4.0);
    assert!(
        loud > 20.0 * quiet.max(1e-6),
        "CurvedTime is convex: {loud} loud against {quiet} quiet"
    );
    Ok(())
}

#[test]
fn smoothed_levels_match_at_30_and_60_calls_a_second() -> Result<()> {
    let signal = fixtures::kicks(128.0, 2.0, RATE);
    let at = |fps: f32| -> Result<Vec<f32>> {
        let frame = fixtures::last_frame(&signal, RATE, fps);
        [
            "syn_Level",
            "syn_BassLevel",
            "syn_MidLevel",
            "syn_HighLevel",
        ]
        .iter()
        .map(|name| get(&frame, name))
        .collect()
    };
    let (slow, fast) = (at(30.0)?, at(60.0)?);
    for ((a, b), name) in slow.iter().zip(&fast).zip(["Level", "Bass", "Mid", "High"]) {
        assert!(
            (a - b).abs() < 0.12,
            "{name} differs between 30 fps ({a}) and 60 fps ({b})"
        );
    }
    // A steady tone removes the one legitimate difference: where in a
    // decaying kick the final frame happens to land.
    let steady = fixtures::tone(200.0, 0.3, 2.0, RATE);
    let a = get(&fixtures::last_frame(&steady, RATE, 30.0), "syn_MidLevel")?;
    let b = get(&fixtures::last_frame(&steady, RATE, 60.0), "syn_MidLevel")?;
    assert!(
        (a - b).abs() < 0.01,
        "a steady tone settles identically at 30 ({a}) and 60 ({b}) fps"
    );
    Ok(())
}

#[test]
fn clocks_are_frame_rate_independent() -> Result<()> {
    let signal = fixtures::tone(200.0, 0.3, 4.0, RATE);
    let a = get(&fixtures::last_frame(&signal, RATE, 30.0), "syn_MidTime")?;
    let b = get(&fixtures::last_frame(&signal, RATE, 60.0), "syn_MidTime")?;
    assert!(
        (a / b - 1.0).abs() < 0.03,
        "a clock integrates real time at any call rate: {a} at 30, {b} at 60"
    );
    Ok(())
}

#[test]
fn the_same_input_gives_bit_identical_output() -> Result<()> {
    let signal = fixtures::kicks(128.0, 6.0, RATE);
    let a = fixtures::last_frame(&signal, RATE, FPS);
    let b = fixtures::last_frame(&signal, RATE, FPS);
    assert_eq!(a, b, "the analyser is deterministic");
    Ok(())
}

#[test]
fn texture_shapes_and_ranges() -> Result<()> {
    let signal = fixtures::tone(1000.0, 0.5, 2.0, RATE);
    let frame = fixtures::last_frame(&signal, RATE, FPS);
    assert_eq!(frame.scalars.len(), SCALARS.len(), "one scalar per name");
    assert_eq!(frame.spectrum.len(), 256, "syn_Spectrum is 256 wide");
    assert_eq!(frame.level_trail.len(), 256, "syn_LevelTrail is 256 wide");
    assert_eq!(frame.shadertoy.len(), 1024, "Shadertoy texture is 512x2");
    for (i, texel) in frame.spectrum.iter().enumerate() {
        assert!(
            texel.iter().all(|v| (0.0..=1.0).contains(v)),
            "spectrum texel {i} out of range: {texel:?}"
        );
    }
    for texel in &frame.level_trail {
        assert!(
            texel.iter().all(|v| (0.0..=1.0).contains(v)),
            "trail texel out of range: {texel:?}"
        );
    }

    // The 1 kHz peak: raw is linear in frequency, juiced is logarithmic.
    let peak_of = |channel: usize| {
        frame
            .spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1[channel].total_cmp(&b.1[channel]))
            .map_or(0, |(i, _)| i)
    };
    let raw_peak = peak_of(0);
    let expected_raw = (1000.0 / 24_000.0 * 256.0) as usize;
    assert!(
        raw_peak.abs_diff(expected_raw) <= 1,
        "raw FFT peak at texel {raw_peak}, expected near {expected_raw}"
    );
    let juiced_peak = peak_of(1);
    let expected_juiced = ((1000.0_f32 / 30.0).ln() / (16_000.0_f32 / 30.0).ln() * 256.0) as usize;
    assert!(
        juiced_peak.abs_diff(expected_juiced) <= 4,
        "juiced peak at texel {juiced_peak}, expected near {expected_juiced}"
    );

    // Shadertoy row 0: 1 kHz of 24 kHz across 512 columns.
    let (spectrum_row, wave_row) = frame.shadertoy.split_at(512);
    let toy_peak = spectrum_row
        .iter()
        .enumerate()
        .max_by_key(|(_, v)| **v)
        .map_or(0, |(i, _)| i);
    assert!(
        toy_peak.abs_diff((1000.0 / 24_000.0 * 512.0) as usize) <= 2,
        "Shadertoy spectrum peak at column {toy_peak}"
    );
    let peak_byte = spectrum_row.iter().copied().max().unwrap_or(0);
    assert!(peak_byte > 150, "a -6 dBFS tone is bright, got {peak_byte}");
    let wave_swing = wave_row.iter().copied().max().unwrap_or(128) as i32
        - wave_row.iter().copied().min().unwrap_or(128) as i32;
    assert!(
        wave_swing > 100,
        "the waveform row swings, got {wave_swing}"
    );
    Ok(())
}

#[test]
fn a_silent_frame_has_a_dark_spectrum_and_a_centred_waveform() -> Result<()> {
    let frame = fixtures::last_frame(&vec![0.0; RATE as usize], RATE, FPS);
    assert!(
        frame.spectrum.iter().all(|t| t[0] == 0.0 && t[1] == 0.0),
        "no energy in silence"
    );
    assert!(
        frame.spectrum.iter().all(|t| (t[3] - 0.5).abs() < 1e-6),
        "waveform sits on the centre line"
    );
    assert!(
        frame.shadertoy[512..].iter().all(|b| *b == 128),
        "Shadertoy waveform row is 128 at silence"
    );
    Ok(())
}

#[test]
fn texture_widths_follow_the_layout() -> Result<()> {
    let layout = Layout {
        spectrum_len: 64,
        trail_len: 100,
        trail_seconds: 2.0,
        shadertoy_width: 200,
    };
    let mut analyzer = Analyzer::with_layout(RATE, layout);
    let signal = fixtures::tone(500.0, 0.4, 1.0, RATE);
    offline::run(&mut analyzer, &signal, FPS, |_, _| {});
    let frame = analyzer.frame();
    assert_eq!(frame.spectrum.len(), 64, "spectrum width");
    assert_eq!(frame.level_trail.len(), 100, "trail width");
    assert_eq!(frame.shadertoy.len(), 400, "Shadertoy width x 2");
    assert!(
        frame.spectrum.iter().any(|t| t[1] > 0.3),
        "a narrower texture still carries the tone"
    );
    Ok(())
}

#[test]
fn the_level_trail_runs_newest_first() -> Result<()> {
    let mut analyzer = Analyzer::new(RATE);
    let quiet = vec![0.0; RATE as usize];
    let loud = fixtures::tone(200.0, 0.5, 1.0, RATE);
    offline::run(&mut analyzer, &quiet, FPS, |_, _| {});
    offline::run(&mut analyzer, &loud, FPS, |_, _| {});
    let trail = &analyzer.frame().level_trail;
    let newest = trail.first().map_or(0.0, |t| t[0]);
    let oldest = trail.last().map_or(1.0, |t| t[0]);
    assert!(newest > 0.5, "index 0 is now, loud: {newest}");
    assert!(
        oldest < 0.1,
        "the far end is a second-plus ago, quiet: {oldest}"
    );
    Ok(())
}

#[test]
fn frame_storage_is_stable() -> Result<()> {
    // analyze() allocates nothing by construction; the observable
    // consequence is that the output buffers never move or grow.
    let mut analyzer = Analyzer::new(RATE);
    let before = {
        let f = analyzer.frame();
        (
            f.scalars.as_ptr(),
            f.scalars.capacity(),
            f.spectrum.as_ptr(),
            f.spectrum.capacity(),
            f.level_trail.as_ptr(),
            f.shadertoy.as_ptr(),
        )
    };
    let signal = fixtures::kicks(128.0, 5.0, RATE);
    offline::run(&mut analyzer, &signal, FPS, |_, _| {});
    let f = analyzer.frame();
    let after = (
        f.scalars.as_ptr(),
        f.scalars.capacity(),
        f.spectrum.as_ptr(),
        f.spectrum.capacity(),
        f.level_trail.as_ptr(),
        f.shadertoy.as_ptr(),
    );
    assert_eq!(
        before, after,
        "frame buffers were reallocated during analysis"
    );
    Ok(())
}

#[test]
fn a_late_caller_does_not_wedge_the_analyser() -> Result<()> {
    // Pushing far more than the history holds between two analyze calls
    // must skip ahead, not panic or read stale data.
    let mut analyzer = Analyzer::new(RATE);
    analyzer.push(&fixtures::tone(60.0, 0.5, 5.0, RATE));
    analyzer.analyze(0.016);
    analyzer.analyze(0.016);
    let bass = get(analyzer.frame(), "syn_BassLevel")?;
    assert!(
        bass.is_finite(),
        "level stays finite after a flood, got {bass}"
    );
    Ok(())
}

#[test]
fn a_stalled_input_decays_to_silence() -> Result<()> {
    let mut analyzer = Analyzer::new(RATE);
    offline::run(
        &mut analyzer,
        &fixtures::tone(200.0, 0.5, 2.0, RATE),
        FPS,
        |_, _| {},
    );
    assert!(
        get(analyzer.frame(), "syn_Level")? > 0.5,
        "loud before the stall"
    );
    for _ in 0..180 {
        analyzer.analyze(1.0 / 60.0);
    }
    let level = get(analyzer.frame(), "syn_Level")?;
    assert!(
        level < 0.01,
        "no new samples for 3 s means silence, got {level}"
    );
    Ok(())
}

#[test]
fn tap_tempo_and_nudge_override_the_estimate() -> Result<()> {
    let mut analyzer = Analyzer::new(RATE);
    analyzer.tap(0.0);
    analyzer.tap(0.5);
    analyzer.tap(1.0);
    assert!(analyzer.manual_tempo(), "tapping takes manual control");
    // Let the published tempo glide to the target.
    for _ in 0..240 {
        analyzer.analyze(1.0 / 60.0);
    }
    let bpm = get(analyzer.frame(), "syn_BPM")?;
    assert!(
        (bpm - 120.0).abs() < 0.5,
        "taps 0.5 s apart are 120 BPM, got {bpm}"
    );

    analyzer.nudge_bpm(10.0);
    for _ in 0..240 {
        analyzer.analyze(1.0 / 60.0);
    }
    let nudged = get(analyzer.frame(), "syn_BPM")?;
    assert!(
        (nudged - 130.0).abs() < 0.5,
        "nudged up by 10, got {nudged}"
    );

    analyzer.clear_manual_tempo();
    assert!(!analyzer.manual_tempo(), "clearing returns control");
    Ok(())
}

#[test]
fn noise_does_not_look_like_a_steady_tempo() -> Result<()> {
    let frame = fixtures::last_frame(&fixtures::noise(0.3, 15.0, RATE), RATE, FPS);
    let confidence = get(&frame, "syn_BPMConfidence")?;
    assert!(
        confidence < 0.5,
        "white noise has no tempo, but confidence is {confidence}"
    );
    Ok(())
}

#[test]
fn works_at_other_sample_rates() -> Result<()> {
    for rate in [22_050_u32, 44_100, 96_000] {
        let signal = fixtures::kicks(120.0, 12.0, rate);
        let frame = fixtures::last_frame(&signal, rate, FPS);
        let bpm = get(&frame, "syn_BPM")?;
        assert!(
            (bpm - 120.0).abs() < 3.0,
            "120 BPM kick at {rate} Hz measured {bpm}"
        );
    }
    Ok(())
}

#[test]
fn a_louder_kick_on_one_is_found_as_the_downbeat_at_every_phase() -> Result<()> {
    let seconds = 40.0;
    for phase in 0..4_usize {
        // A kick every beat at 120 BPM, and a second kick on every fourth
        // beat starting `phase` beats in.
        let mut signal = fixtures::kicks(120.0, seconds, RATE);
        let delay = phase * RATE as usize / 2;
        let accents = fixtures::kicks(30.0, seconds, RATE);
        for (slot, accent) in signal.iter_mut().skip(delay).zip(accents) {
            *slot += accent;
        }
        let mut analyzer = Analyzer::new(RATE);
        // The grid beat a fifth of a beat after each accented kick of the
        // last ten seconds.
        let mut downbeats = Vec::new();
        offline::run(&mut analyzer, &signal, FPS, |time, frame| {
            let since = time - 0.2 - 0.5 * phase as f32 - 0.1;
            let bar = since / 2.0;
            if time > seconds - 10.0 && bar.fract() < 1.0 / FPS / 2.0 && since > 0.0 {
                let beat = frame.scalar("syn_BeatTime").unwrap_or(f32::NAN);
                downbeats.push((beat.floor() as i64, frame.downbeat));
            }
        });
        assert!(downbeats.len() >= 4, "phase {phase}: sampled {downbeats:?}");
        for (beat, [offset, confidence]) in downbeats {
            assert!(
                confidence > 0.5,
                "phase {phase}: sure by now, got {confidence}"
            );
            assert_eq!(
                (beat - offset as i64).rem_euclid(4),
                0,
                "phase {phase}: the accented kick is on grid beat {beat}, offset {offset}"
            );
        }
    }
    Ok(())
}

#[test]
fn even_kicks_report_no_downbeat_at_any_tempo() {
    for bpm in [97.0, 120.0, 123.5, 128.0, 140.0, 174.0] {
        let signal = fixtures::kicks(bpm, 40.0, RATE);
        let frame = fixtures::last_frame(&signal, RATE, FPS);
        assert!(
            frame.downbeat[1] < 0.1,
            "{bpm} BPM, every beat alike: {:?}",
            frame.downbeat
        );
    }
}
