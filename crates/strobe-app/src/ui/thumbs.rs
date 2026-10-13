// ============================================================================
// Module:       strobe_app::ui::thumbs
// Description:  Scene thumbnails for the browser: decoded and shrunk on a
//               worker thread, uploaded as egui textures, bounded in number.
//
// Dependencies: egui; image; std (thread, mpsc)
// ============================================================================

//! Scene thumbnails (backlog U-06, D212).
//!
//! A scene names its picture in `scene.json`'s `IMAGE_PATH` (Synesthesia
//! ships one with every scene). The browser asks [`Thumbs::get`] for a row's
//! picture while drawing it:
//!
//! - **Never decoded on the UI thread.** The first request queues the path
//!   for a worker thread, which decodes it and shrinks it to fit
//!   [`MAX_SIZE`]; the row draws its colour swatch until it arrives.
//! - **Asked only for rows on screen.** The browser is virtualised, so a
//!   library of thousands decodes what is scrolled past, once.
//! - **Bounded.** At most [`MAX_TEXTURES`] pictures are kept; past that the
//!   least recently drawn is dropped and asked for again if it comes back.
//! - **A picture that will not decode is remembered as missing**, so a
//!   broken file is tried once a run, not once a frame.
//!
//! None of this is on the scene frame: the control window draws the
//! browser, and the output never waits for a thumbnail.

use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

/// The largest a thumbnail is kept, in pixels.
pub const MAX_SIZE: [u32; 2] = [160, 90];

/// The most thumbnails kept as textures.
pub const MAX_TEXTURES: usize = 256;

/// The most decoded pictures uploaded in one frame, so a burst of arrivals
/// cannot make one UI frame slow.
const UPLOADS_PER_FRAME: usize = 8;

enum Slot {
    /// Asked for; the worker has it.
    Pending,
    /// Uploaded, and when it was last drawn.
    Ready(TextureHandle, u64),
    /// Would not read or decode.
    Missing,
}

/// The thumbnail cache (see the module docs). Starts its worker on the
/// first request.
#[derive(Default)]
pub struct Thumbs {
    slots: HashMap<PathBuf, Slot>,
    requests: Option<Sender<PathBuf>>,
    results: Option<Receiver<(PathBuf, Option<ColorImage>)>>,
    frame: u64,
}

impl std::fmt::Debug for Thumbs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Thumbs")
            .field("slots", &self.slots.len())
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}

impl Thumbs {
    /// Once per control-window frame, before the browser draws: uploads
    /// what the worker finished and drops the oldest past the bound.
    pub fn pump(&mut self, ctx: &Context) {
        self.frame += 1;
        let Some(results) = &self.results else {
            return;
        };
        for (path, image) in results.try_iter().take(UPLOADS_PER_FRAME) {
            let slot = match image {
                Some(image) => {
                    let name = format!("thumb:{}", path.display());
                    Slot::Ready(
                        ctx.load_texture(name, image, TextureOptions::LINEAR),
                        self.frame,
                    )
                }
                None => Slot::Missing,
            };
            self.slots.insert(path, slot);
        }
        self.evict();
    }

    /// The texture for the picture at `path`, if it is ready; asks for it
    /// otherwise.
    pub fn get(&mut self, path: &Path) -> Option<(TextureId, [f32; 2])> {
        let frame = self.frame;
        match self.slots.get_mut(path) {
            Some(Slot::Ready(texture, used)) => {
                *used = frame;
                let [w, h] = texture.size();
                Some((texture.id(), [w as f32, h as f32]))
            }
            Some(Slot::Pending | Slot::Missing) => None,
            None => {
                if self.ask(path) {
                    self.slots.insert(path.to_path_buf(), Slot::Pending);
                }
                None
            }
        }
    }

    /// How many pictures are kept as textures.
    pub fn len(&self) -> usize {
        self.slots
            .values()
            .filter(|slot| matches!(slot, Slot::Ready(..)))
            .count()
    }

    /// Whether none are.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Queues `path` for the worker, starting it if need be.
    fn ask(&mut self, path: &Path) -> bool {
        if self.requests.is_none() {
            let (requests, jobs) = channel::<PathBuf>();
            let (done, results) = channel();
            let started = std::thread::Builder::new()
                .name("strobe-thumbs".to_string())
                .spawn(move || {
                    for path in jobs {
                        let image = decode(&path);
                        if done.send((path, image)).is_err() {
                            break;
                        }
                    }
                });
            if let Err(error) = started {
                log::warn!("thumbnails: no worker thread: {error}");
                return false;
            }
            self.requests = Some(requests);
            self.results = Some(results);
        }
        self.requests
            .as_ref()
            .is_some_and(|requests| requests.send(path.to_path_buf()).is_ok())
    }

    /// Drops the least recently drawn textures past [`MAX_TEXTURES`].
    fn evict(&mut self) {
        let mut ready: Vec<(u64, PathBuf)> = self
            .slots
            .iter()
            .filter_map(|(path, slot)| match slot {
                Slot::Ready(_, used) => Some((*used, path.clone())),
                Slot::Pending | Slot::Missing => None,
            })
            .collect();
        if ready.len() <= MAX_TEXTURES {
            return;
        }
        ready.sort();
        for (_, path) in ready.iter().take(ready.len() - MAX_TEXTURES) {
            self.slots.remove(path);
        }
    }
}

/// Reads and shrinks one picture; `None` if it cannot be.
fn decode(path: &Path) -> Option<ColorImage> {
    let image = match image::open(path) {
        Ok(image) => image,
        Err(error) => {
            log::info!("thumbnail {}: {error}", path.display());
            return None;
        }
    };
    let small = image.thumbnail(MAX_SIZE[0], MAX_SIZE[1]).to_rgba8();
    let size = [small.width() as usize, small.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, small.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::time::{Duration, Instant};

    /// Pumps until `path` is ready or missing, or a few seconds pass.
    fn settle(thumbs: &mut Thumbs, ctx: &Context, path: &Path) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            thumbs.pump(ctx);
            if !matches!(thumbs.slots.get(path), Some(Slot::Pending) | None) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_picture_arrives_shrunk_and_a_broken_one_is_asked_for_once() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("strobe-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let good = dir.join("thumb.png");
        image::RgbaImage::from_pixel(640, 360, image::Rgba([255, 0, 0, 255])).save(&good)?;
        let bad = dir.join("broken.png");
        std::fs::write(&bad, b"not a png")?;

        let ctx = Context::default();
        let mut thumbs = Thumbs::default();
        assert!(thumbs.get(&good).is_none(), "not on the first frame");
        settle(&mut thumbs, &ctx, &good);
        let Some((_, size)) = thumbs.get(&good) else {
            anyhow::bail!("the picture did not arrive");
        };
        assert_eq!(size, [160.0, 90.0], "shrunk to fit, aspect kept");

        assert!(thumbs.get(&bad).is_none(), "asked for");
        settle(&mut thumbs, &ctx, &bad);
        assert!(
            matches!(thumbs.slots.get(&bad), Some(Slot::Missing)),
            "remembered as missing, not retried"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn past_the_bound_the_least_recently_drawn_go() -> Result<()> {
        let ctx = Context::default();
        let mut thumbs = Thumbs::default();
        let image = ColorImage::new([1, 1], vec![egui::Color32::BLACK]);
        for i in 0..(MAX_TEXTURES + 5) {
            thumbs.frame = i as u64;
            let texture = ctx.load_texture(format!("t{i}"), image.clone(), TextureOptions::LINEAR);
            thumbs.slots.insert(
                PathBuf::from(format!("p{i}")),
                Slot::Ready(texture, i as u64),
            );
        }
        thumbs.evict();
        assert_eq!(thumbs.len(), MAX_TEXTURES, "bounded");
        assert!(
            !thumbs.slots.contains_key(Path::new("p0")),
            "the oldest went first"
        );
        assert!(
            thumbs
                .slots
                .contains_key(Path::new(&format!("p{}", MAX_TEXTURES + 4))),
            "the newest stayed"
        );
        Ok(())
    }
}
