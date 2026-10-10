// ============================================================================
// Module:       strobe_autocontrol::source
// Description:  A scene's GLSL files as editable text, and a positioned token
//               view of one file: byte offsets, nesting depth, scannable regions.
//
// Dependencies: strobe-scene (SceneFolder), strobe-shadertoy (tokenizer, markers)
// ============================================================================

//! Source files and the scan view over one of them.
//!
//! [`Sources`] holds each distinct GLSL file a scene runs (`main.glsl`, or
//! one file per pass under `PASS_SOURCES`), keyed by the name `scene.json`
//! gives it, so a rewrite can put the edited text back where the player
//! will look for it.
//!
//! [`Scan`] is the view the analysis works on. It runs
//! `strobe_shadertoy::glsl::lex` (the lossless tokenizer) and adds what the
//! tokenizer leaves out: each token's byte offset, the brace and
//! parenthesis depth it sits at, and the **regions** that hold author code.
//! A converted Shadertoy pass file is a generated shim, then the author's
//! tabs, then a generated entry point; only the tabs are scanned, because a
//! literal in the shim is Strobe's, not the author's. A plain SSF file is
//! one region covering all of it.

use std::collections::BTreeMap;
use strobe_scene::SceneFolder;
use strobe_scene::load::MAIN_SHADER;
use strobe_shadertoy::convert::{CLOCK_MARKER, ENTRY_MARKER, TAB_MARKER_PREFIX, TAB_MARKER_SUFFIX};
use strobe_shadertoy::glsl::{self, Kind};

/// A scene's distinct GLSL files, by the name `scene.json` uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    /// File name to text, in first-use order of `order`.
    files: BTreeMap<String, String>,
    /// The names in pass order, without repeats.
    order: Vec<String>,
}

impl Sources {
    /// The files `folder` runs: one per distinct `PASS_SOURCES` entry, or
    /// `main.glsl` alone.
    pub fn from_folder(folder: &SceneFolder) -> Self {
        let mut files = BTreeMap::new();
        let mut order = Vec::new();
        let mut add = |name: &str, text: &str| {
            if !files.contains_key(name) {
                files.insert(name.to_string(), text.to_string());
                order.push(name.to_string());
            }
        };
        match &folder.def.pass_sources {
            Some(names) => {
                for (index, name) in names.iter().enumerate() {
                    add(name, folder.shader_for(index));
                }
            }
            None => add(MAIN_SHADER, &folder.shader),
        }
        Self { files, order }
    }

    /// The file names, in pass order.
    pub fn names(&self) -> &[String] {
        &self.order
    }

    /// One file's text.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.files.get(name).map(String::as_str)
    }

    /// Replaces one file's text. A name that is not one of the scene's
    /// files is ignored.
    pub fn set(&mut self, name: &str, text: String) {
        if let Some(slot) = self.files.get_mut(name) {
            *slot = text;
        }
    }

    /// `base` with these files' text in place of its own. `main.glsl` is
    /// updated even when it is also the final pass's file.
    pub fn apply_to(&self, base: &SceneFolder) -> SceneFolder {
        let mut out = base.clone();
        if let Some(main) = self.files.get(MAIN_SHADER) {
            out.shader.clone_from(main);
        }
        if let Some(names) = &base.def.pass_sources {
            out.pass_shaders = names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    self.files
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| base.shader_for(index).to_string())
                })
                .collect();
        }
        out
    }

    /// Every file as `(name, text)`, in pass order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.order.iter().filter_map(|name| {
            self.files
                .get(name)
                .map(|text| (name.as_str(), text.as_str()))
        })
    }
}

/// One token with its position.
#[derive(Debug, Clone, Copy)]
pub struct Tok<'a> {
    /// What it is.
    pub kind: Kind,
    /// The source text, verbatim.
    pub text: &'a str,
    /// Byte offset of its first byte.
    pub start: usize,
    /// Byte offset just past its last byte.
    pub end: usize,
}

/// A stretch of a file that holds author code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// First byte.
    pub start: usize,
    /// Exclusive end.
    pub end: usize,
    /// The Shadertoy tab the region is (`Common`, `Image`, `Buffer A`...),
    /// `None` for a plain SSF file.
    pub tab: Option<String>,
    /// 1-based line of the first line of the region's code.
    pub first_line: u32,
}

/// The regions of `text` that hold author code.
///
/// A file with Shadertoy tab markers has one region per tab, from the line
/// after its marker to the next marker (or the entry marker). The clock
/// pass has none. Any other file is a single region.
pub fn regions(text: &str) -> Vec<Region> {
    if text.starts_with(CLOCK_MARKER) {
        return Vec::new();
    }
    let mut markers: Vec<(usize, usize, Option<String>)> = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix(TAB_MARKER_PREFIX) {
            let name = rest.strip_suffix(TAB_MARKER_SUFFIX).unwrap_or(rest);
            markers.push((offset, offset + line.len(), Some(name.to_string())));
        } else if trimmed == ENTRY_MARKER {
            markers.push((offset, offset + line.len(), None));
        }
        offset += line.len();
    }
    if !markers.iter().any(|(_, _, name)| name.is_some()) {
        return vec![Region {
            start: 0,
            end: text.len(),
            tab: None,
            first_line: 1,
        }];
    }
    let mut found = Vec::new();
    for (index, (_, after, name)) in markers.iter().enumerate() {
        let Some(tab) = name else { continue };
        let end = markers
            .get(index + 1)
            .map_or(text.len(), |(next_start, _, _)| *next_start);
        let first_line = text.get(..*after).map_or(1, |head| {
            head.bytes().filter(|b| *b == b'\n').count() as u32 + 1
        });
        found.push(Region {
            start: *after,
            end,
            tab: Some(tab.clone()),
            first_line,
        });
    }
    found
}

/// A positioned, depth-annotated token view of one file.
#[derive(Debug)]
pub struct Scan<'a> {
    /// The file name, as `scene.json` names it.
    pub path: &'a str,
    /// The whole text.
    pub text: &'a str,
    /// Every token, trivia included, with offsets.
    pub toks: Vec<Tok<'a>>,
    /// Indices into `toks` of the tokens that are not whitespace or
    /// comments, and fall inside a region.
    pub sig: Vec<usize>,
    /// For each `sig` entry: `{` depth just before the token.
    pub brace: Vec<u32>,
    /// For each `sig` entry: `(` and `[` depth just before the token.
    pub paren: Vec<u32>,
    /// The author-code regions.
    pub regions: Vec<Region>,
    line_starts: Vec<usize>,
}

impl<'a> Scan<'a> {
    /// Tokenizes `text`.
    pub fn new(path: &'a str, text: &'a str) -> Self {
        let mut at = 0;
        let toks: Vec<Tok<'a>> = glsl::lex(text)
            .into_iter()
            .map(|token| {
                let start = at;
                at += token.text.len();
                Tok {
                    kind: token.kind,
                    text: token.text,
                    start,
                    end: at,
                }
            })
            .collect();
        let regions = regions(text);
        let mut sig = Vec::new();
        let mut brace = Vec::new();
        let mut paren = Vec::new();
        let (mut braces, mut parens) = (0_u32, 0_u32);
        for (index, tok) in toks.iter().enumerate() {
            match tok.kind {
                Kind::Space | Kind::LineComment | Kind::BlockComment => continue,
                Kind::Ident | Kind::Number | Kind::Punct | Kind::Directive => {}
            }
            // Depth is tracked over the whole file so a region that starts
            // after a shim still has the right nesting.
            let inside = regions
                .iter()
                .any(|r| tok.start >= r.start && tok.start < r.end);
            if inside {
                sig.push(index);
                brace.push(braces);
                paren.push(parens);
            }
            if tok.kind == Kind::Punct {
                match tok.text {
                    "{" => braces += 1,
                    "}" => braces = braces.saturating_sub(1),
                    "(" | "[" => parens += 1,
                    ")" | "]" => parens = parens.saturating_sub(1),
                    _ => {}
                }
            }
        }
        let mut line_starts = vec![0];
        line_starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        Self {
            path,
            text,
            toks,
            sig,
            brace,
            paren,
            regions,
            line_starts,
        }
    }

    /// How many significant tokens there are.
    pub fn len(&self) -> usize {
        self.sig.len()
    }

    /// Whether there are no significant tokens.
    pub fn is_empty(&self) -> bool {
        self.sig.is_empty()
    }

    /// The `i`th significant token.
    pub fn tok(&self, i: usize) -> Option<&Tok<'a>> {
        self.sig.get(i).and_then(|index| self.toks.get(*index))
    }

    /// Its text, or `""` past either end.
    pub fn t(&self, i: usize) -> &'a str {
        self.tok(i).map_or("", |tok| tok.text)
    }

    /// Its kind, if there is such a token.
    pub fn kind(&self, i: usize) -> Option<Kind> {
        self.tok(i).map(|tok| tok.kind)
    }

    /// Whether token `i` is the punctuation `p`.
    pub fn is(&self, i: usize, p: &str) -> bool {
        self.tok(i)
            .is_some_and(|tok| tok.kind == Kind::Punct && tok.text == p)
    }

    /// Whether token `i` is the identifier `name`.
    pub fn is_ident(&self, i: usize, name: &str) -> bool {
        self.tok(i)
            .is_some_and(|tok| tok.kind == Kind::Ident && tok.text == name)
    }

    /// Whether token `i` is a number.
    pub fn is_number(&self, i: usize) -> bool {
        self.kind(i) == Some(Kind::Number)
    }

    /// Whether token `i` is an identifier.
    pub fn is_any_ident(&self, i: usize) -> bool {
        self.kind(i) == Some(Kind::Ident)
    }

    /// Byte offset where token `i` starts.
    pub fn start(&self, i: usize) -> usize {
        self.tok(i).map_or(self.text.len(), |tok| tok.start)
    }

    /// Byte offset just past token `i`.
    pub fn end(&self, i: usize) -> usize {
        self.tok(i).map_or(self.text.len(), |tok| tok.end)
    }

    /// The brace depth just before token `i`.
    pub fn brace_depth(&self, i: usize) -> u32 {
        self.brace.get(i).copied().unwrap_or(0)
    }

    /// The parenthesis and bracket depth just before token `i`.
    pub fn paren_depth(&self, i: usize) -> u32 {
        self.paren.get(i).copied().unwrap_or(0)
    }

    /// 1-based `(line, column)` of a byte offset; the column counts
    /// characters.
    pub fn line_col(&self, offset: usize) -> (u32, u32) {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(found) => found,
            Err(insert) => insert.saturating_sub(1),
        };
        let line_start = self.line_starts.get(line).copied().unwrap_or(0);
        let column = self
            .text
            .get(line_start..offset)
            .map_or(0, |head| head.chars().count());
        (line as u32 + 1, column as u32 + 1)
    }

    /// The text of the line `offset` is on, trimmed, for reports.
    pub fn line_text(&self, offset: usize) -> String {
        let (line, _) = self.line_col(offset);
        let start = self
            .line_starts
            .get(line as usize - 1)
            .copied()
            .unwrap_or(0);
        let end = self
            .line_starts
            .get(line as usize)
            .copied()
            .unwrap_or(self.text.len());
        self.text
            .get(start..end)
            .unwrap_or("")
            .trim()
            .chars()
            .take(100)
            .collect()
    }

    /// The region containing byte `offset`.
    pub fn region_at(&self, offset: usize) -> Option<&Region> {
        self.regions
            .iter()
            .find(|region| offset >= region.start && offset < region.end)
    }

    /// The identity of an offset that survives a Shadertoy `Common` tab
    /// being copied into every pass file: the tab name and the line and
    /// column within the tab.
    pub fn origin(&self, offset: usize) -> String {
        let (line, column) = self.line_col(offset);
        match self.region_at(offset) {
            Some(Region {
                tab: Some(tab),
                first_line,
                ..
            }) => format!("{tab}:{}:{column}", line.saturating_sub(*first_line) + 1),
            Some(Region { tab: None, .. }) | None => {
                format!("{}:{line}:{column}", self.path)
            }
        }
    }

    /// The first significant token of the statement token `i` is in: the
    /// token after the previous `;`, `{`, `}` or directive.
    pub fn statement_start(&self, i: usize) -> usize {
        let mut at = i;
        while at > 0 {
            let before = at - 1;
            let boundary = self.is(before, ";")
                || self.is(before, "{")
                || self.is(before, "}")
                || self.kind(before) == Some(Kind::Directive);
            if boundary {
                break;
            }
            at = before;
        }
        at
    }

    /// The operator that starts at token `i`: two characters when the next
    /// token is adjacent and the pair is an operator (`<=`, `*=`, `++`...),
    /// otherwise just the one. The tokenizer emits one punctuation
    /// character at a time.
    pub fn op(&self, i: usize) -> &'a str {
        const PAIRS: [&str; 16] = [
            "<=", ">=", "==", "!=", "&&", "||", "^^", "+=", "-=", "*=", "/=", "%=", "<<", ">>",
            "++", "--",
        ];
        let (Some(first), Some(second)) = (self.tok(i), self.tok(i + 1)) else {
            return self.t(i);
        };
        if first.kind == Kind::Punct && second.kind == Kind::Punct && first.end == second.start {
            let pair = self.text.get(first.start..second.end).unwrap_or("");
            if PAIRS.contains(&pair) {
                return pair;
            }
        }
        first.text
    }

    /// The index of the token that closes the bracket opened at `open`
    /// (`(`, `[` or `{`), if it is closed.
    pub fn matching(&self, open: usize) -> Option<usize> {
        let (opening, closing) = match self.t(open) {
            "(" => ("(", ")"),
            "[" => ("[", "]"),
            "{" => ("{", "}"),
            _ => return None,
        };
        let mut depth = 0_u32;
        for at in open..self.len() {
            if self.is(at, opening) {
                depth += 1;
            } else if self.is(at, closing) {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
        }
        None
    }

    /// The text of the whole line `offset` is on, from its start to the
    /// end of the line, without the newline.
    pub fn rest_of_line(&self, offset: usize) -> &'a str {
        let tail = self.text.get(offset..).unwrap_or("");
        tail.split('\n').next().unwrap_or("")
    }

    /// The text from the start of `offset`'s line up to `offset`.
    pub fn line_head(&self, offset: usize) -> &'a str {
        let head = self.text.get(..offset).unwrap_or("");
        match head.rfind('\n') {
            Some(newline) => head.get(newline + 1..).unwrap_or(""),
            None => head,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_plain_file_is_one_region_and_positions_are_one_based() {
        let scan = Scan::new("main.glsl", "float a;\n  float b = 1.0;\n");
        assert_eq!(scan.regions.len(), 1, "no markers: one region");
        let start = scan.text.find("1.0").unwrap_or(0);
        assert_eq!(scan.line_col(start), (2, 13), "line 2, column 13");
        assert_eq!(scan.line_col(0), (1, 1), "the very start");
    }

    #[test]
    fn a_converted_file_scans_only_the_author_tabs() -> Result<()> {
        let text = "float shim = 0.25;\n// ---- Shadertoy tab: Common ----\nfloat c = 3.0;\n\
                    // ---- Shadertoy tab: Image ----\nfloat i = 4.0;\n\
                    // ---- Strobe entry (generated) ----\nfloat e = 5.0;\n";
        let scan = Scan::new("main.glsl", text);
        let tabs: Vec<Option<&str>> = scan.regions.iter().map(|r| r.tab.as_deref()).collect();
        assert_eq!(tabs, vec![Some("Common"), Some("Image")], "two tabs");
        let numbers: Vec<&str> = (0..scan.len())
            .filter(|i| scan.is_number(*i))
            .map(|i| scan.t(i))
            .collect();
        assert_eq!(
            numbers,
            vec!["3.0", "4.0"],
            "the shim and the entry point are not scanned"
        );
        let regions = &scan.regions;
        assert_eq!(regions[0].first_line, 3, "Common starts on line 3");
        assert_eq!(regions[1].first_line, 5, "Image starts on line 5");
        Ok(())
    }

    #[test]
    fn the_clock_pass_has_no_author_code() {
        let text = format!("{CLOCK_MARKER}\nfloat k = 2.0;\n");
        assert!(regions(&text).is_empty(), "nothing to scan in the clock");
    }

    #[test]
    fn a_common_tab_has_the_same_origin_in_every_pass_file() {
        let a = "shim a\n// ---- Shadertoy tab: Common ----\nconst float K = 3.0;\n// ---- Shadertoy tab: Image ----\n";
        let b = "a much longer shim\nwith more lines\n// ---- Shadertoy tab: Common ----\nconst float K = 3.0;\n// ---- Shadertoy tab: Buffer A ----\n";
        let origin = |text: &str| {
            let scan = Scan::new("f.glsl", text);
            scan.origin(text.find("3.0").unwrap_or(0))
        };
        assert_eq!(
            origin(a),
            origin(b),
            "Common is one place, however many copies"
        );
    }

    #[test]
    fn depth_tracks_braces_and_parentheses() {
        let scan = Scan::new("f.glsl", "void f() { g(1.0); }\nfloat k = 2.0;\n");
        let at = |needle: &str| {
            (0..scan.len())
                .find(|i| scan.t(*i) == needle)
                .unwrap_or(usize::MAX)
        };
        assert_eq!(scan.brace_depth(at("1.0")), 1, "inside the function body");
        assert_eq!(scan.paren_depth(at("1.0")), 1, "inside the call");
        assert_eq!(scan.brace_depth(at("2.0")), 0, "after the function");
    }

    #[test]
    fn matching_finds_the_closing_bracket() {
        let scan = Scan::new("f.glsl", "for (int i = 0; i < f(2); i++) { x[1]; }");
        let open = (0..scan.len()).find(|i| scan.is(*i, "(")).unwrap_or(0);
        let close = scan.matching(open).unwrap_or(0);
        assert_eq!(scan.t(close), ")", "the loop header closes");
        assert_eq!(scan.t(close + 1), "{", "and the body follows");
    }
}
