// ============================================================================
// Module:       xtask::symbolicate
// Description:  `cargo xtask symbolicate`: turns a crash minidump into a
//               readable stack using the build's own debug information.
//
// Dependencies: anyhow, std::fs, std::process; external dump_syms and
//               minidump-stackwalk (cargo install)
// ============================================================================

//! `cargo xtask symbolicate <dump.dmp> [--symbols FILE]... [--out FILE]`.
//!
//! A crash report's `.dmp` holds raw addresses. Two tools from the
//! rust-minidump project turn them into function names and `file:line`
//! without a Windows debugger. Both are installed with
//! `cargo install dump_syms minidump-stackwalk --locked`.
//!
//! 1. `dump_syms` reads a PDB on Windows, or an unstripped binary on Linux
//!    or macOS, and writes a Breakpad `.sym` file. Its first line is
//!    `MODULE <os> <arch> <id> <debug-file>`.
//! 2. `minidump-stackwalk` reads the dump. It looks for symbols at
//!    `<store>/<debug-file>/<id>/<name>.sym`, where `<name>` is the debug
//!    file with a trailing `.pdb` replaced by `.sym`. `dump_syms` does not
//!    create that layout itself, which is the step people get wrong. This
//!    command reads the `MODULE` line and files each `.sym` where the
//!    stackwalker will look.
//!
//! Symbols must come from the **same build** as the crashed executable.
//! The build id in the `MODULE` line is what ties the two together, and a
//! mismatched PDB simply yields unsymbolicated frames. Release archives
//! ship their PDBs separately for this reason (D53).
//!
//! Verified on the T480 (2026-10-10, Q11): `crash_demo overflow` symbolicated
//! to `crash_demo::recurse() [crash_demo.rs : 32]` called from
//! `crash_demo::main() [crash_demo.rs : 90]`.

use anyhow::{Context as _, Result, anyhow, bail};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the symbol store is built, under the workspace root.
const STORE: &str = "target/symbols";

/// The two external tools, and how to get them.
const DUMP_SYMS: &str = "dump_syms";
const STACKWALK: &str = "minidump-stackwalk";
const INSTALL_HINT: &str = "cargo install dump_syms minidump-stackwalk --locked";

/// Arguments for `cargo xtask symbolicate`.
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// The minidump to symbolicate (a `crash-*.dmp` from the crashes folder).
    dump: PathBuf,
    /// Debug information to use: a `.pdb` on Windows, or an unstripped
    /// binary elsewhere. Repeatable. By default, every PDB under
    /// `target/release` and `target/release/examples` is used, or on a
    /// platform without PDBs, the release `strobe` and `crash_demo`
    /// binaries.
    #[arg(long = "symbols", value_name = "FILE")]
    symbols: Vec<PathBuf>,
    /// Write the stack to this file instead of standard output.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
}

impl Args {
    /// The arguments with relative paths taken from `cwd`, the directory the
    /// command was run in. `main` changes into the workspace root before
    /// running anything, so a relative `crash.dmp` typed in a crashes
    /// folder inside the repository would otherwise be looked for at the
    /// root and not found.
    pub(crate) fn anchored(self, cwd: &Path) -> Self {
        Self {
            dump: cwd.join(self.dump),
            symbols: self
                .symbols
                .into_iter()
                .map(|path| cwd.join(path))
                .collect(),
            out: self.out.map(|path| cwd.join(path)),
        }
    }
}

/// A parsed `MODULE` line from a `.sym` file.
#[derive(Debug, PartialEq, Eq)]
struct Module {
    /// The build id that ties the symbols to one exact build.
    id: String,
    /// The debug file's name: `strobe.pdb`, or a binary's own name.
    debug_file: String,
}

impl Module {
    /// Reads the first line of a Breakpad symbol file.
    fn parse(sym: &str) -> Result<Self> {
        let line = sym.lines().next().unwrap_or_default();
        let mut fields = line.split_whitespace();
        let (Some("MODULE"), Some(_os), Some(_arch), Some(id), Some(debug_file)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            bail!("not a Breakpad symbol file (first line: {line:?})");
        };
        Ok(Self {
            id: id.to_string(),
            debug_file: debug_file.to_string(),
        })
    }

    /// Where the stackwalker looks for this module's symbols, relative to
    /// the store: `<debug-file>/<id>/<name>.sym`.
    fn store_path(&self) -> PathBuf {
        let name = match self.debug_file.strip_suffix(".pdb") {
            Some(stem) => format!("{stem}.sym"),
            None => format!("{}.sym", self.debug_file),
        };
        PathBuf::from(&self.debug_file).join(&self.id).join(name)
    }
}

/// Symbolicates `args.dump` and prints or writes the stack.
pub(crate) fn run(args: &Args) -> Result<()> {
    if !args.dump.is_file() {
        bail!("no minidump at {}", args.dump.display());
    }
    let dump_syms = tool(DUMP_SYMS)?;
    let stackwalk = tool(STACKWALK)?;
    let sources = if args.symbols.is_empty() {
        default_symbol_sources()?
    } else {
        args.symbols.clone()
    };
    if sources.is_empty() {
        bail!(
            "no debug information found under target/release; build in release first, or pass --symbols"
        );
    }

    let store = PathBuf::from(STORE);
    fs::create_dir_all(&store).with_context(|| format!("creating {}", store.display()))?;
    for source in &sources {
        let filed = file_symbols(&dump_syms, source, &store)?;
        eprintln!("symbols: {} -> {}", source.display(), filed.display());
    }

    let output = Command::new(&stackwalk)
        .arg("--symbols-path")
        .arg(&store)
        .arg(&args.dump)
        .output()
        .with_context(|| format!("failed to run {}", stackwalk.display()))?;
    if !output.status.success() {
        bail!(
            "{STACKWALK} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    match &args.out {
        Some(path) => {
            fs::write(path, &output.stdout)
                .with_context(|| format!("writing {}", path.display()))?;
            eprintln!("stack written to {}", path.display());
        }
        None => print!("{}", String::from_utf8_lossy(&output.stdout)),
    }
    Ok(())
}

/// Runs `dump_syms` on `source` and moves the result to where the
/// stackwalker looks. Returns the final path.
fn file_symbols(dump_syms: &Path, source: &Path, store: &Path) -> Result<PathBuf> {
    let scratch = store.join("incoming.sym");
    let status = Command::new(dump_syms)
        .arg(source)
        .arg("-o")
        .arg(&scratch)
        .status()
        .with_context(|| format!("failed to run {}", dump_syms.display()))?;
    if !status.success() {
        bail!("{DUMP_SYMS} failed on {} ({status})", source.display());
    }
    let text =
        fs::read_to_string(&scratch).with_context(|| format!("reading {}", scratch.display()))?;
    let module =
        Module::parse(&text).with_context(|| format!("symbols from {}", source.display()))?;
    let target = store.join(module.store_path());
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::rename(&scratch, &target)
        .with_context(|| format!("moving symbols to {}", target.display()))?;
    Ok(target)
}

/// The PDBs a release build leaves behind, or the unstripped binaries on a
/// platform without PDBs.
fn default_symbol_sources() -> Result<Vec<PathBuf>> {
    // A packaged build is built with `--target` (release.yml), so its PDB
    // is under the target triple, not `target/release` (T480 Task 21).
    let dirs = [
        Path::new("target/x86_64-pc-windows-msvc/release"),
        Path::new("target/release"),
        Path::new("target/release/examples"),
    ];
    let mut pdbs = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries {
            let path = entry
                .with_context(|| format!("listing {}", dir.display()))?
                .path();
            if path.extension() == Some(OsStr::new("pdb")) {
                pdbs.push(path);
            }
        }
    }
    if !pdbs.is_empty() {
        pdbs.sort();
        return Ok(pdbs);
    }
    // Release builds keep line tables and are not stripped (D42), so the
    // binaries themselves carry the debug information on Linux and macOS.
    let binaries = [
        "target/release/strobe",
        "target/release/examples/crash_demo",
    ];
    Ok(binaries
        .iter()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .collect())
}

/// Finds an external tool on `PATH`, or in cargo's own `bin` folder (where
/// `cargo install` puts it, and which is not always on `PATH`).
fn tool(name: &str) -> Result<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    if let Some(home) = cargo_home() {
        candidates.push(home.join("bin"));
    }
    candidates
        .into_iter()
        .map(|dir| dir.join(&file))
        .find(|path| path.is_file())
        .ok_or_else(|| anyhow!("{name} is not installed; run `{INSTALL_HINT}`"))
}

/// `$CARGO_HOME`, else `~/.cargo`.
fn cargo_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".cargo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pdb_module_files_under_its_pdb_name_with_a_sym_extension() -> Result<()> {
        // The MODULE line the T480 produced for crash_demo.pdb.
        let module = Module::parse(
            "MODULE windows x86_64 FE0078B7DE6E4387BA09076502B4D64C6 crash_demo.pdb\nFILE 0 x\n",
        )?;
        assert_eq!(
            module.store_path(),
            PathBuf::from("crash_demo.pdb")
                .join("FE0078B7DE6E4387BA09076502B4D64C6")
                .join("crash_demo.sym"),
            "the layout minidump-stackwalk reads"
        );
        Ok(())
    }

    #[test]
    fn an_elf_module_keeps_its_name_and_adds_sym() -> Result<()> {
        let module = Module::parse("MODULE Linux x86_64 0A1B2C3D0 strobe\n")?;
        assert_eq!(
            module.store_path(),
            PathBuf::from("strobe").join("0A1B2C3D0").join("strobe.sym"),
            "a binary's symbols are <name>/<id>/<name>.sym"
        );
        Ok(())
    }

    #[test]
    fn relative_paths_are_taken_from_where_the_command_was_run() -> Result<()> {
        let cwd = std::env::temp_dir().join("crashes");
        let elsewhere = std::env::temp_dir().join("strobe.pdb");
        let args = Args {
            dump: PathBuf::from("crash-20261017-061205.dmp"),
            symbols: vec![PathBuf::from("crash_demo.pdb"), elsewhere.clone()],
            out: Some(PathBuf::from("stack.txt")),
        }
        .anchored(&cwd);
        assert_eq!(args.dump, cwd.join("crash-20261017-061205.dmp"), "the dump");
        assert_eq!(
            args.symbols,
            vec![cwd.join("crash_demo.pdb"), elsewhere],
            "a relative symbol file moves, an absolute one does not"
        );
        assert_eq!(args.out, Some(cwd.join("stack.txt")), "the output");
        Ok(())
    }

    #[test]
    fn a_file_that_is_not_breakpad_symbols_is_refused() {
        for text in ["", "INFO CODE_ID 1234\n", "MODULE windows x86_64\n"] {
            assert!(
                Module::parse(text).is_err(),
                "{text:?} is not a MODULE line"
            );
        }
    }
}
