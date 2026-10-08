// ============================================================================
// Module:       wav-analyze (example)
// Description:  Offline tuning harness: reads a WAV file, runs the analyser
//               at a fixed frame rate, and writes every audio uniform per
//               frame as CSV.
//
// Dependencies: hound (WAV decoding), anyhow; strobe-audio (Analyzer, offline)
// ============================================================================

//! `wav-analyze`: WAV in, CSV of every SSF audio uniform out.
//!
//! ```text
//! cargo run --release -p strobe-audio --example wav-analyze -- track.wav out.csv [--fps 60]
//! ```
//!
//! One row per simulated render frame, columns `time` then every name in
//! `strobe_scene::audio::SCALARS` in order. Plot the CSV in any tool to see
//! what a scene would see: whether `syn_BPM` settles, whether `syn_OnBeat`
//! lands on the kick, how fast `syn_BassLevel` falls. The analysis path is
//! exactly the one the live renderer uses ([`strobe_audio::offline::run`]),
//! so a number tuned here is the number a scene gets.
//!
//! Multi-channel files are averaged to mono, as the live capture does.
//! Passing `-` as the output writes the CSV to standard output.

use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::{Context, Result, bail};
use hound::{SampleFormat, WavReader};
use strobe_audio::{Analyzer, offline};
use strobe_scene::audio::SCALARS;

/// Frame rate used when `--fps` is not given.
const DEFAULT_FPS: f32 = 60.0;

/// Reads a WAV file as mono `f32` in -1..1, returning it with its rate.
fn read_mono(path: &str) -> Result<(Vec<f32>, u32)> {
    let mut reader = WavReader::open(path).with_context(|| format!("opening {path}"))?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));
    let interleaved: Vec<f32> = match spec.sample_format {
        SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .context("decoding float samples")?,
        SampleFormat::Int => {
            let scale = 1.0 / (1_i64 << (spec.bits_per_sample.saturating_sub(1))) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .context("decoding integer samples")?
        }
    };
    let mono = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok((mono, spec.sample_rate))
}

fn main() -> Result<()> {
    let mut input = None;
    let mut output = None;
    let mut fps = DEFAULT_FPS;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--fps" {
            let value = args.next().context("--fps needs a number")?;
            fps = value.parse().context("--fps must be a number")?;
        } else if input.is_none() {
            input = Some(arg);
        } else if output.is_none() {
            output = Some(arg);
        } else {
            bail!("unexpected argument `{arg}`");
        }
    }
    let (Some(input), Some(output)) = (input, output) else {
        bail!("usage: wav-analyze <input.wav> <output.csv|-> [--fps N]");
    };
    if !fps.is_finite() || fps <= 0.0 {
        bail!("--fps must be positive");
    }

    let (samples, rate) = read_mono(&input)?;
    eprintln!(
        "{input}: {} samples at {rate} Hz ({:.1} s), analysing at {fps} fps",
        samples.len(),
        samples.len() as f32 / rate as f32
    );

    let sink: Box<dyn Write> = if output == "-" {
        Box::new(std::io::stdout().lock())
    } else {
        Box::new(File::create(&output).with_context(|| format!("creating {output}"))?)
    };
    let mut out = BufWriter::new(sink);
    write!(out, "time")?;
    for name in SCALARS {
        write!(out, ",{name}")?;
    }
    writeln!(out)?;

    let mut analyzer = Analyzer::new(rate);
    let mut failure = None;
    offline::run(&mut analyzer, &samples, fps, |time, frame| {
        if failure.is_some() {
            return;
        }
        let mut row = format!("{time:.4}");
        for value in &frame.scalars {
            row.push_str(&format!(",{value:.5}"));
        }
        if let Err(error) = writeln!(out, "{row}") {
            failure = Some(error);
        }
    });
    if let Some(error) = failure {
        return Err(error).context("writing CSV");
    }
    out.flush().context("flushing CSV")?;
    Ok(())
}
