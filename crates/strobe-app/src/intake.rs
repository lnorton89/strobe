// ============================================================================
// Module:       strobe_app::intake
// Description:  What a file or folder dropped on a window is for: a Shadertoy
//               export, a playlist, media for syn_Media, or a scene folder,
//               and copying a scene folder into the library, under its own
//               name or, for Copy to my library, a free one.
//
// Dependencies: strobe-gfx (assets::list_sources); strobe-scene
//               (load::folder_title); std (fs)
// ============================================================================

//! Dropped files (backlog I-02, D218).
//!
//! Anything can be dropped on either window, and [`classify`] decides what
//! it is for from the path alone, so a drop and a typed path take the same
//! route:
//!
//! - **a `.json` file** is a Shadertoy export, for the import worker
//!   (unless it is a scene's own `scene.json`, which stands for its folder);
//! - **a `.toml` file** is a playlist, loaded as the LIST pane's Load would;
//! - **a picture, or a folder of pictures**, goes behind `syn_Media`, as the
//!   Inspector's Choose would (D217);
//! - **a scene folder** (one with a `main.glsl`) is played: from where it is
//!   if it is already in the library, else after [`copy_scene`] has copied
//!   it in;
//! - **a folder of scene folders** (a pack) adds every scene in it to the
//!   library and plays none of them: one is copied a frame, so the show
//!   keeps running, and a banner then says how many were added, already
//!   there or refused (D235). At most [`MAX_PACK`] scenes are taken.
//!
//! A **watched folder** ([`Inbox`], `--watch-folder` or Settings) takes the
//! same route without a drop: a scene folder or a folder of scenes that
//! appears in it is added to the library as a pack is, and nothing plays
//! (D240). Launch and Settings check the folder the same way
//! ([`check_watch_folder`], D265).
//!
//! Anything else is refused with a sentence saying what can be dropped.
//!
//! **Copy to my library** ([`copy_as_own`], D271) is the one copy that
//! never refuses a taken name: a scene of Synesthesia's, which Strobe only
//! reads, becomes the person's own under a free name, so it can be tagged,
//! edited and given presets.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The most scenes taken from one dropped folder of scenes.
pub const MAX_PACK: usize = 500;

/// The most files a dropped scene folder may hold to be copied.
pub const MAX_SCENE_FILES: usize = 2000;

/// The most bytes a dropped scene folder may hold to be copied. Scenes are
/// small; a folder past this is more likely a drop by mistake, and the copy
/// runs on the UI thread.
pub const MAX_SCENE_BYTES: u64 = 256 * 1024 * 1024;

/// What a dropped path is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropped {
    /// An exported Shadertoy shader.
    Shadertoy,
    /// A playlist file.
    Playlist,
    /// A picture or a folder of pictures, for `syn_Media`.
    Media,
    /// A scene folder (the dropped folder, or a dropped `scene.json` or
    /// `main.glsl`'s folder).
    Scene(PathBuf),
    /// A folder of scene folders, in name order.
    Pack(Vec<PathBuf>),
    /// Nothing Strobe opens; the sentence says what can be dropped.
    Unknown(String),
}

/// What `path` is for (see the module docs). Reads the disk only to tell a
/// folder's kind and whether a `scene.json` has a shader beside it.
pub fn classify(path: &Path) -> Dropped {
    if path.is_dir() {
        if is_scene(path) {
            return Dropped::Scene(path.to_path_buf());
        }
        let scenes = pack(path);
        if !scenes.is_empty() {
            return Dropped::Pack(scenes);
        }
        return match strobe_gfx::assets::list_sources(path) {
            Ok(_) => Dropped::Media,
            Err(_) => Dropped::Unknown(format!(
                "{} holds no scene (main.glsl) and no .png or .jpg pictures.",
                name_of(path)
            )),
        };
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let parent = path.parent().filter(|dir| is_scene(dir));
    match extension.as_str() {
        "json" | "glsl" if file_name == "scene.json" || file_name == "main.glsl" => match parent {
            Some(dir) => Dropped::Scene(dir.to_path_buf()),
            None if extension == "json" => Dropped::Unknown(format!(
                "{} has no main.glsl beside it, so it is not a scene.",
                name_of(path)
            )),
            None => unknown(path),
        },
        "json" => Dropped::Shadertoy,
        "toml" => Dropped::Playlist,
        "png" | "jpg" | "jpeg" => Dropped::Media,
        _ => unknown(path),
    }
}

/// What dropping a path of this kind will do, for the banner shown while
/// it hovers over a window.
pub fn hint(kind: &Dropped) -> String {
    match kind {
        Dropped::Shadertoy => "Drop to import this Shadertoy export.".to_string(),
        Dropped::Playlist => "Drop to load this playlist.".to_string(),
        Dropped::Media => "Drop to show this behind syn_Media.".to_string(),
        Dropped::Scene(_) => {
            "Drop to play this scene (one from outside the library is copied in first).".to_string()
        }
        Dropped::Pack(scenes) => match scenes.len() {
            1 => "Drop to add the scene in this folder to the library.".to_string(),
            n => format!("Drop to add the {n} scenes in this folder to the library."),
        },
        Dropped::Unknown(why) => why.clone(),
    }
}

/// The scene folders directly inside `dir` (not following links), in name
/// order, at most [`MAX_PACK`].
fn pack(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut scenes: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|path| is_scene(path))
        .collect();
    scenes.sort();
    scenes.truncate(MAX_PACK);
    scenes
}

/// Whether `dir` is a scene folder.
fn is_scene(dir: &Path) -> bool {
    dir.join("main.glsl").is_file()
}

fn unknown(path: &Path) -> Dropped {
    Dropped::Unknown(format!(
        "{} is not something Strobe opens. Drop a scene folder, a playlist (.toml), \
         a picture or a folder of pictures (for syn_Media), or a Shadertoy export (.json).",
        name_of(path)
    ))
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Copies the scene folder `from` into `library`, under its own name, and
/// returns where it landed. Refuses a name already in the library, a folder
/// past [`MAX_SCENE_FILES`] or [`MAX_SCENE_BYTES`], and a symbolic link
/// inside it; a copy that fails part-way is removed.
pub fn copy_scene(from: &Path, library: &Path) -> Result<PathBuf, String> {
    let name = from
        .file_name()
        .ok_or_else(|| format!("{} has no folder name", from.display()))?;
    let to = library.join(name);
    if to.exists() {
        return Err(format!(
            "the library already has a scene called {}",
            name.to_string_lossy()
        ));
    }
    let files = bounded_files(from)?;
    match copy_files(from, &files, &to) {
        Ok(()) => Ok(to),
        Err(error) => {
            remove_partial(&to);
            Err(error)
        }
    }
}

/// The most numbered names [`copy_as_own`] tries before giving up.
const MAX_COPY_NAMES: u32 = 99;

/// Copies the scene folder `from` (one of Synesthesia's, D271) into
/// `library` as a scene the person owns, and returns where it landed. It
/// keeps its folder name when the library has no scene of that name, and
/// otherwise becomes `<name>-2`, `-3`... (with the `.synScene` suffix kept
/// when it had one), so it never lands on another scene. Bounded and
/// link-free as [`copy_scene`] is. The files go into a hidden staging
/// folder that is renamed into place, as an import's are, so a copy that
/// fails or is cut off part-way leaves nothing under a scene's name.
pub fn copy_as_own(from: &Path, library: &Path) -> Result<PathBuf, String> {
    let title = strobe_scene::load::folder_title(from);
    if title.is_empty() {
        return Err(format!("{} has no folder name", from.display()));
    }
    let suffix = if from
        .file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(".synScene"))
    {
        ".synScene"
    } else {
        ""
    };
    // A scene is known by its title, so `x` and `x.synScene` are one name.
    let taken = |stem: &str| {
        library.join(stem).exists() || library.join(format!("{stem}.synScene")).exists()
    };
    let stem = std::iter::once(title.clone())
        .chain((2..=MAX_COPY_NAMES).map(|n| format!("{title}-{n}")))
        .find(|stem| !taken(stem))
        .ok_or_else(|| {
            format!("the library already has {title} and {MAX_COPY_NAMES} numbered copies of it")
        })?;
    let folder = format!("{stem}{suffix}");
    let files = bounded_files(from)?;
    std::fs::create_dir_all(library).map_err(|e| format!("{}: {e}", library.display()))?;
    let staging = library.join(format!(".{folder}.copying"));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    }
    let to = library.join(&folder);
    let copied = copy_files(from, &files, &staging).and_then(|()| {
        std::fs::rename(&staging, &to).map_err(|e| format!("{}: {e}", to.display()))
    });
    match copied {
        Ok(()) => Ok(to),
        Err(error) => {
            remove_partial(&staging);
            Err(error)
        }
    }
}

/// Every file under the scene folder `from`, refused past
/// [`MAX_SCENE_FILES`] or [`MAX_SCENE_BYTES`] or at a link.
fn bounded_files(from: &Path) -> Result<Vec<(PathBuf, u64)>, String> {
    let mut files = Vec::new();
    measure(from, from, &mut files)?;
    let bytes: u64 = files.iter().map(|(_, len)| *len).sum();
    if files.len() > MAX_SCENE_FILES || bytes > MAX_SCENE_BYTES {
        return Err(format!(
            "{} holds {} files, {} MB: more than a scene should ({MAX_SCENE_FILES} files, {} MB)",
            from.display(),
            files.len(),
            bytes / (1024 * 1024),
            MAX_SCENE_BYTES / (1024 * 1024)
        ));
    }
    Ok(files)
}

/// Copies `files` (relative to `from`) to the same places under `to`.
fn copy_files(from: &Path, files: &[(PathBuf, u64)], to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    files.iter().try_for_each(|(relative, _)| {
        let target = to.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(from.join(relative), &target)
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", target.display()))
    })
}

/// Removes what a failed copy left at `dir`, saying so if it cannot.
fn remove_partial(dir: &Path) {
    if let Err(cleanup) = std::fs::remove_dir_all(dir) {
        log::warn!(
            "could not remove the partial copy {}: {cleanup}",
            dir.display()
        );
    }
}

/// A watched folder: scenes that appear in it go to the library (D240).
///
/// Polled every [`Inbox::EVERY`] seconds from the frame loop: one
/// `read_dir` and, for an entry not yet taken, a bounded walk that sums its
/// files. An entry is taken once its sum (files, bytes, newest time) is the
/// same on two polls in a row, so a folder still being copied in is left
/// until the copy has finished. What was there when watching began is
/// never taken: turning the watch on does not import the folder's past.
#[derive(Debug)]
pub struct Inbox {
    dir: PathBuf,
    since: f32,
    /// Entries already taken, or there at the start, or not scenes.
    done: HashSet<PathBuf>,
    /// Entries seen on the last poll, with their sums.
    seen: HashMap<PathBuf, Sum>,
}

/// What a folder holds, enough to tell that it stopped changing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sum {
    files: u64,
    bytes: u64,
    newest: Option<SystemTime>,
}

impl Inbox {
    /// Seconds between polls.
    pub const EVERY: f32 = 2.0;

    /// The most files walked to sum one entry; past it the entry is not a
    /// scene a person dropped there.
    const MAX_WALK: u64 = 10_000;

    /// Watches `dir`, leaving alone what is in it now.
    pub fn new(dir: PathBuf) -> Self {
        let done = entries(&dir).into_iter().collect();
        Self {
            dir,
            since: 0.0,
            done,
            seen: HashMap::new(),
        }
    }

    /// The watched folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Counts `dt`; every [`Inbox::EVERY`] seconds, polls and returns the
    /// scene folders that have settled since the last poll (a folder of
    /// scenes gives its scenes), in name order.
    pub fn poll(&mut self, dt: f32) -> Vec<PathBuf> {
        if dt.is_finite() && dt > 0.0 {
            self.since += dt;
        }
        if self.since < Self::EVERY {
            return Vec::new();
        }
        self.since = 0.0;
        self.check()
    }

    /// One poll, now.
    fn check(&mut self) -> Vec<PathBuf> {
        let mut taken = Vec::new();
        let mut seen = HashMap::new();
        for entry in entries(&self.dir) {
            if self.done.contains(&entry) {
                continue;
            }
            let Some(sum) = sum(&entry) else {
                continue;
            };
            if self.seen.get(&entry) != Some(&sum) {
                seen.insert(entry, sum);
                continue;
            }
            match classify(&entry) {
                Dropped::Scene(dir) => taken.push(dir),
                Dropped::Pack(scenes) => taken.extend(scenes),
                Dropped::Shadertoy | Dropped::Playlist | Dropped::Media | Dropped::Unknown(_) => {
                    log::info!(
                        "watched folder: {} is not a scene; left alone",
                        entry.display()
                    );
                }
            }
            self.done.insert(entry);
        }
        self.seen = seen;
        taken
    }
}

/// The entries directly inside `dir`, in name order.
fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|read| read.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.sort();
    paths
}

/// The files under `path` (or `path` itself), summed; `None` past
/// [`Inbox::MAX_WALK`] files or when nothing can be read.
fn sum(path: &Path) -> Option<Sum> {
    let mut total = Sum {
        files: 0,
        bytes: 0,
        newest: None,
    };
    let mut stack = vec![path.to_path_buf()];
    while let Some(next) = stack.pop() {
        let meta = std::fs::symlink_metadata(&next).ok()?;
        if meta.is_dir() {
            stack.extend(
                std::fs::read_dir(&next)
                    .ok()?
                    .filter_map(Result::ok)
                    .map(|e| e.path()),
            );
            continue;
        }
        total.files += 1;
        total.bytes += meta.len();
        let modified = meta.modified().ok();
        total.newest = total.newest.max(modified);
        if total.files > Inbox::MAX_WALK {
            return None;
        }
    }
    Some(total)
}

/// Whether `folder` can be watched for scenes to add to the library in
/// `library` (D240, D265): it must be a folder that exists, and not the
/// library itself, whose own new scenes (an import, a pack) would come back
/// through the queue as "already there". Launch and Settings both ask this.
///
/// # Errors
///
/// One sentence saying why not.
pub fn check_watch_folder(folder: &Path, library: &Path) -> Result<PathBuf, String> {
    if folder.as_os_str().is_empty() {
        return Err("Type the folder to watch.".to_string());
    }
    if !folder.is_dir() {
        return Err(format!("{} is not a folder.", folder.display()));
    }
    if same_dir(folder, library) {
        return Err(format!(
            "{} is the library itself: watch a folder scenes are copied into before they \
             join it.",
            folder.display()
        ));
    }
    Ok(folder.to_path_buf())
}

/// Whether two paths are the same folder (also through links or `..`).
pub fn same_dir(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

/// A dropped folder of scenes being added to the library, one scene a
/// [`Pack::step`] (D235).
#[derive(Debug, Default)]
pub struct Pack {
    queue: VecDeque<PathBuf>,
    added: usize,
    already: usize,
    refused: Vec<(PathBuf, String)>,
}

impl Pack {
    /// A pack of these scene folders.
    pub fn new(scenes: Vec<PathBuf>) -> Self {
        Self {
            queue: scenes.into(),
            ..Self::default()
        }
    }

    /// Adds more scenes to the queue (a second folder dropped meanwhile).
    pub fn extend(&mut self, scenes: Vec<PathBuf>) {
        self.queue.extend(scenes);
    }

    /// Takes the next scene: skips it if `known` says the library has it or
    /// the library has a scene of that name, else copies it in. Returns
    /// whether any are left.
    pub fn step(&mut self, library: &Path, known: impl Fn(&Path) -> bool) -> bool {
        let Some(dir) = self.queue.pop_front() else {
            return false;
        };
        let taken = dir
            .file_name()
            .is_some_and(|name| library.join(name).exists());
        if known(&dir) || taken {
            self.already += 1;
        } else {
            match copy_scene(&dir, library) {
                Ok(_) => self.added += 1,
                Err(why) => self.refused.push((dir, why)),
            }
        }
        !self.queue.is_empty()
    }

    /// How many were copied in.
    pub fn added(&self) -> usize {
        self.added
    }

    /// The scenes that could not be copied, and why.
    pub fn refused(&self) -> &[(PathBuf, String)] {
        &self.refused
    }

    /// One sentence for the banner, once the queue is empty.
    pub fn summary(&self) -> String {
        let scenes = |n: usize| {
            if n == 1 {
                "1 scene".to_string()
            } else {
                format!("{n} scenes")
            }
        };
        let mut parts = vec![format!("Added {} to the library", scenes(self.added))];
        if self.already > 0 {
            parts.push(format!("{} already there", self.already));
        }
        if !self.refused.is_empty() {
            parts.push(format!(
                "{} could not be added (see the log)",
                self.refused.len()
            ));
        }
        format!("{}.", parts.join("; "))
    }
}

/// Lists every file under `dir` relative to `root`, with its length, and
/// stops as soon as the bounds are passed (so a huge folder is not walked
/// to the end).
fn measure(root: &Path, dir: &Path, files: &mut Vec<(PathBuf, u64)>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if kind.is_symlink() {
            return Err(format!(
                "{} is a link; copy the scene by hand",
                path.display()
            ));
        }
        if kind.is_dir() {
            measure(root, &path, files)?;
        } else {
            let len = entry
                .metadata()
                .map_err(|e| format!("{}: {e}", path.display()))?
                .len();
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
            files.push((relative.to_path_buf(), len));
        }
        if files.len() > MAX_SCENE_FILES {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// A scratch folder removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Result<Self> {
            let dir =
                std::env::temp_dir().join(format!("strobe-intake-{}-{tag}", std::process::id()));
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scene(dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir.join("images"))?;
        std::fs::write(
            dir.join("main.glsl"),
            "vec4 renderMain() { return vec4(1.0); }",
        )?;
        std::fs::write(dir.join("scene.json"), "{}")?;
        std::fs::write(dir.join("images/a.png"), b"not really")?;
        Ok(())
    }

    #[test]
    fn a_watched_folder_must_exist_and_not_be_the_library() -> Result<()> {
        let scratch = Scratch::new("watch-check")?;
        let library = scratch.0.join("scenes");
        let inbox = scratch.0.join("inbox");
        std::fs::create_dir_all(&library)?;
        std::fs::create_dir_all(&inbox)?;
        std::fs::write(scratch.0.join("file.txt"), "x")?;
        assert_eq!(
            check_watch_folder(&inbox, &library),
            Ok(inbox.clone()),
            "a folder beside the library"
        );
        let refused = |folder: &Path| check_watch_folder(folder, &library).err();
        assert!(
            refused(Path::new("")).is_some_and(|why| why.contains("Type")),
            "nothing typed"
        );
        assert!(
            refused(&scratch.0.join("missing")).is_some_and(|why| why.contains("not a folder")),
            "a folder that does not exist"
        );
        assert!(
            refused(&scratch.0.join("file.txt")).is_some_and(|why| why.contains("not a folder")),
            "a file"
        );
        assert!(
            refused(&scratch.0.join("inbox/../scenes"))
                .is_some_and(|why| why.contains("library itself")),
            "the library, however it is spelled"
        );
        Ok(())
    }

    #[test]
    fn each_kind_of_drop_is_told_apart() -> Result<()> {
        let scratch = Scratch::new("classify")?;
        let root = &scratch.0;
        let ink = root.join("ink.synScene");
        scene(&ink)?;
        let pictures = root.join("loop");
        std::fs::create_dir_all(&pictures)?;
        std::fs::write(pictures.join("f1.png"), b"x")?;
        let empty = root.join("empty");
        std::fs::create_dir_all(&empty)?;
        std::fs::write(root.join("loose-scene.json"), "{}")?;

        assert_eq!(
            classify(&ink),
            Dropped::Scene(ink.clone()),
            "a scene folder"
        );
        assert_eq!(
            classify(&ink.join("scene.json")),
            Dropped::Scene(ink.clone()),
            "a scene's scene.json stands for its folder"
        );
        assert_eq!(
            classify(&ink.join("main.glsl")),
            Dropped::Scene(ink.clone()),
            "so does its main.glsl"
        );
        assert_eq!(classify(&pictures), Dropped::Media, "a folder of pictures");
        assert_eq!(classify(&root.join("x.JPG")), Dropped::Media, "a picture");
        assert_eq!(
            classify(&root.join("show.toml")),
            Dropped::Playlist,
            "a playlist"
        );
        assert_eq!(
            classify(&root.join("loose-scene.json")),
            Dropped::Shadertoy,
            "any other .json is a Shadertoy export"
        );
        assert!(
            matches!(classify(&empty), Dropped::Unknown(why) if why.contains("no scene")),
            "an empty folder says why"
        );
        assert!(
            matches!(classify(&root.join("notes.txt")), Dropped::Unknown(why) if why.contains("Drop a scene folder")),
            "anything else says what can be dropped"
        );
        Ok(())
    }

    #[test]
    fn a_scene_is_copied_whole_and_never_over_another() -> Result<()> {
        let scratch = Scratch::new("copy")?;
        let from = scratch.0.join("outside/ink.synScene");
        scene(&from)?;
        let library = scratch.0.join("library");
        std::fs::create_dir_all(&library)?;

        let landed = copy_scene(&from, &library).map_err(anyhow::Error::msg)?;
        assert_eq!(landed, library.join("ink.synScene"), "under its own name");
        assert!(landed.join("main.glsl").is_file(), "the shader came");
        assert!(landed.join("images/a.png").is_file(), "and its subfolders");
        assert!(
            from.join("main.glsl").is_file(),
            "the original is left where it was"
        );

        let again = copy_scene(&from, &library);
        assert!(
            again.as_ref().is_err_and(|why| why.contains("already has")),
            "a second copy is refused, not merged: {again:?}"
        );
        Ok(())
    }

    #[test]
    fn copy_to_my_library_takes_a_free_name_and_leaves_no_staging() -> Result<()> {
        let scratch = Scratch::new("copy-own")?;
        let from = scratch.0.join("synesthesia/release/ink.synScene");
        scene(&from)?;
        let library = scratch.0.join("library");
        std::fs::create_dir_all(library.join("ink"))?;

        let first = copy_as_own(&from, &library).map_err(anyhow::Error::msg)?;
        assert_eq!(
            first,
            library.join("ink-2.synScene"),
            "the library has an `ink` already (without the suffix), so a numbered name"
        );
        assert!(
            first.join("images/a.png").is_file(),
            "copied whole, subfolders too"
        );
        let second = copy_as_own(&from, &library).map_err(anyhow::Error::msg)?;
        assert_eq!(second, library.join("ink-3.synScene"), "and the next one");
        let left: Vec<String> = std::fs::read_dir(&library)?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(left.is_empty(), "no staging folder is left: {left:?}");
        assert!(
            from.join("main.glsl").is_file(),
            "the original is left where it was"
        );
        Ok(())
    }

    #[test]
    fn a_folder_of_scenes_is_a_pack_and_lands_one_scene_a_step() -> Result<()> {
        let scratch = Scratch::new("pack")?;
        let drop = scratch.0.join("pack");
        for name in ["b.synScene", "a.synScene", "c.synScene"] {
            scene(&drop.join(name))?;
        }
        std::fs::create_dir_all(drop.join("not-a-scene"))?;
        std::fs::write(drop.join("readme.txt"), "x")?;
        let library = scratch.0.join("library");
        scene(&library.join("c.synScene"))?;

        let Dropped::Pack(scenes) = classify(&drop) else {
            anyhow::bail!("not a pack: {:?}", classify(&drop));
        };
        let names: Vec<_> = scenes.iter().filter_map(|p| p.file_name()).collect();
        assert_eq!(
            names,
            ["a.synScene", "b.synScene", "c.synScene"],
            "scenes only, in name order"
        );
        assert!(
            hint(&Dropped::Pack(scenes.clone())).contains("3 scenes"),
            "the banner counts them"
        );

        let mut pack = Pack::new(scenes);
        let mut steps = 1;
        while pack.step(&library, |_| false) {
            steps += 1;
        }
        assert_eq!(steps, 3, "one scene a step");
        assert!(library.join("a.synScene/main.glsl").is_file(), "a landed");
        assert!(library.join("b.synScene/main.glsl").is_file(), "b landed");
        assert_eq!(pack.added(), 2, "two added");
        assert_eq!(
            pack.summary(),
            "Added 2 scenes to the library; 1 already there.",
            "c was there by name"
        );
        assert!(
            !pack.step(&library, |_| false),
            "an empty pack does nothing"
        );
        Ok(())
    }

    #[test]
    fn a_watched_folder_takes_new_scenes_once_they_settle_and_leaves_the_old() -> Result<()> {
        let scratch = Scratch::new("inbox")?;
        let inbox_dir = scratch.0.join("inbox");
        scene(&inbox_dir.join("old.synScene"))?;
        let mut inbox = Inbox::new(inbox_dir.clone());

        scene(&inbox_dir.join("new.synScene"))?;
        std::fs::write(inbox_dir.join("notes.txt"), "x")?;
        assert!(inbox.check().is_empty(), "first sight: not yet settled");
        // Still being copied: it changes between polls.
        std::fs::write(inbox_dir.join("new.synScene/more.glsl"), "// more")?;
        assert!(inbox.check().is_empty(), "it changed: wait again");
        let taken = inbox.check();
        assert_eq!(
            taken,
            vec![inbox_dir.join("new.synScene")],
            "settled: taken, and the old scene and the note are not"
        );
        assert!(inbox.check().is_empty(), "taken once");

        let pack = inbox_dir.join("pack");
        scene(&pack.join("a.synScene"))?;
        scene(&pack.join("b.synScene"))?;
        inbox.check();
        assert_eq!(
            inbox.check(),
            vec![pack.join("a.synScene"), pack.join("b.synScene")],
            "a folder of scenes gives its scenes"
        );
        assert!(inbox.poll(0.5).is_empty(), "polls only every two seconds");
        Ok(())
    }
}
