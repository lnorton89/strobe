// ============================================================================
// Module:       strobe_shadertoy (binary crate root)
// Description:  `strobe-import`: converts an exported Shadertoy JSON file into
//               a Strobe scene folder.
//
// Dependencies: clap, anyhow; strobe_shadertoy (the library half)
// ============================================================================

//! `strobe-import`.
//!
//! ```text
//! strobe-import shader.json --out scenes
//! strobe-import shader.json --out scenes --name "My tunnel" --assets ~/Downloads/media
//! strobe-import --id https://www.shadertoy.com/view/XsXXDn      (not built yet)
//! ```
//!
//! Reads one exported Shadertoy JSON (the API's `{"Shader": ...}` or the
//! bare export), converts it, and writes `<name>-<id>.synScene` into the
//! output folder atomically. Textures the shader needs are looked up by
//! file name in `--assets`; any not found are listed in the scene's
//! `images/pending-assets.json` with their download URLs and read black
//! until the import is repeated with the files present.

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use std::collections::BTreeMap;
use std::path::PathBuf;
use strobe_shadertoy::{Options, Shader, ShaderId, convert};

/// The command line.
#[derive(Parser, Debug)]
#[command(
    name = "strobe-import",
    about = "Convert a Shadertoy shader (exported JSON) into a Strobe scene folder"
)]
struct Cli {
    /// An exported Shadertoy JSON file.
    input: Option<PathBuf>,
    /// Fetch a shader by id or URL. Not built yet.
    #[arg(long, conflicts_with = "input")]
    id: Option<String>,
    /// The scene library to write into.
    #[arg(long, default_value = "scenes")]
    out: PathBuf,
    /// The scene's title (default: the shader's own name).
    #[arg(long)]
    name: Option<String>,
    /// A folder of already downloaded texture files, found by file name.
    #[arg(long)]
    assets: Option<PathBuf>,
    /// Fail when a texture is missing instead of binding black.
    #[arg(long)]
    strict: bool,
    /// Replace the scene if it was imported before.
    #[arg(long)]
    force: bool,
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("strobe-import: {error:#}");
            std::process::ExitCode::from(2)
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Some(text) = &cli.id {
        let id = ShaderId::parse(text)?;
        bail!(
            "fetching {} by id is not built yet. The plan is the Shadertoy REST API with your \
             API key (path B in docs/PLAN.md section 6.1: GET {} on www.shadertoy.com) for \
             shaders published as \"Public + API\", and a browser userscript for the rest. \
             For now open {} in your browser, save the shader's JSON, and run: \
             strobe-import <file.json>",
            id,
            id.api_path(),
            id.page_url()
        );
    }
    let Some(input) = &cli.input else {
        bail!("give an exported JSON file to convert (strobe-import <file.json>), or --id");
    };

    let text = std::fs::read_to_string(input)
        .with_context(|| format!("cannot read {}", input.display()))?;
    let shader = Shader::from_json(&text).with_context(|| format!("{}", input.display()))?;

    let mut assets = BTreeMap::new();
    if let Some(dir) = &cli.assets {
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("cannot read assets folder {}", dir.display()))?;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_file()
                && let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned())
                && let Ok(bytes) = std::fs::read(&path)
            {
                assets.insert(name, bytes);
            }
        }
    }

    let converted = convert(
        &shader,
        &Options {
            name: cli.name.clone(),
            assets,
            strict_assets: cli.strict,
            original_json: Some(text),
        },
    )?;
    let written = strobe_shadertoy::convert::write(&converted, &cli.out, cli.force)?;

    for warning in &converted.warnings {
        eprintln!("warning: {warning}");
    }
    println!("{}", written.display());
    eprintln!(
        "imported \"{}\": {} file(s), {} warning(s), {} texture(s) still to download",
        converted.title,
        converted.files.len(),
        converted.warnings.len(),
        converted.pending_assets.len()
    );
    Ok(())
}
