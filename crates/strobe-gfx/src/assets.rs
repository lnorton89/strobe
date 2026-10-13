// ============================================================================
// Module:       strobe_gfx::assets
// Description:  Decoding a scene's picture files on the CPU: cube map faces,
//               and the frames of a syn_Media still or image sequence, with
//               the background thread that decodes a sequence.
//
// Dependencies: image; strobe-scene (CubemapDef, MediaDef)
// ============================================================================

//! Scene assets, decoded without a GL context.
//!
//! Everything here is plain CPU work, so it can run on a thread that has no
//! context and be tested without one. The player turns the results into
//! textures on the render thread.
//!
//! # Cube maps (D185)
//!
//! A scene's `CUBEMAPS` entry lists paths relative to its folder. Two
//! shapes are read:
//!
//! - **Six paths**, the faces in GL's order: +X, -X, +Y, -Y, +Z, -Z. Every
//!   face is square and they all have the same size.
//! - **One path** holding all six in a single picture, told apart by its
//!   shape: a **6:1 strip** or **1:6 strip** in the same face order, or a
//!   **4:3 horizontal cross** (`+Y` above `+Z`, with `-X +Z +X -Z` across the
//!   middle row and `-Y` below `+Z`).
//!
//! Faces are used as decoded, top row first, which is what GL's cube
//! targets expect. SSF's own documentation of `CUBEMAPS` was not available
//! to check any of this against (Q22): it is the layout the six-path form
//! implies plus the three single-image layouts people actually ship.
//!
//! # `syn_Media` (D186)
//!
//! A [`MediaRequest`] names a file (a still) or a folder (an image
//! sequence, its PNG and JPEG files in name order). [`spawn_decoder`] reads
//! and decodes it on a thread of its own and sends the frames back one at a
//! time, so the render thread never waits on a disk or a decoder. Frames are
//! flipped upright like scene images unless asked not to, shrunk to
//! [`MAX_MEDIA_EDGE`] on their long side. A sequence must fit
//! [`MAX_MEDIA_BYTES`] of decoded pictures, since video memory on an old
//! laptop is shared with the system: a long one is shrunk as a whole, every
//! frame alike, until it fits ([`sequence_edge`], D225), so the loop plays
//! complete at a lower resolution. Only below [`MIN_SEQUENCE_EDGE`] is it
//! cut short instead.

use image::RgbaImage;
use image::imageops::{self, FilterType};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use strobe_scene::scene::{
    CubemapDef, DEFAULT_MEDIA_FPS, Filter, MediaDef, Wrap, is_folder_relative,
};

/// The largest cube face edge accepted, in texels.
pub const MAX_CUBE_EDGE: u32 = 2048;

/// The longest side a media frame keeps; bigger pictures are shrunk to fit.
pub const MAX_MEDIA_EDGE: u32 = 2048;

/// The most decoded picture data one sequence may hold, in bytes.
pub const MAX_MEDIA_BYTES: usize = 192 * 1024 * 1024;

/// The smallest long side a sequence is shrunk to so that it fits
/// [`MAX_MEDIA_BYTES`]; below this a long sequence is cut short instead,
/// because a postage stamp is worse than a shorter loop.
pub const MIN_SEQUENCE_EDGE: u32 = 256;

/// How many decoded frames may wait for the render thread to take them.
const IN_FLIGHT: usize = 3;

/// File extensions a sequence reads, lower case.
const IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// A scene asset that did not load, and what the player did about it. The
/// scene keeps running; the app reports it through the failure path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetProblem {
    /// What it was for: an image or cube map sampler's name, or `syn_Media`.
    pub asset: String,
    /// The file or folder, as written in the scene or given by the user.
    pub path: String,
    /// What was wrong.
    pub message: String,
}

/// Six decoded cube faces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CubeFaces {
    /// The edge of every face, in texels.
    pub size: u32,
    /// RGBA8, +X, -X, +Y, -Y, +Z, -Z.
    pub faces: [Vec<u8>; 6],
}

fn open(path: &Path) -> Result<RgbaImage, String> {
    decode_file(path)
        .map(image::DynamicImage::into_rgba8)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Decodes an image by its content, not its extension: a real Synesthesia
/// scene ships a PNG named `skytex.jpg`, and Synesthesia shows it (D196).
pub(crate) fn decode_file(path: &Path) -> Result<image::DynamicImage, String> {
    image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())
}

/// Reads the cube map `def` declares, relative to the scene folder `dir`.
pub fn load_cubemap(dir: &Path, def: &CubemapDef) -> Result<CubeFaces, String> {
    if let Some(bad) = def.paths.iter().find(|path| !is_folder_relative(path)) {
        return Err(format!(
            "`{bad}` must be a relative path inside the scene folder"
        ));
    }
    match def.paths.as_slice() {
        [single] => split_single(open(&dir.join(single))?),
        paths if paths.len() == 6 => {
            let mut images = Vec::with_capacity(6);
            for path in paths {
                images.push(open(&dir.join(path))?);
            }
            join_faces(images)
        }
        other => Err(format!(
            "a cube map needs six face paths or one strip/cross picture, got {} path(s)",
            other.len()
        )),
    }
}

fn join_faces(images: Vec<RgbaImage>) -> Result<CubeFaces, String> {
    let first = images.first().map(RgbaImage::dimensions);
    let Some((width, height)) = first else {
        return Err("no faces".to_string());
    };
    if width != height {
        return Err(format!(
            "faces must be square, the first is {width}x{height}"
        ));
    }
    if width > MAX_CUBE_EDGE {
        return Err(format!(
            "faces are {width} texels wide, the most accepted is {MAX_CUBE_EDGE}"
        ));
    }
    if let Some((index, other)) = images
        .iter()
        .enumerate()
        .find(|(_, image)| image.dimensions() != (width, height))
    {
        let (w, h) = other.dimensions();
        return Err(format!(
            "face {index} is {w}x{h} but the first is {width}x{height}; all six must match"
        ));
    }
    let mut faces: [Vec<u8>; 6] = Default::default();
    for (slot, image) in faces.iter_mut().zip(images) {
        *slot = image.into_raw();
    }
    Ok(CubeFaces { size: width, faces })
}

/// Cuts a strip or a horizontal cross into six faces.
fn split_single(image: RgbaImage) -> Result<CubeFaces, String> {
    let (w, h) = image.dimensions();
    // (column, row) of each face, in +X, -X, +Y, -Y, +Z, -Z order.
    let (edge, cells): (u32, [(u32, u32); 6]) = if w == h * 6 {
        (h, [(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (5, 0)])
    } else if h == w * 6 {
        (w, [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 5)])
    } else if w * 3 == h * 4 && w % 4 == 0 {
        (w / 4, [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)])
    } else {
        return Err(format!(
            "a single cube picture must be a 6:1 or 1:6 strip or a 4:3 cross, this is {w}x{h}"
        ));
    };
    let faces: Vec<RgbaImage> = cells
        .iter()
        .map(|&(column, row)| {
            imageops::crop_imm(&image, column * edge, row * edge, edge, edge).to_image()
        })
        .collect();
    join_faces(faces)
}

/// What to show behind `syn_Media`, and how.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaRequest {
    /// A file (a still) or a folder (a sequence).
    pub path: PathBuf,
    /// Sequence rate, frames per second.
    pub fps: f32,
    /// Whether a sequence repeats.
    pub looping: bool,
    /// Sampler wrap mode.
    pub wrap: Wrap,
    /// Sampler filter mode.
    pub filter: Filter,
    /// Flip upright on load.
    pub vflip: bool,
}

impl MediaRequest {
    /// The scene's own media, its path resolved against the scene folder.
    pub fn from_scene(dir: &Path, def: &MediaDef) -> Self {
        Self {
            path: dir.join(&def.path),
            fps: def.fps,
            looping: def.looping,
            wrap: def.wrap,
            filter: def.filter,
            vflip: def.vflip,
        }
    }

    /// A file or folder the operator chose, with the defaults a scene's
    /// `MEDIA` key gets.
    pub fn for_path(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            fps: DEFAULT_MEDIA_FPS,
            looping: true,
            wrap: Wrap::Clamp,
            filter: Filter::Linear,
            vflip: true,
        }
    }
}

/// One decoded media picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// RGBA8, top row first (already flipped upright if asked).
    pub rgba: Vec<u8>,
}

/// What the decoder thread sends.
#[derive(Debug)]
pub enum MediaMessage {
    /// The next picture of the sequence.
    Frame(Frame),
    /// Everything that will be sent has been; the sequence is complete.
    Done,
    /// Nothing more can be loaded; why. Frames already sent stay valid.
    Failed(String),
}

/// The pictures `path` names, in the order they play: the file itself, or
/// the PNG and JPEG files in a folder sorted by name (case-insensitively,
/// so `Frame2` precedes `frame10` only if padded, as sequences are).
pub fn list_sources(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let entries = std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate.is_file()
                && candidate
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
                    })
        })
        .collect();
    files.sort_by_key(|file| file.to_string_lossy().to_lowercase());
    if files.is_empty() {
        return Err(format!(
            "{}: no .png or .jpg pictures found",
            path.display()
        ));
    }
    Ok(files)
}

/// Decodes one picture, shrinking it to [`MAX_MEDIA_EDGE`] if it is larger.
pub fn decode_frame(path: &Path, vflip: bool) -> Result<Frame, String> {
    decode_frame_within(path, vflip, MAX_MEDIA_EDGE)
}

/// The longest side every frame of a sequence may keep so that `count`
/// frames like the first (`first`, width and height after the
/// [`MAX_MEDIA_EDGE`] limit) fit `budget` bytes of RGBA, never below
/// [`MIN_SEQUENCE_EDGE`] (D225). The first frame's own long side when they
/// already fit.
pub fn sequence_edge(first: (u32, u32), count: usize, budget: usize) -> u32 {
    let long = first.0.max(first.1).max(1);
    let frame_bytes = u64::from(first.0) * u64::from(first.1) * 4;
    let total = frame_bytes.saturating_mul(count as u64);
    if total <= budget as u64 || total == 0 {
        return long;
    }
    // Area scales with the square of the edge.
    let scale = (budget as f64 / total as f64).sqrt();
    let edge = (f64::from(long) * scale).floor() as u32;
    edge.clamp(MIN_SEQUENCE_EDGE.min(long), long)
}

/// Decodes one picture, shrinking it so its long side is at most `edge`.
fn decode_frame_within(path: &Path, vflip: bool, edge: u32) -> Result<Frame, String> {
    let mut image = open(path)?;
    let (w, h) = image.dimensions();
    if w.max(h) > edge {
        let scale = edge as f32 / w.max(h) as f32;
        let new_w = ((w as f32 * scale).round() as u32).max(1);
        let new_h = ((h as f32 * scale).round() as u32).max(1);
        image = imageops::resize(&image, new_w, new_h, FilterType::Triangle);
    }
    if vflip {
        imageops::flip_vertical_in_place(&mut image);
    }
    let (width, height) = image.dimensions();
    Ok(Frame {
        width,
        height,
        rgba: image.into_raw(),
    })
}

/// Starts decoding `request` on a thread of its own and returns the
/// channel its pictures arrive on. The thread ends when it is done, when
/// the receiver is dropped, or when a picture fails to decode.
pub fn spawn_decoder(request: &MediaRequest) -> Result<Receiver<MediaMessage>, String> {
    let (sender, receiver) = sync_channel(IN_FLIGHT);
    let path = request.path.clone();
    let vflip = request.vflip;
    std::thread::Builder::new()
        .name("strobe-media".to_string())
        .spawn(move || decode_all(&path, vflip, &sender, MAX_MEDIA_BYTES))
        .map_err(|e| format!("could not start the media thread: {e}"))?;
    Ok(receiver)
}

fn decode_all(path: &Path, vflip: bool, sender: &SyncSender<MediaMessage>, budget: usize) {
    let sources = match list_sources(path) {
        Ok(sources) => sources,
        Err(message) => {
            let _ = sender.send(MediaMessage::Failed(message));
            return;
        }
    };
    let total = sources.len();
    let mut bytes = 0_usize;
    let mut edge = MAX_MEDIA_EDGE;
    for (index, source) in sources.iter().enumerate() {
        let decoded = decode_frame_within(source, vflip, edge);
        // The first frame sets the size the whole sequence keeps (D225).
        let decoded = match decoded {
            Ok(first) if index == 0 && total > 1 => {
                let fits = sequence_edge((first.width, first.height), total, budget);
                if fits < first.width.max(first.height) {
                    log::info!(
                        "syn_Media: {total} pictures of {}x{} are more than {} MiB; \
                         every frame is shrunk to {fits} px on its long side so the loop fits",
                        first.width,
                        first.height,
                        budget / (1024 * 1024)
                    );
                    edge = fits;
                    decode_frame_within(source, vflip, edge)
                } else {
                    Ok(first)
                }
            }
            other => other,
        };
        let frame = match decoded {
            Ok(frame) => frame,
            Err(message) => {
                // A still that cannot be read is a failure; a bad frame
                // partway through a sequence ends it there.
                let _ = sender.send(MediaMessage::Failed(message));
                return;
            }
        };
        bytes += frame.rgba.len();
        if index > 0 && bytes > budget {
            let _ = sender.send(MediaMessage::Failed(format!(
                "sequence cut at {index} of {total} pictures: more than {} MiB of decoded frames",
                budget / (1024 * 1024)
            )));
            return;
        }
        if sender.send(MediaMessage::Frame(frame)).is_err() {
            return;
        }
    }
    let _ = sender.send(MediaMessage::Done);
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use image::Rgba;

    /// A scratch folder that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Result<Self> {
            let dir =
                std::env::temp_dir().join(format!("strobe-assets-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_image_is_decoded_by_its_content_not_its_extension() -> Result<()> {
        // D196: a real scene ships a PNG named `skytex.jpg`.
        let scratch = Scratch::new("sniff")?;
        let png = scratch.0.join("png.png");
        RgbaImage::from_pixel(2, 2, Rgba([10, 20, 30, 255])).save(&png)?;
        let misnamed = scratch.0.join("skytex.jpg");
        std::fs::rename(&png, &misnamed)?;
        let frame = decode_frame(&misnamed, false).map_err(|e| anyhow!(e))?;
        assert_eq!(
            (frame.width, frame.height),
            (2, 2),
            "decoded as the PNG it is"
        );
        assert_eq!(
            frame.rgba.get(..3),
            Some(&[10_u8, 20, 30][..]),
            "pixels intact"
        );
        Ok(())
    }

    fn solid(w: u32, h: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(color))
    }

    fn cube(paths: &[&str]) -> CubemapDef {
        CubemapDef {
            name: "env".to_string(),
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
        }
    }

    #[test]
    fn six_faces_load_in_order() -> Result<()> {
        let scratch = Scratch::new("six")?;
        let names = ["px", "nx", "py", "ny", "pz", "nz"];
        for (i, name) in names.iter().enumerate() {
            solid(4, 4, [i as u8 * 40, 0, 0, 255]).save(scratch.0.join(format!("{name}.png")))?;
        }
        let files: Vec<String> = names.iter().map(|n| format!("{n}.png")).collect();
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        let cube = load_cubemap(&scratch.0, &cube(&refs)).map_err(|e| anyhow!(e))?;
        assert_eq!(cube.size, 4, "edge");
        for (i, face) in cube.faces.iter().enumerate() {
            assert_eq!(face[0], i as u8 * 40, "face {i} keeps its slot");
        }
        Ok(())
    }

    #[test]
    fn a_missing_face_a_bad_count_and_mismatched_sizes_are_errors() -> Result<()> {
        let scratch = Scratch::new("bad")?;
        solid(4, 4, [1, 2, 3, 255]).save(scratch.0.join("a.png"))?;
        solid(8, 8, [1, 2, 3, 255]).save(scratch.0.join("big.png"))?;
        solid(4, 2, [1, 2, 3, 255]).save(scratch.0.join("flat.png"))?;
        let missing = load_cubemap(
            &scratch.0,
            &cube(&["a.png", "a.png", "a.png", "a.png", "a.png", "gone.png"]),
        );
        assert!(
            missing.is_err_and(|e| e.contains("gone.png")),
            "names the file"
        );
        let count = load_cubemap(&scratch.0, &cube(&["a.png", "a.png"]));
        assert!(
            count.is_err_and(|e| e.contains("six")),
            "says what is needed"
        );
        let mixed = load_cubemap(
            &scratch.0,
            &cube(&["a.png", "a.png", "a.png", "a.png", "a.png", "big.png"]),
        );
        assert!(mixed.is_err_and(|e| e.contains("face 5")), "names the face");
        let oblong = load_cubemap(&scratch.0, &cube(&["flat.png"; 6]));
        assert!(
            oblong.is_err_and(|e| e.contains("square")),
            "faces are square"
        );
        let outside = load_cubemap(&scratch.0, &cube(&["../a.png"]));
        assert!(outside.is_err(), "a path may not leave the folder");
        Ok(())
    }

    #[test]
    fn a_strip_and_a_cross_are_cut_into_faces() -> Result<()> {
        let scratch = Scratch::new("single")?;
        // A 6:1 strip of six solid cells.
        let mut strip = solid(12, 2, [0; 4]);
        for face in 0..6_u32 {
            for x in 0..2 {
                for y in 0..2 {
                    strip.put_pixel(face * 2 + x, y, Rgba([face as u8 * 30, 9, 9, 255]));
                }
            }
        }
        strip.save(scratch.0.join("strip.png"))?;
        let cut = load_cubemap(&scratch.0, &cube(&["strip.png"])).map_err(|e| anyhow!(e))?;
        for (i, face) in cut.faces.iter().enumerate() {
            assert_eq!(face[0], i as u8 * 30, "strip cell {i}");
        }
        // A 4:3 cross: cell (column,row) painted with a number.
        let mut cross = solid(8, 6, [0; 4]);
        let cells = [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)];
        for (face, (column, row)) in cells.iter().enumerate() {
            for x in 0..2 {
                for y in 0..2 {
                    cross.put_pixel(
                        column * 2 + x,
                        row * 2 + y,
                        Rgba([face as u8 * 30, 9, 9, 255]),
                    );
                }
            }
        }
        cross.save(scratch.0.join("cross.png"))?;
        let cut = load_cubemap(&scratch.0, &cube(&["cross.png"])).map_err(|e| anyhow!(e))?;
        for (i, face) in cut.faces.iter().enumerate() {
            assert_eq!(face[0], i as u8 * 30, "cross cell {i}");
        }
        solid(5, 7, [0; 4]).save(scratch.0.join("odd.png"))?;
        assert!(
            load_cubemap(&scratch.0, &cube(&["odd.png"])).is_err_and(|e| e.contains("6:1")),
            "an unrecognised shape says what is accepted"
        );
        Ok(())
    }

    #[test]
    fn a_folder_is_a_sequence_in_name_order_and_a_file_is_a_still() -> Result<()> {
        let scratch = Scratch::new("seq")?;
        for (name, red) in [("f02.png", 20), ("f01.PNG", 10), ("f03.jpg", 30)] {
            let picture = solid(2, 2, [red, 0, 0, 255]);
            if name.ends_with("jpg") {
                // JPEG has no alpha channel.
                image::DynamicImage::ImageRgba8(picture)
                    .to_rgb8()
                    .save(scratch.0.join(name))?;
            } else {
                picture.save(scratch.0.join(name))?;
            }
        }
        std::fs::write(scratch.0.join("notes.txt"), "not a picture")?;
        let names: Vec<String> = list_sources(&scratch.0)
            .map_err(|e| anyhow!(e))?
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(
            names,
            ["f01.PNG", "f02.png", "f03.jpg"],
            "sorted, pictures only"
        );
        let still = list_sources(&scratch.0.join("f02.png")).map_err(|e| anyhow!(e))?;
        assert_eq!(still.len(), 1, "a file is one picture");
        assert!(
            list_sources(&scratch.0.join("nowhere")).is_err(),
            "missing path"
        );
        let empty = Scratch::new("empty")?;
        assert!(
            list_sources(&empty.0).is_err_and(|e| e.contains("no .png")),
            "empty folder"
        );
        Ok(())
    }

    #[test]
    fn a_frame_is_flipped_upright_and_big_ones_are_shrunk() -> Result<()> {
        let scratch = Scratch::new("frame")?;
        let mut two = solid(2, 2, [0, 0, 0, 255]);
        two.put_pixel(0, 0, Rgba([200, 0, 0, 255]));
        two.save(scratch.0.join("t.png"))?;
        let flipped = decode_frame(&scratch.0.join("t.png"), true).map_err(|e| anyhow!(e))?;
        assert_eq!(
            flipped.rgba[8], 200,
            "the top-left pixel moved to the bottom row"
        );
        let kept = decode_frame(&scratch.0.join("t.png"), false).map_err(|e| anyhow!(e))?;
        assert_eq!(kept.rgba[0], 200, "unflipped keeps it on top");
        solid(MAX_MEDIA_EDGE + 100, 10, [1, 1, 1, 255]).save(scratch.0.join("wide.png"))?;
        let wide = decode_frame(&scratch.0.join("wide.png"), false).map_err(|e| anyhow!(e))?;
        assert_eq!(wide.width, MAX_MEDIA_EDGE, "long side limited");
        assert!(wide.height >= 1, "height kept");
        Ok(())
    }

    #[test]
    fn the_decoder_thread_delivers_frames_then_done() -> Result<()> {
        let scratch = Scratch::new("thread")?;
        for i in 0..5_u8 {
            solid(2, 2, [i, 0, 0, 255]).save(scratch.0.join(format!("{i}.png")))?;
        }
        let receiver =
            spawn_decoder(&MediaRequest::for_path(&scratch.0)).map_err(|e| anyhow!(e))?;
        let mut frames = 0;
        loop {
            match receiver.recv()? {
                MediaMessage::Frame(_) => frames += 1,
                MediaMessage::Done => break,
                MediaMessage::Failed(message) => return Err(anyhow!(message)),
            }
        }
        assert_eq!(frames, 5, "all five arrive through a three-deep channel");
        let bad = spawn_decoder(&MediaRequest::for_path(scratch.0.join("nope")))
            .map_err(|e| anyhow!(e))?;
        assert!(
            matches!(bad.recv()?, MediaMessage::Failed(_)),
            "a missing path is reported, not panicked on"
        );
        Ok(())
    }

    #[test]
    fn a_long_sequence_is_shrunk_to_fit_not_cut() {
        assert_eq!(
            sequence_edge((512, 512), 8, 8 << 20),
            512,
            "it fits as it is"
        );
        assert_eq!(
            sequence_edge((512, 512), 8, 2 << 20),
            256,
            "a quarter of the bytes is half the edge"
        );
        assert_eq!(
            sequence_edge((1024, 512), 100, 1 << 20),
            MIN_SEQUENCE_EDGE,
            "never below the floor"
        );
        assert_eq!(
            sequence_edge((100, 50), 10_000, 1),
            100,
            "a picture already under the floor keeps its size"
        );
    }

    /// Everything the decoder sends for `dir`, at `budget`.
    fn decode_with(dir: &Path, budget: usize) -> Vec<MediaMessage> {
        let (sender, receiver) = sync_channel(64);
        decode_all(dir, false, &sender, budget);
        drop(sender);
        receiver.into_iter().collect()
    }

    #[test]
    fn the_whole_loop_plays_smaller_and_only_a_too_long_one_is_cut() -> Result<()> {
        let scratch = Scratch::new("budget")?;
        for i in 0..8 {
            solid(512, 512, [i * 30, 0, 0, 255]).save(scratch.0.join(format!("f{i}.png")))?;
        }
        let fitted = decode_with(&scratch.0, 2 << 20);
        let frames: Vec<&Frame> = fitted
            .iter()
            .filter_map(|m| match m {
                MediaMessage::Frame(frame) => Some(frame),
                MediaMessage::Done | MediaMessage::Failed(_) => None,
            })
            .collect();
        assert_eq!(frames.len(), 8, "every frame arrives");
        assert!(
            frames.iter().all(|f| f.width == 256 && f.height == 256),
            "all shrunk alike to fit 2 MiB"
        );
        assert!(
            matches!(fitted.last(), Some(MediaMessage::Done)),
            "and it completes"
        );

        let cut = decode_with(&scratch.0, 1 << 20);
        let count = cut
            .iter()
            .filter(|m| matches!(m, MediaMessage::Frame(_)))
            .count();
        assert!(count < 8, "below the floor it is cut: {count} frames");
        assert!(
            matches!(cut.last(), Some(MediaMessage::Failed(why)) if why.contains("sequence cut")),
            "and says so"
        );
        Ok(())
    }
}
