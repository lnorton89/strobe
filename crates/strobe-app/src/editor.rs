// ============================================================================
// Module:       strobe_app::editor
// Description:  The Editor's documents: a scene's source files as text, what
//               is unsaved, a safe save with timestamped backups, and where a
//               compiler error points. Pure apart from file reads and writes.
//
// Dependencies: std; strobe-diag (the data directory, for backups);
//               strobe-scene (the scene.json parser)
// ============================================================================

//! The Editor workspace's data (backlog U-02, UI_DESIGN section 14, D204).
//!
//! A [`Doc`] holds one scene's source files: `main.glsl`, any pass shaders
//! in `passes/`, `scene.json` and `script.js`. Each [`Tab`] keeps the text
//! as typed and the text last saved, so "unsaved" is a comparison, never a
//! flag that can drift.
//!
//! ## Editing is not playing
//!
//! Typing changes nothing on screen. **Save** writes the file; **Save and
//! reload** writes it and asks the app to load the scene again, the same
//! path the Reload button takes. A scene that fails to compile leaves the
//! previous scene playing, with the error shown here and in the failure
//! banner. Nothing compiles while typing, so an old laptop never stalls
//! mid-show because someone opened the editor.
//!
//! ## Saves are atomic and backed up
//!
//! A save writes `<file>.strobe-tmp` beside the file and renames it into
//! place, so a crash mid-save leaves the old file whole. Before the first
//! save of a file in a session, its on-disk text is copied to the data
//! directory (`backups/<scene>/<file>.<timestamp>`), keeping the newest
//! [`KEEP_BACKUPS`], so "Restore backup" can undo a whole editing session.
//! Backups are never written into the scene folder, which is the operator's.
//!
//! ## Errors point at a line and, where known, a column
//!
//! [`error_location`] reads `<file>.glsl:<line>[:<column>]` from a compiler
//! message: `strobe-gfx` writes the column when the driver gives one (only
//! Mesa does), already put back on the user's line (D263). Before Save and
//! reload, [`check_scene_json`] parses `scene.json` as the loader will; a
//! file that does not parse is refused with its own line and column, and
//! nothing is written or reloaded.

use std::io;
use std::path::{Path, PathBuf};

/// Backups kept per file.
pub const KEEP_BACKUPS: usize = 10;

/// The files the Editor opens, in tab order, when they exist.
const FIXED_FILES: [&str; 3] = ["main.glsl", "scene.json", "script.js"];

/// One open file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    /// The path inside the scene folder, with `/` separators.
    pub file: String,
    /// The text as edited.
    pub text: String,
    /// The text on disk, as last read or saved.
    pub saved: String,
    /// Whether this session has backed the file up yet.
    backed_up: bool,
}

impl Tab {
    /// Whether the text differs from what is on disk.
    pub fn is_dirty(&self) -> bool {
        self.text != self.saved
    }

    /// Whether this is GLSL (for highlighting).
    pub fn is_glsl(&self) -> bool {
        self.file.ends_with(".glsl")
    }
}

/// One scene's files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    /// The scene folder.
    pub dir: PathBuf,
    /// The scene's library name (its folder name, without `.synScene`).
    pub name: String,
    /// The files, in tab order.
    pub tabs: Vec<Tab>,
    /// The tab shown.
    pub active: usize,
    /// Why the last Save and reload was refused: `scene.json` did not
    /// parse (D263). Cleared when that file is edited or reverted.
    pub invalid: Option<Invalid>,
}

/// A `scene.json` that the scene loader would refuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    /// The parser's message.
    pub message: String,
    /// Where it points, when it points somewhere.
    pub at: Option<Location>,
}

/// A place in one of the scene's files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The file, relative to the scene folder.
    pub file: String,
    /// The 1-based line.
    pub line: usize,
    /// The 1-based column, in bytes, when known.
    pub column: Option<usize>,
}

impl Location {
    /// "line 12" or "line 12, column 5", for a button.
    pub fn describe(&self) -> String {
        match self.column {
            Some(column) => format!("line {}, column {column}", self.line),
            None => format!("line {}", self.line),
        }
    }
}

/// The file the Editor validates before a reload.
pub const SCENE_JSON: &str = "scene.json";

/// Whether `text`, as `scene.json` of a scene titled `title`, is one the
/// scene loader accepts: the same newline handling and the same parser
/// (`strobe_scene::scene::SceneDef::parse`), so a reload that passes this
/// check does not fail on its `scene.json`.
///
/// # Errors
///
/// The parser's message, with the line and column a JSON syntax or type
/// error has.
pub fn check_scene_json(text: &str, title: &str) -> Result<(), Invalid> {
    let normalised;
    let text = if text.contains('\r') {
        normalised = strobe_scene::load::newlines(text);
        normalised.as_str()
    } else {
        text
    };
    match strobe_scene::scene::SceneDef::parse(text, title) {
        Ok(_) => Ok(()),
        Err(error) => {
            // Only a JSON error has a place; the rest are about a value.
            let at = if let strobe_scene::SceneError::Json(json) = &error
                && json.line() > 0
            {
                Some(Location {
                    file: SCENE_JSON.to_owned(),
                    line: json.line(),
                    column: (json.column() > 0).then_some(json.column()),
                })
            } else {
                None
            };
            Err(Invalid {
                message: error.to_string(),
                at,
            })
        }
    }
}

impl Doc {
    /// Reads the scene's source files. A file that is absent is not opened;
    /// one that cannot be read is an error.
    ///
    /// # Errors
    ///
    /// When a file that exists cannot be read as text.
    pub fn open(dir: &Path, name: &str) -> io::Result<Self> {
        let mut files: Vec<String> = Vec::new();
        files.push(FIXED_FILES[0].to_owned());
        let mut passes: Vec<String> = std::fs::read_dir(dir.join("passes"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .filter(|name| name.ends_with(".glsl"))
                    .map(|name| format!("passes/{name}"))
                    .collect()
            })
            .unwrap_or_default();
        passes.sort();
        files.extend(passes);
        files.extend(FIXED_FILES[1..].iter().map(|f| (*f).to_owned()));
        let mut tabs = Vec::new();
        for file in files {
            let path = dir.join(&file);
            if !path.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&path)?;
            tabs.push(Tab {
                file,
                saved: text.clone(),
                text,
                backed_up: false,
            });
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            name: name.to_owned(),
            tabs,
            active: 0,
            invalid: None,
        })
    }

    /// A document that is not read from disk, for tests and screenshots.
    pub fn in_memory(dir: &str, name: &str, files: &[(&str, &str)]) -> Self {
        Self {
            dir: PathBuf::from(dir),
            name: name.to_owned(),
            tabs: files
                .iter()
                .map(|(file, text)| Tab {
                    file: (*file).to_owned(),
                    text: (*text).to_owned(),
                    saved: (*text).to_owned(),
                    backed_up: false,
                })
                .collect(),
            active: 0,
            invalid: None,
        }
    }

    /// Whether any tab is unsaved.
    pub fn is_dirty(&self) -> bool {
        self.tabs.iter().any(Tab::is_dirty)
    }

    /// The tab for `file`, if open.
    pub fn tab_index(&self, file: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.file == file)
    }

    /// Saves every unsaved tab, backing each file up first. Returns how many
    /// files were written.
    ///
    /// # Errors
    ///
    /// The first file that could not be written; the files before it are
    /// saved.
    pub fn save(&mut self, backups: &Path) -> io::Result<usize> {
        let mut written = 0;
        for tab in &mut self.tabs {
            if !tab.is_dirty() {
                continue;
            }
            let path = self.dir.join(&tab.file);
            if !tab.backed_up {
                backup(backups, &self.name, &tab.file, &tab.saved)?;
                tab.backed_up = true;
            }
            write_atomically(&path, &tab.text)?;
            tab.saved.clone_from(&tab.text);
            written += 1;
        }
        Ok(written)
    }

    /// Throws away the unsaved edits in every tab.
    pub fn revert(&mut self) {
        for tab in &mut self.tabs {
            tab.text.clone_from(&tab.saved);
        }
        self.invalid = None;
    }

    /// Checks the `scene.json` tab as edited (D263); a scene without one is
    /// fine, as the loader reads a missing file as `{}`.
    ///
    /// # Errors
    ///
    /// Why the loader would refuse it.
    pub fn check(&self) -> Result<(), Invalid> {
        match self.tab_index(SCENE_JSON).and_then(|i| self.tabs.get(i)) {
            Some(tab) => check_scene_json(&tab.text, &self.name),
            None => Ok(()),
        }
    }
}

/// Where backups go: `<data dir>/backups`.
pub fn backup_root() -> PathBuf {
    strobe_diag::paths::base_dir().join("backups")
}

/// The folder holding `file`'s backups for scene `name`.
fn backup_dir(root: &Path, name: &str, file: &str) -> PathBuf {
    root.join(sanitise(name)).join(sanitise(file))
}

/// A path component made of the safe characters of `text`.
fn sanitise(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Writes `text` as a new backup of `file` and keeps the newest
/// [`KEEP_BACKUPS`].
fn backup(root: &Path, name: &str, file: &str, text: &str) -> io::Result<()> {
    let dir = backup_dir(root, name, file);
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    std::fs::write(dir.join(format!("{stamp:015}")), text)?;
    let mut all = backups_in(&dir);
    while all.len() > KEEP_BACKUPS {
        let oldest = all.remove(0);
        let _ = std::fs::remove_file(oldest);
    }
    Ok(())
}

/// The backups in `dir`, oldest first.
fn backups_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// The newest backup of `file` in scene `name`, as text.
pub fn latest_backup(root: &Path, name: &str, file: &str) -> Option<String> {
    let newest = backups_in(&backup_dir(root, name, file)).pop()?;
    std::fs::read_to_string(newest).ok()
}

/// Writes `text` to `path` through a temporary file beside it.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".strobe-tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

/// The first `<file>.glsl:<line>` or `<file>.glsl:<line>:<column>` a
/// compiler message names: a file inside the scene, a 1-based line and,
/// when the message has one, a 1-based column on the user's line.
pub fn error_location(message: &str) -> Option<Location> {
    let bytes = message.as_bytes();
    let number_end = |from: usize| {
        bytes
            .get(from..)
            .and_then(|rest| rest.iter().position(|b| !b.is_ascii_digit()))
            .map_or(bytes.len(), |n| from + n)
    };
    let mut search = 0;
    while let Some(found) = message.get(search..)?.find(".glsl:") {
        let dot = search + found;
        let digits_start = dot + ".glsl:".len();
        let digits_end = number_end(digits_start);
        let start = message
            .get(..dot)?
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.')))
            .map_or(0, |i| i + 1);
        if let (Some(file), Ok(line)) = (
            message.get(start..dot + ".glsl".len()),
            message
                .get(digits_start..digits_end)
                .unwrap_or("")
                .parse::<usize>(),
        ) && !file.is_empty()
            && line > 0
        {
            // `:<column>:` straight after the line, as `strobe-gfx` writes
            // it; a driver's own `1:14(3)` after a space is not one.
            let column = (bytes.get(digits_end) == Some(&b':'))
                .then(|| {
                    let end = number_end(digits_end + 1);
                    let number = message.get(digits_end + 1..end)?.parse::<usize>().ok()?;
                    (bytes.get(end) == Some(&b':') && number > 0).then_some(number)
                })
                .flatten();
            return Some(Location {
                file: file.to_owned(),
                line,
                column,
            });
        }
        search = digits_start;
    }
    None
}

/// The byte offset of the start of 1-based `line` in `text`.
pub fn line_offset(text: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line - 2)
        .map_or(text.len(), |(at, _)| at + 1)
}

/// The byte offset of 1-based `line` and, when given, 1-based byte
/// `column` in `text`: never past the end of that line, and always on a
/// character boundary.
pub fn location_offset(text: &str, line: usize, column: Option<usize>) -> usize {
    let start = line_offset(text, line);
    let rest = text.get(start..).unwrap_or_default();
    let line_len = rest.find('\n').unwrap_or(rest.len());
    let mut at = start + column.map_or(0, |c| c.saturating_sub(1)).min(line_len);
    while at > start && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// What a stretch of GLSL is, for colouring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    /// Plain code.
    Plain,
    /// A `//` or `/* */` comment.
    Comment,
    /// A keyword or type.
    Keyword,
    /// A number.
    Number,
    /// A preprocessor line.
    Directive,
    /// An SSF or Strobe built-in name (`syn_*`, `_uv`, `TIME`...).
    Builtin,
}

/// GLSL keywords and types.
const KEYWORDS: [&str; 52] = [
    "if",
    "else",
    "for",
    "while",
    "do",
    "return",
    "break",
    "continue",
    "discard",
    "switch",
    "case",
    "default",
    "const",
    "uniform",
    "in",
    "out",
    "inout",
    "struct",
    "void",
    "bool",
    "int",
    "uint",
    "float",
    "vec2",
    "vec3",
    "vec4",
    "ivec2",
    "ivec3",
    "ivec4",
    "uvec2",
    "uvec3",
    "uvec4",
    "bvec2",
    "bvec3",
    "bvec4",
    "mat2",
    "mat3",
    "mat4",
    "sampler2D",
    "samplerCube",
    "sampler1D",
    "sampler3D",
    "true",
    "false",
    "highp",
    "mediump",
    "lowp",
    "precision",
    "flat",
    "smooth",
    "layout",
    "invariant",
];

/// Names SSF and Strobe provide.
fn is_builtin(word: &str) -> bool {
    word.starts_with("syn_")
        || word.starts_with("_")
        || matches!(
            word,
            "TIME" | "RENDERSIZE" | "FRAMECOUNT" | "PASSINDEX" | "renderMain" | "PI"
        )
}

/// Splits `text` into coloured stretches: `(start, end, token)` byte ranges
/// covering the whole text, in order.
pub fn highlight(text: &str) -> Vec<(usize, usize, Token)> {
    let bytes = text.as_bytes();
    let mut out: Vec<(usize, usize, Token)> = Vec::new();
    let mut push = |start: usize, end: usize, token: Token| {
        if end <= start {
            return;
        }
        match out.last_mut() {
            Some(last) if last.2 == token && last.1 == start => last.1 = end,
            Some(_) | None => out.push((start, end, token)),
        }
    };
    let mut at = 0;
    let mut line_start = true;
    while at < bytes.len() {
        let byte = bytes.get(at).copied().unwrap_or(b' ');
        let next = bytes.get(at + 1).copied();
        let rest_of_line = |from: usize| {
            bytes
                .get(from..)
                .and_then(|r| r.iter().position(|b| *b == b'\n'))
                .map_or(bytes.len(), |n| from + n)
        };
        if byte == b'/' && next == Some(b'/') {
            let end = rest_of_line(at);
            push(at, end, Token::Comment);
            at = end;
            continue;
        }
        if byte == b'/' && next == Some(b'*') {
            let end = text
                .get(at + 2..)
                .and_then(|r| r.find("*/"))
                .map_or(bytes.len(), |n| at + 2 + n + 2);
            push(at, end, Token::Comment);
            at = end;
            continue;
        }
        if byte == b'#' && line_start {
            let end = rest_of_line(at);
            push(at, end, Token::Directive);
            at = end;
            continue;
        }
        if byte == b'\n' {
            line_start = true;
            push(at, at + 1, Token::Plain);
            at += 1;
            continue;
        }
        if byte.is_ascii_whitespace() {
            push(at, at + 1, Token::Plain);
            at += 1;
            continue;
        }
        line_start = false;
        if byte.is_ascii_digit() || (byte == b'.' && next.is_some_and(|b| b.is_ascii_digit())) {
            let mut end = at;
            while bytes
                .get(end)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'.')
            {
                end += 1;
            }
            push(at, end, Token::Number);
            at = end;
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let mut end = at;
            while bytes
                .get(end)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                end += 1;
            }
            let word = text.get(at..end).unwrap_or_default();
            let token = if KEYWORDS.contains(&word) {
                Token::Keyword
            } else if is_builtin(word) {
                Token::Builtin
            } else {
                Token::Plain
            };
            push(at, end, token);
            at = end;
            continue;
        }
        let end = at
            + text
                .get(at..)
                .and_then(|r| r.chars().next())
                .map_or(1, char::len_utf8);
        push(at, end, Token::Plain);
        at = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Result<Self> {
            let dir =
                std::env::temp_dir().join(format!("strobe-editor-{}-{name}", std::process::id()));
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
    fn a_scene_opens_with_its_files_in_order() -> Result<()> {
        let scratch = Scratch::new("open")?;
        let scene = scratch.0.join("demo.synScene");
        std::fs::create_dir_all(scene.join("passes"))?;
        std::fs::write(
            scene.join("main.glsl"),
            "vec4 renderMain() { return vec4(1); }",
        )?;
        std::fs::write(scene.join("scene.json"), "{}")?;
        std::fs::write(scene.join("passes/b.glsl"), "// b")?;
        std::fs::write(scene.join("passes/a.glsl"), "// a")?;
        let doc = Doc::open(&scene, "demo")?;
        let files: Vec<&str> = doc.tabs.iter().map(|t| t.file.as_str()).collect();
        assert_eq!(
            files,
            ["main.glsl", "passes/a.glsl", "passes/b.glsl", "scene.json"],
            "main first, passes sorted, no script.js because there is none"
        );
        assert!(!doc.is_dirty(), "nothing edited");
        Ok(())
    }

    #[test]
    fn save_writes_only_edited_files_and_backs_up_the_original_once() -> Result<()> {
        let scratch = Scratch::new("save")?;
        let scene = scratch.0.join("demo.synScene");
        let backups = scratch.0.join("backups");
        std::fs::create_dir_all(&scene)?;
        std::fs::write(scene.join("main.glsl"), "original")?;
        std::fs::write(scene.join("scene.json"), "{}")?;
        let mut doc = Doc::open(&scene, "demo")?;
        doc.tabs[0].text = "edited once".to_owned();
        assert!(doc.is_dirty(), "edited");
        assert_eq!(doc.save(&backups)?, 1, "one file written");
        assert!(!doc.is_dirty(), "saved");
        assert_eq!(
            std::fs::read_to_string(scene.join("main.glsl"))?,
            "edited once",
            "on disk"
        );
        doc.tabs[0].text = "edited twice".to_owned();
        doc.save(&backups)?;
        assert_eq!(
            latest_backup(&backups, "demo", "main.glsl").as_deref(),
            Some("original"),
            "the backup is the text from before the session, taken once"
        );
        assert!(
            !scene.join("main.glsl.strobe-tmp").exists(),
            "no temporary file left behind"
        );
        Ok(())
    }

    #[test]
    fn revert_throws_away_unsaved_edits() -> Result<()> {
        let scratch = Scratch::new("revert")?;
        std::fs::write(scratch.0.join("main.glsl"), "on disk")?;
        let mut doc = Doc::open(&scratch.0, "demo")?;
        doc.tabs[0].text = "typed".to_owned();
        doc.revert();
        assert_eq!(doc.tabs[0].text, "on disk", "back to the saved text");
        Ok(())
    }

    #[test]
    fn backups_are_capped() -> Result<()> {
        let scratch = Scratch::new("cap")?;
        for n in 0..(KEEP_BACKUPS + 4) {
            backup(&scratch.0, "demo", "main.glsl", &format!("v{n}"))?;
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let kept = backups_in(&backup_dir(&scratch.0, "demo", "main.glsl"));
        assert_eq!(kept.len(), KEEP_BACKUPS, "the oldest are removed");
        assert_eq!(
            latest_backup(&scratch.0, "demo", "main.glsl").as_deref(),
            Some(format!("v{}", KEEP_BACKUPS + 3).as_str()),
            "the newest is kept"
        );
        Ok(())
    }

    fn at(file: &str, line: usize, column: Option<usize>) -> Option<Location> {
        Some(Location {
            file: file.to_owned(),
            line,
            column,
        })
    }

    #[test]
    fn compiler_messages_point_at_a_file_and_line() {
        assert_eq!(
            error_location("main.glsl:42: unknown identifier `foo`"),
            at("main.glsl", 42, None),
            "the assembler's form"
        );
        assert_eq!(
            error_location("pass 1: passes/blur.glsl:7: 0:7(3): error: syntax error"),
            at("passes/blur.glsl", 7, None),
            "a pass file; the driver's own (3) is not on the user's line"
        );
        assert_eq!(
            error_location("(generated): 3:202(2): error"),
            None,
            "generated code is not a file the editor has"
        );
    }

    #[test]
    fn a_column_is_read_when_the_message_has_one() {
        assert_eq!(
            error_location("pass 0: main.glsl:2:35: 1:2(34): error: `x' undeclared"),
            at("main.glsl", 2, Some(35)),
            "line and column, as strobe-gfx writes them"
        );
        assert_eq!(
            error_location("passes/a.glsl:9:1: 1:9(1): error"),
            at("passes/a.glsl", 9, Some(1)),
            "a pass file at the line's start"
        );
        assert_eq!(
            error_location("main.glsl:4:x: nope"),
            at("main.glsl", 4, None),
            "not a number"
        );
        assert_eq!(
            error_location("main.glsl:4:0: nope"),
            at("main.glsl", 4, None),
            "columns count from 1"
        );
    }

    #[test]
    fn a_line_number_finds_its_offset() {
        let text = "one\ntwo\nthree";
        assert_eq!(line_offset(text, 1), 0, "first line");
        assert_eq!(line_offset(text, 3), 8, "third line");
        assert_eq!(line_offset(text, 9), text.len(), "past the end");
    }

    #[test]
    fn a_line_and_column_find_their_offset_within_the_line() {
        let text = "one\ntw\u{e9}o\nthree";
        assert_eq!(location_offset(text, 2, Some(2)), 5, "the w");
        assert_eq!(location_offset(text, 2, None), 4, "no column: the start");
        assert_eq!(
            location_offset(text, 2, Some(40)),
            9,
            "past the line's end: its end, not the next line"
        );
        assert_eq!(
            location_offset(text, 2, Some(4)),
            6,
            "inside a character: its start"
        );
    }

    #[test]
    fn scene_json_is_checked_as_the_loader_reads_it() -> Result<()> {
        assert_eq!(
            check_scene_json("{ \"TITLE\": \"x\" }", "t"),
            Ok(()),
            "valid"
        );
        assert_eq!(
            check_scene_json("{\r\n  \"TITLE\": \"x\"\r\n}\r\n", "t"),
            Ok(()),
            "Windows line endings"
        );
        let broken = check_scene_json("{\n  \"TITLE\": \"x\"\n  \"WIDTH\": 1\n}", "t");
        let Err(invalid) = broken else {
            anyhow::bail!("a missing comma must be refused");
        };
        assert_eq!(
            invalid.at.as_ref().map(|at| (at.file.as_str(), at.line)),
            Some((SCENE_JSON, 3)),
            "it points at the line after the missing comma: {invalid:?}"
        );
        assert!(
            invalid.at.as_ref().and_then(|at| at.column).is_some(),
            "with a column: {invalid:?}"
        );
        let bad_type = check_scene_json(
            "{ \"CONTROLS\": [ { \"NAME\": \"a\", \"TYPE\": \"wobble\" } ] }",
            "t",
        );
        assert!(
            matches!(&bad_type, Err(Invalid { at: None, message }) if message.contains("wobble")),
            "valid JSON the scene loader refuses: {bad_type:?}"
        );
        Ok(())
    }

    #[test]
    fn a_doc_is_checked_by_its_scene_json_tab() {
        let mut doc = Doc::in_memory(
            "scenes/demo",
            "demo",
            &[("main.glsl", "void x() {}"), ("scene.json", "{}")],
        );
        assert_eq!(doc.check(), Ok(()), "valid");
        if let Some(tab) = doc.tabs.get_mut(1) {
            tab.text = "{ oops".to_owned();
        }
        assert!(doc.check().is_err(), "the edited text is what is checked");
        let shader_only = Doc::in_memory("scenes/demo", "demo", &[("main.glsl", "")]);
        assert_eq!(shader_only.check(), Ok(()), "no scene.json reads as {{}}");
    }

    #[test]
    fn highlighting_covers_the_text_and_names_what_it_finds() {
        let text = "#define N 3\n// note\nfloat x = TIME * 2.0; /* c */ syn_BassLevel;\n";
        let spans = highlight(text);
        let mut covered = 0;
        for (start, end, _) in &spans {
            assert_eq!(*start, covered, "spans are contiguous");
            covered = *end;
        }
        assert_eq!(covered, text.len(), "and cover everything");
        let kind = |needle: &str| {
            let at = text.find(needle).unwrap_or(usize::MAX);
            spans
                .iter()
                .find(|(s, e, _)| *s <= at && at < *e)
                .map(|(_, _, t)| *t)
        };
        assert_eq!(kind("#define"), Some(Token::Directive), "directive");
        assert_eq!(kind("// note"), Some(Token::Comment), "line comment");
        assert_eq!(kind("float"), Some(Token::Keyword), "type");
        assert_eq!(kind("TIME"), Some(Token::Builtin), "built-in uniform");
        assert_eq!(kind("2.0"), Some(Token::Number), "number");
        assert_eq!(kind("/* c */"), Some(Token::Comment), "block comment");
        assert_eq!(kind("syn_BassLevel"), Some(Token::Builtin), "audio uniform");
    }
}
