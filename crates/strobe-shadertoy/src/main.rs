// ============================================================================
// Module:       strobe_shadertoy (binary crate root)
// Description:  `strobe-import`: converts an exported Shadertoy JSON file, or
//               a shader fetched by id, into a Strobe scene folder.
//
// Dependencies: clap, anyhow; strobe_shadertoy (the library half);
//               strobe-diag (the data directory)
// ============================================================================

//! `strobe-import`.
//!
//! ```text
//! strobe-import shader.json --out scenes
//! strobe-import shader.json --out scenes --name "My tunnel" --assets ~/Downloads/media
//! strobe-import --id https://www.shadertoy.com/view/XsXXDn --out scenes
//! strobe-import --id XsXXDn --offline        (leave textures black)
//! ```
//!
//! **From a file** (path C): reads one exported Shadertoy JSON (the API's
//! `{"Shader": ...}` or the bare export) and converts it.
//!
//! **By id** (path B): fetches `GET /api/v1/shaders/<ID>?key=<KEY>`. The
//! key comes from `STROBE_SHADERTOY_KEY` or the one-line file `shadertoy.key`
//! in Strobe's data directory (`--data-dir`, by default the folder that
//! holds the logs). Only shaders published "Public + API" are reachable
//! this way, and Shadertoy may refuse automated clients outright; the
//! browser userscript is the route for everything else.
//!
//! Either way the scene is written as `<name>-<id>.synScene` into `--out`,
//! atomically. Textures the shader needs are taken from `--assets` (by file
//! name) or the cache, then **downloaded** from Shadertoy (10 s each) unless
//! `--offline`; any still missing read black and are listed in the scene's
//! `images/pending-assets.json`.

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use std::collections::BTreeMap;
use std::path::PathBuf;
use strobe_shadertoy::ShaderId;
use strobe_shadertoy::acquire::{ApiKey, AssetCache, Endpoints, UreqFetch};
use strobe_shadertoy::import::{
    Downloader, ImportError, ImportOptions, Imported, import_file, import_id,
};

/// The command line.
#[derive(Parser, Debug)]
#[command(
    name = "strobe-import",
    about = "Convert a Shadertoy shader (exported JSON, or fetched by id) into a Strobe scene folder"
)]
struct Cli {
    /// An exported Shadertoy JSON file.
    input: Option<PathBuf>,
    /// Fetch a shader by id or URL through the REST API (needs an API key).
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
    /// Do not download textures; any not found read black.
    #[arg(long)]
    offline: bool,
    /// Fail when a texture is missing instead of binding black.
    #[arg(long)]
    strict: bool,
    /// Replace the scene if it was imported before.
    #[arg(long)]
    force: bool,
    /// Where the API key and the texture cache live (default: the folder
    /// that holds Strobe's logs).
    #[arg(long)]
    data_dir: Option<PathBuf>,
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
    let data_dir = cli
        .data_dir
        .clone()
        .unwrap_or_else(strobe_diag::paths::base_dir);
    let endpoints = Endpoints::from_env();
    let fetch = UreqFetch::new();
    let cache = AssetCache::in_data_dir(&data_dir);

    let mut local_assets = BTreeMap::new();
    if let Some(dir) = &cli.assets {
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("cannot read assets folder {}", dir.display()))?;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_file()
                && let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned())
                && let Ok(bytes) = std::fs::read(&path)
            {
                local_assets.insert(name, bytes);
            }
        }
    }
    let options = ImportOptions {
        library: &cli.out,
        replace: cli.force,
        name: cli.name.clone(),
        local_assets,
        strict_assets: cli.strict,
        downloader: (!cli.offline).then_some(Downloader {
            fetch: &fetch,
            endpoints: &endpoints,
            cache: &cache,
        }),
    };

    let result = if let Some(text) = &cli.id {
        let id = ShaderId::parse(text)?;
        let key = ApiKey::find(&data_dir)?;
        import_id(&id, &key, &fetch, &endpoints, &options)
    } else if let Some(input) = &cli.input {
        import_file(input, &options)
    } else {
        bail!("give an exported JSON file to convert (strobe-import <file.json>), or --id");
    };
    let done = match result {
        Ok(done) => done,
        Err(ImportError::Exists { dir, .. }) => {
            bail!(
                "{} already exists (use --force to replace it)",
                dir.display()
            )
        }
        Err(error) => return Err(error.into()),
    };
    report(&done);
    Ok(())
}

fn report(done: &Imported) {
    for warning in &done.warnings {
        eprintln!("warning: {warning}");
    }
    println!("{}", done.dir.display());
    eprintln!(
        "imported \"{}\"{}: {} file(s), {} warning(s), {} texture(s) found, {} still missing",
        done.title,
        if done.replaced { " (replaced)" } else { "" },
        done.files,
        done.warnings.len(),
        done.textures_found,
        done.textures_missing
    );
}
