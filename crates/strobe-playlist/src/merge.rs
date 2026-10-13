// ============================================================================
// Module:       strobe_playlist::merge
// Description:  Saving over a hand-written playlist without losing its
//               comments: the new text's values in the old text's layout
//               wherever the two still agree.
//
// Dependencies: toml_edit, toml
// ============================================================================

//! Keeping a person's comments (backlog U-07, D231).
//!
//! The app edits a playlist as a model and [`crate::write`] lays the model
//! out canonically, which drops every comment a person wrote in the file.
//! [`keep_layout`] takes the file as it was and the canonical text of the
//! edited model and returns the canonical *content* in the old *layout*:
//!
//! - a value that did not change keeps its text exactly as written (`90`
//!   stays `90` rather than becoming `"90s"`), with its comments;
//! - a value that changed takes the new text but keeps the comments
//!   around it;
//! - in an array (a section's cues), elements are matched by content, so
//!   inserting, deleting or moving a cue keeps every other cue's comment
//!   with that cue, not with whatever now sits at its index;
//! - keys and tables that are new are added canonically, and ones that are
//!   gone are removed with their comments.
//!
//! The caller checks that the result still reads back to the same model
//! and falls back to the canonical text if it does not
//! ([`crate::file::save`]), so a mistake here can cost comments,
//! never the show.

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};

/// `fresh` (canonical text) with `original`'s layout and comments wherever
/// they still fit. Either text failing to parse gives `fresh` unchanged.
pub fn keep_layout(original: &str, fresh: &str) -> String {
    let (Ok(mut kept), Ok(new)) = (
        original.parse::<DocumentMut>(),
        fresh.parse::<DocumentMut>(),
    ) else {
        return fresh.to_string();
    };
    merge_table(kept.as_table_mut(), new.as_table());
    kept.to_string()
}

fn merge_table(kept: &mut Table, new: &Table) {
    let gone: Vec<String> = kept
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| !new.contains_key(key))
        .collect();
    for key in gone {
        kept.remove(&key);
    }
    for (key, item) in new.iter() {
        match kept.get_mut(key) {
            Some(old) => merge_item(old, item),
            None => {
                kept.insert(key, item.clone());
            }
        }
    }
}

fn merge_item(kept: &mut Item, new: &Item) {
    match (kept, new) {
        (Item::Table(old), Item::Table(table)) => merge_table(old, table),
        (Item::ArrayOfTables(old), Item::ArrayOfTables(tables)) => merge_tables(old, tables),
        (Item::Value(Value::Array(old)), Item::Value(Value::Array(array))) => {
            merge_array(old, array);
        }
        (Item::Value(old), Item::Value(value)) => merge_value(old, value),
        (old, item) => *old = item.clone(),
    }
}

/// `[[sections]]`: matched by position (a section has a name, but two may
/// share one, and sections are rarely reordered).
fn merge_tables(kept: &mut ArrayOfTables, new: &ArrayOfTables) {
    while kept.len() > new.len() {
        kept.remove(kept.len() - 1);
    }
    for (index, table) in new.iter().enumerate() {
        match kept.get_mut(index) {
            Some(old) => merge_table(old, table),
            None => kept.push(table.clone()),
        }
    }
}

/// A value that is the same keeps its text; one that changed takes the new
/// text inside the old comments.
fn merge_value(kept: &mut Value, new: &Value) {
    if same(kept, new) {
        return;
    }
    let decor = kept.decor().clone();
    *kept = new.clone();
    *kept.decor_mut() = decor;
}

/// An array's elements, matched by content (see the module docs).
fn merge_array(kept: &mut Array, new: &Array) {
    let old: Vec<Value> = kept.iter().cloned().collect();
    let mut next = 0_usize;
    let mut out: Vec<Value> = Vec::with_capacity(new.len());
    for (position, value) in new.iter().enumerate() {
        // The same element further on: keep it, comments and all.
        let found = (next..old.len()).find(|&i| old.get(i).is_some_and(|o| same(o, value)));
        if let Some(i) = found {
            if let Some(o) = old.get(i) {
                out.push(o.clone());
            }
            next = i + 1;
            continue;
        }
        // The element at this point, edited: the same thing (a cue for the
        // same scene) with other settings. The new value in its comments.
        let current = old.get(next).filter(|o| similar(o, value));
        // Otherwise it is new here, unless the old element appears nowhere
        // later, in which case it was replaced in place.
        let replaced = current.or_else(|| {
            old.get(next).filter(|o| {
                !new.iter()
                    .skip(position + 1)
                    .any(|later| same(o, later) || similar(o, later))
            })
        });
        match replaced {
            Some(o) => {
                let mut changed = value.clone();
                *changed.decor_mut() = o.decor().clone();
                out.push(changed);
                next += 1;
            }
            None => out.push(value.clone()),
        }
    }
    let trailing = kept.trailing().clone();
    let comma = kept.trailing_comma();
    let decor = kept.decor().clone();
    kept.clear();
    for value in out {
        kept.push_formatted(value);
    }
    kept.set_trailing(trailing);
    kept.set_trailing_comma(comma || new.trailing_comma());
    *kept.decor_mut() = decor;
}

/// Whether two values are the same element, edited: inline tables whose
/// first entry (a cue's `scene`) agrees.
fn similar(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::InlineTable(x), Value::InlineTable(y)) => {
            match (x.iter().next(), y.iter().next()) {
                (Some((kx, vx)), Some((ky, vy))) => kx == ky && same(vx, vy),
                (None, _) | (_, None) => false,
            }
        }
        (
            Value::String(_)
            | Value::Integer(_)
            | Value::Float(_)
            | Value::Boolean(_)
            | Value::Datetime(_)
            | Value::Array(_)
            | Value::InlineTable(_),
            _,
        ) => false,
    }
}

/// Whether two values mean the same, however they are written.
fn same(a: &Value, b: &Value) -> bool {
    let read = |value: &Value| {
        let mut bare = value.clone();
        bare.decor_mut().clear();
        format!("v = {bare}").parse::<toml::Table>().ok()
    };
    match (read(a), read(b)) {
        (Some(x), Some(y)) => x == y,
        (None, _) | (_, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: &str = r#"# Friday night, main room.
name = "Lobby"
loop_mode = "loop"

[defaults]
duration = 90 # a minute and a half, as typed

# The warm-up.
[[sections]]
name = "Early"
cues = [
  # opener
  { scene = "a" },
  # the slow one
  { scene = "b", duration = "3m" },
  # closer
  { scene = "c" },
]
"#;

    #[test]
    fn unchanged_text_is_kept_exactly() {
        assert_eq!(keep_layout(ORIGINAL, ORIGINAL), ORIGINAL, "itself");
    }

    #[test]
    fn an_edit_keeps_every_comment_with_its_cue() {
        // b's length changes, a new cue goes in after a, and the duration
        // is written canonically.
        let fresh = r#"name = "Lobby"
loop_mode = "loop"

[defaults]
duration = "90s"

[[sections]]
name = "Early"
cues = [
  { scene = "a" },
  { scene = "new" },
  { scene = "b", duration = "4m" },
  { scene = "c" },
]
"#;
        let merged = keep_layout(ORIGINAL, fresh);
        assert!(merged.starts_with("# Friday night"), "the header: {merged}");
        assert!(merged.contains("# The warm-up."), "the section comment");
        assert!(
            merged.contains("# opener\n  { scene = \"a\" }"),
            "a keeps its comment: {merged}"
        );
        assert!(
            merged.contains("# the slow one\n  { scene = \"b\", duration = \"4m\" }"),
            "b keeps its comment with its new length: {merged}"
        );
        assert!(
            merged.contains("# closer\n  { scene = \"c\" }"),
            "c keeps its comment: {merged}"
        );
        assert!(
            merged.contains("{ scene = \"new\" }"),
            "the new cue is there"
        );
        assert!(
            merged.contains("duration = \"90s\" # a minute and a half"),
            "a changed value keeps its trailing comment: {merged}"
        );
        let a = merged.find("\"a\"").unwrap_or(usize::MAX);
        let new = merged.find("\"new\"").unwrap_or(0);
        assert!(a < new, "in the new order");
    }

    #[test]
    fn a_deleted_cue_takes_its_comment_and_a_bad_original_is_ignored() {
        let fresh = ORIGINAL.replace(
            "  # the slow one\n  { scene = \"b\", duration = \"3m\" },\n",
            "",
        );
        let merged = keep_layout(ORIGINAL, &fresh);
        assert!(!merged.contains("slow one"), "gone with b: {merged}");
        assert!(merged.contains("# closer"), "c's stays");
        assert_eq!(
            keep_layout("not = [toml", "a = 1\n"),
            "a = 1\n",
            "fresh as is"
        );
    }
}
