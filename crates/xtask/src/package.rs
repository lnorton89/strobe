// ============================================================================
// Module:       xtask::package
// Description:  `cargo xtask package` builds the release zip and its symbols
//               zip from a build; `cargo xtask package-check` unpacks a zip
//               outside the checkout and proves it is what a user should get.
//
// Dependencies: anyhow, clap, serde_json; crate::{credits, native}; Windows'
//               tar.exe (zip) and PowerShell (the version resource)
// ============================================================================

//! The package, built and checked by the same code everywhere (D537).
//!
//! The release workflow used to stage the zip with an inline bash script,
//! and nothing checked the result except a user. So CI never built the
//! package a release would ship, and nothing ran the exact archive.
//! `package` stages from a build folder and zips; `package-check` takes
//! any zip, unpacks it into a temporary folder (never the checkout, so
//! nothing it runs can find a file the user would not have), and checks:
//!
//! - **the file list**: the program, the four native DLLs and their licence
//!   file, the scene library (every scene in `scenes/`), README, LICENSE,
//!   THIRD_PARTY.md and the full notices, and nothing else;
//! - **the native DLLs** against `scripts/native-deps.json` (D536);
//! - **the notices** are the generated file, not an empty one;
//! - **the version resource** on `strobe.exe` names the workspace version,
//!   so Explorer's Properties says which build it is (D244);
//! - **the symbols**: the `.pdb` in the symbols zip has the GUID the
//!   program's CodeView record names, so a minidump from this program
//!   symbolicates against it and against nothing else;
//! - **it runs**: `strobe.exe --diagnose` from the unpacked folder exits,
//!   prints its report, and says the DLLs beside it match the pin.
//!
//! Windows only: the package is a Windows package, built with Windows'
//! own `tar.exe` (bsdtar, which writes and reads zip).

use crate::native;
use anyhow::{Context as _, Result, anyhow, bail};
use clap::Args as ClapArgs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The files at the top of the package besides the native ones and
/// `scenes/`.
const DOCS: [&str; 4] = [
    "README.md",
    "LICENSE",
    "THIRD_PARTY.md",
    "THIRD-PARTY-NOTICES.txt",
];

/// The platform the package is for.
const TRIPLE: &str = "x86_64-pc-windows-msvc";

/// Options for `cargo xtask package`.
#[derive(ClapArgs, Debug)]
pub(crate) struct PackageArgs {
    /// The cargo profile whose `strobe.exe` and `strobe.pdb` are packaged.
    #[arg(long, default_value = "release")]
    profile: String,
    /// The target the build was made for with `--target`, if any; the
    /// binaries are then under `target/<target>/<profile>`.
    #[arg(long)]
    target: Option<String>,
    /// The version in the archive's name, such as `v0.2.0`. Defaults to the
    /// workspace version with a `-dev` suffix.
    #[arg(long)]
    tag: Option<String>,
    /// Where the native DLLs are. Defaults to `target/<profile>`, where
    /// `scripts/fetch-angle.ps1 -Profile <profile>` puts them.
    #[arg(long)]
    angle_dir: Option<PathBuf>,
    /// Where the staging folders and the two zips go.
    #[arg(long, default_value = "dist")]
    out: PathBuf,
}

/// Options for `cargo xtask package-check`.
#[derive(ClapArgs, Debug)]
pub(crate) struct CheckArgs {
    /// The package zip.
    zip: PathBuf,
    /// The matching symbols zip, to check the `.pdb` belongs to the program.
    #[arg(long)]
    symbols: Option<PathBuf>,
}

impl PackageArgs {
    /// The options CI's `package` lane uses: `profile`'s build, its own
    /// ANGLE folder, a dev tag, into `out`.
    pub(crate) fn for_lane(profile: &str, out: PathBuf) -> Self {
        Self {
            profile: profile.to_owned(),
            target: None,
            tag: None,
            angle_dir: None,
            out,
        }
    }

    /// Paths a person typed, taken from where they typed them.
    pub(crate) fn anchored(self, invoked_from: &Path) -> Self {
        Self {
            angle_dir: self.angle_dir.map(|dir| invoked_from.join(dir)),
            out: invoked_from.join(self.out),
            ..self
        }
    }
}

impl CheckArgs {
    /// Paths a person typed, taken from where they typed them.
    pub(crate) fn anchored(self, invoked_from: &Path) -> Self {
        Self {
            zip: invoked_from.join(self.zip),
            symbols: self.symbols.map(|zip| invoked_from.join(zip)),
        }
    }

    /// The options for checking `zip` and `symbols` from code.
    pub(crate) fn new(zip: PathBuf, symbols: Option<PathBuf>) -> Self {
        Self { zip, symbols }
    }
}

/// `cargo xtask package`: stages and zips; prints both zips' paths.
pub(crate) fn run_package(args: &PackageArgs) -> Result<()> {
    let (zip, symbols) = package(args)?;
    println!("{}\n{}", zip.display(), symbols.display());
    Ok(())
}

/// Stages the package and its symbols from a build and zips both. Returns
/// the two zips.
pub(crate) fn package(args: &PackageArgs) -> Result<(PathBuf, PathBuf)> {
    if !cfg!(windows) {
        bail!("the package is a Windows package; build it on Windows");
    }
    let root = crate::workspace_root()?;
    let tag = args
        .tag
        .clone()
        .unwrap_or_else(|| format!("v{}-dev", env!("CARGO_PKG_VERSION")));
    let triple = args.target.as_deref().unwrap_or(TRIPLE);
    let name = format!("strobe-{tag}-{triple}");
    let bin_dir = match &args.target {
        Some(target) => root.join("target").join(target).join(&args.profile),
        None => root.join("target").join(&args.profile),
    };
    let angle_dir = args
        .angle_dir
        .clone()
        .unwrap_or_else(|| root.join("target").join(&args.profile));

    std::fs::create_dir_all(&args.out)?;
    let staging = args.out.join(&name);
    let symbols = args.out.join(format!("{name}-symbols"));
    for folder in [&staging, &symbols] {
        // A leftover from an earlier run must not slip into this package.
        let _ = std::fs::remove_dir_all(folder);
        std::fs::create_dir_all(folder)?;
    }

    copy(&bin_dir.join("strobe.exe"), &staging.join("strobe.exe"))?;
    copy(&bin_dir.join("strobe.pdb"), &symbols.join("strobe.pdb"))?;
    let manifest = native::manifest()?;
    for (file, pinned) in native_files(&manifest)? {
        let from = angle_dir.join(&file);
        let actual = native::sha256(&from)?;
        if actual != pinned {
            bail!(
                "{} is {actual}, not the pinned {pinned}; run scripts\\fetch-angle.ps1",
                from.display()
            );
        }
        copy(&from, &staging.join(&file))?;
    }
    let licence = licence_file(&manifest)?;
    copy(&angle_dir.join(&licence), &staging.join(&licence))?;

    let notices = root.join("target").join("THIRD-PARTY-NOTICES.txt");
    crate::credits::run(false, Some(&notices))?;
    for doc in DOCS {
        let from = if doc == "THIRD-PARTY-NOTICES.txt" {
            notices.clone()
        } else {
            root.join(doc)
        };
        copy(&from, &staging.join(doc))?;
    }
    copy_tree(&root.join("scenes"), &staging.join("scenes"))?;

    let zip = args.out.join(format!("{name}.zip"));
    let symbols_zip = args.out.join(format!("{name}-symbols.zip"));
    for (folder, archive) in [(&name, &zip), (&format!("{name}-symbols"), &symbols_zip)] {
        // tar adds to an existing archive; absent is as good as removed.
        let _ = std::fs::remove_file(archive);
        tar(&["-a", "-c", "-f"], archive, &args.out, Some(folder))?;
    }
    Ok((zip, symbols_zip))
}

/// The pinned native files, `(name, sha256)`.
fn native_files(manifest: &serde_json::Value) -> Result<Vec<(String, String)>> {
    manifest["angle"]["files"]
        .as_array()
        .context("the manifest has no angle.files")?
        .iter()
        .map(|file| {
            Ok((
                file["name"]
                    .as_str()
                    .context("a file without a name")?
                    .to_owned(),
                file["sha256"]
                    .as_str()
                    .context("a file without a sha256")?
                    .to_owned(),
            ))
        })
        .collect()
}

/// The licence file that travels with the native files.
fn licence_file(manifest: &serde_json::Value) -> Result<String> {
    manifest["angle"]["licence_file"]
        .as_str()
        .map(str::to_owned)
        .context("the manifest has no angle.licence_file")
}

/// Copies one file, naming it on failure.
fn copy(from: &Path, to: &Path) -> Result<()> {
    std::fs::copy(from, to)
        .map(|_| ())
        .with_context(|| format!("cannot copy {} to {}", from.display(), to.display()))
}

/// Copies a folder and everything under it.
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in
        std::fs::read_dir(from).with_context(|| format!("cannot list {}", from.display()))?
    {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            copy(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Runs Windows' `tar.exe`: `tar <flags> <archive> -C <dir> [member]`.
/// `-a` with a `.zip` name writes a zip; `-x` reads one.
fn tar(flags: &[&str], archive: &Path, dir: &Path, member: Option<&str>) -> Result<()> {
    let mut command = Command::new("tar");
    command.args(flags).arg(archive).arg("-C").arg(dir);
    if let Some(member) = member {
        command.arg(member);
    }
    let status = command.status().context("failed to run tar")?;
    if !status.success() {
        bail!("tar exited with {status} for {}", archive.display());
    }
    Ok(())
}

/// What a check found.
#[derive(Debug, Default)]
pub(crate) struct Checked {
    /// The unpacked package's top folder.
    pub(crate) root: PathBuf,
    /// One line per check that passed.
    pub(crate) passed: Vec<String>,
    /// One line per problem.
    pub(crate) problems: Vec<String>,
    /// `strobe.exe --diagnose`'s report, when it ran.
    pub(crate) diagnose: Option<String>,
}

/// `cargo xtask package-check`: fails on any problem.
pub(crate) fn run_check(args: &CheckArgs) -> Result<()> {
    let checked = check(args, &scratch("package-check"))?;
    for line in &checked.passed {
        println!("ok       {line}");
    }
    for line in &checked.problems {
        println!("PROBLEM  {line}");
    }
    if !checked.problems.is_empty() {
        bail!(
            "{} problem(s) with {}",
            checked.problems.len(),
            args.zip.display()
        );
    }
    println!("\n{} is what a user should get.", args.zip.display());
    Ok(())
}

/// A fresh folder in the system temp directory, outside the checkout.
pub(crate) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("strobe-{name}-{}", std::process::id()));
    // Fresh each time; a leftover from a crashed run is not this run's.
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Unpacks `args.zip` (and its symbols) into `work` and checks it.
pub(crate) fn check(args: &CheckArgs, work: &Path) -> Result<Checked> {
    if !cfg!(windows) {
        bail!("the package is a Windows package; check it on Windows");
    }
    std::fs::create_dir_all(work)?;
    tar(&["-x", "-f"], &args.zip, work, None)
        .with_context(|| format!("cannot unpack {}", args.zip.display()))?;
    let tops: Vec<PathBuf> = std::fs::read_dir(work)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    let [root] = tops.as_slice() else {
        bail!(
            "{} should hold one folder, and holds {}",
            args.zip.display(),
            tops.len()
        );
    };
    let mut checked = Checked {
        root: root.clone(),
        ..Checked::default()
    };
    let manifest = native::manifest()?;
    let files = native_files(&manifest)?;
    let licence = licence_file(&manifest)?;

    // The file list, both ways.
    let mut expected: Vec<String> = vec!["strobe.exe".to_owned(), "scenes".to_owned(), licence];
    expected.extend(files.iter().map(|(name, _)| name.clone()));
    expected.extend(DOCS.iter().map(|d| (*d).to_owned()));
    let present: Vec<String> = std::fs::read_dir(root)?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let missing: Vec<&String> = expected.iter().filter(|e| !present.contains(e)).collect();
    let extra: Vec<&String> = present.iter().filter(|p| !expected.contains(p)).collect();
    note(
        &mut checked,
        missing.is_empty() && extra.is_empty(),
        format!("the file list ({} entries)", expected.len()),
        format!("missing {missing:?}, unexpected {extra:?}"),
    );
    let library = crate::workspace_root()?.join("scenes");
    let scenes: Vec<String> = std::fs::read_dir(&library)?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let absent: Vec<&String> = scenes
        .iter()
        .filter(|scene| !root.join("scenes").join(scene).join("main.glsl").is_file())
        .collect();
    note(
        &mut checked,
        absent.is_empty(),
        format!("the scene library ({} scenes)", scenes.len()),
        format!("scenes missing from the package: {absent:?}"),
    );

    // The native files against the pin.
    for (file, pinned) in &files {
        let actual = native::sha256(&root.join(file)).unwrap_or_default();
        note(
            &mut checked,
            actual == *pinned,
            format!("{file} is the pinned build"),
            format!("{file} is {actual}, pinned {pinned}"),
        );
    }

    // The notices are the generated file.
    let notices = std::fs::read_to_string(root.join("THIRD-PARTY-NOTICES.txt")).unwrap_or_default();
    note(
        &mut checked,
        notices.len() > 10_000 && notices.contains("glow"),
        "the third-party notices are the full generated file".to_owned(),
        format!(
            "THIRD-PARTY-NOTICES.txt is {} bytes and does not look generated",
            notices.len()
        ),
    );

    // The version resource names the release the zip says it is: the
    // version in its own name (`strobe-v0.2.1-…`), or the workspace's for a
    // name without one. Not the checkout's alone: the harness may check a
    // release from any checkout (D542).
    let exe = root.join("strobe.exe");
    let version = file_version(&exe).unwrap_or_else(|error| format!("unreadable ({error:#})"));
    let named = args
        .zip
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(version_in_name);
    let expected = named.as_deref().unwrap_or(env!("CARGO_PKG_VERSION"));
    note(
        &mut checked,
        version == expected,
        format!("strobe.exe's version resource says {version}"),
        format!("strobe.exe's version resource says `{version}`, the package is {expected}"),
    );

    // Every DLL the program imports directly is part of Windows or in the
    // package. One that is neither (the MSVC runtime, say) would make a
    // clean machine refuse to start it, which no test here would notice.
    match imports(&exe) {
        Ok(names) => {
            let missing: Vec<&String> = names
                .iter()
                .filter(|name| !is_windows_dll(name))
                .filter(|name| {
                    !present
                        .iter()
                        .any(|file| file.eq_ignore_ascii_case(name.as_str()))
                })
                .collect();
            note(
                &mut checked,
                missing.is_empty(),
                format!(
                    "strobe.exe's {} direct imports are Windows' own or in the package",
                    names.len()
                ),
                format!("strobe.exe imports DLLs neither Windows nor the package has: {missing:?}"),
            );
        }
        Err(error) => checked
            .problems
            .push(format!("strobe.exe's import table unreadable: {error:#}")),
    }

    // The symbols belong to this program.
    if let Some(symbols) = &args.symbols {
        let unpacked = PathBuf::from(format!("{}-symbols", work.display()));
        // A leftover from an earlier check is not this one's; absent is fine.
        let _ = std::fs::remove_dir_all(&unpacked);
        std::fs::create_dir_all(&unpacked)?;
        tar(&["-x", "-f"], symbols, &unpacked, None)
            .with_context(|| format!("cannot unpack {}", symbols.display()))?;
        let pdb = find(&unpacked, "strobe.pdb");
        let matched = match (pdb, codeview(&exe)) {
            (Some(pdb), Ok((guid, age))) => match pdb_guid(&pdb) {
                Ok(pdb_guid) if pdb_guid == guid => Ok(format!(
                    "strobe.pdb matches strobe.exe (GUID {}, age {age})",
                    hex(&guid)
                )),
                Ok(pdb_guid) => Err(format!(
                    "strobe.pdb is GUID {}, strobe.exe names {}",
                    hex(&pdb_guid),
                    hex(&guid)
                )),
                Err(error) => Err(format!("strobe.pdb unreadable: {error:#}")),
            },
            (None, _) => Err("no strobe.pdb in the symbols zip".to_owned()),
            (_, Err(error)) => Err(format!("strobe.exe has no CodeView record: {error:#}")),
        };
        match matched {
            Ok(line) => checked.passed.push(line),
            Err(line) => checked.problems.push(line),
        }
    }

    // It runs, from where a user would unpack it.
    let diag = PathBuf::from(format!("{}-diag", work.display()));
    match run_diagnose(&exe, &diag) {
        Ok(report) => {
            let pinned = report
                .lines()
                .any(|line| line.contains("angle") && line.contains("match the pin"));
            note(
                &mut checked,
                pinned,
                "`strobe --diagnose` ran and found the pinned ANGLE beside it".to_owned(),
                format!("`strobe --diagnose` ran but did not verify ANGLE:\n{report}"),
            );
            checked.diagnose = Some(report);
        }
        Err(error) => checked
            .problems
            .push(format!("`strobe --diagnose` failed: {error:#}")),
    }
    Ok(checked)
}

/// The `MAJOR.MINOR.PATCH` in a package's name, `strobe-v0.2.1-<triple>.zip`
/// (a `-dev` or other suffix after it is not part of the version resource).
fn version_in_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("strobe-v")?;
    let version: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let parts = version.split('.').filter(|p| !p.is_empty()).count();
    (parts == 3).then_some(version)
}

/// Records a check as passed or as a problem.
fn note(checked: &mut Checked, ok: bool, passed: String, problem: String) {
    if ok {
        checked.passed.push(passed);
    } else {
        checked.problems.push(problem);
    }
}

/// The first file called `name` under `dir`.
fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).ok()?.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if entry.file_name() == name {
                return Some(path);
            }
        }
    }
    None
}

/// `strobe.exe --diagnose` with its files kept in `diag`, its report
/// returned. A minute is far more than it needs.
pub(crate) fn run_diagnose(exe: &Path, diag: &Path) -> Result<String> {
    std::fs::create_dir_all(diag)?;
    let report = diag.join("diagnose.out");
    let child = Command::new(exe)
        .arg("--diagnose")
        .current_dir(exe.parent().context("the program has no folder")?)
        .env("STROBE_DIAG_DIR", diag)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&report)?)
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("cannot start {}", exe.display()))?;
    let status = wait(child, Duration::from_secs(60))?;
    let text = std::fs::read_to_string(&report).unwrap_or_default();
    if !status.success() {
        bail!("exited with {status}:\n{text}");
    }
    Ok(text)
}

/// Waits for a child, killing it at `limit`.
pub(crate) fn wait(
    mut child: std::process::Child,
    limit: Duration,
) -> Result<std::process::ExitStatus> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if started.elapsed() > limit {
            // It is being abandoned; the error says so whether or not the
            // kill landed.
            let _ = child.kill();
            // Reaps it.
            let _ = child.wait();
            bail!("still running after {} s; stopped", limit.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The `FileVersion` of a Windows executable's version resource, read by
/// PowerShell (`.VersionInfo`), which knows the resource format.
fn file_version(exe: &Path) -> Result<String> {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "(Get-Item -LiteralPath '{}').VersionInfo.ProductVersion",
            exe.display()
        ))
        .output()
        .context("failed to run PowerShell")?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || text.is_empty() {
        bail!("no version resource");
    }
    Ok(text)
}

/// Lower-case hex of bytes.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A little-endian `u32` at `at`, or an error naming what was cut short.
fn u32_at(bytes: &[u8], at: usize, what: &str) -> Result<u32> {
    bytes
        .get(at..at + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| anyhow!("{what}: cut short at byte {at}"))
}

/// A little-endian `u16` at `at`.
fn u16_at(bytes: &[u8], at: usize, what: &str) -> Result<u16> {
    bytes
        .get(at..at + 2)
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| anyhow!("{what}: cut short at byte {at}"))
}

/// A `u32` read from a file as a `usize` offset.
fn offset(bytes: &[u8], at: usize, what: &str) -> Result<usize> {
    usize::try_from(u32_at(bytes, at, what)?).context("offset does not fit")
}

/// A PE image's bytes and where its data directories and sections are.
struct Pe {
    bytes: Vec<u8>,
    /// File offset of the first data directory.
    directories: usize,
    /// File offset of the section table, and how many sections it holds.
    sections: (usize, usize),
}

impl Pe {
    /// Reads and checks the headers of a PE file.
    fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let header = offset(&bytes, 0x3c, "the DOS header")?;
        if bytes.get(header..header + 4) != Some(b"PE\0\0") {
            bail!("not a PE file");
        }
        let coff = header + 4;
        let count = usize::from(u16_at(&bytes, coff + 2, "the COFF header")?);
        let optional_size = usize::from(u16_at(&bytes, coff + 16, "the COFF header")?);
        let optional = coff + 20;
        // The data directories start after the fields that differ between
        // PE32 (0x10b) and PE32+ (0x20b).
        let directories = optional
            + match u16_at(&bytes, optional, "the optional header")? {
                0x20b => 112,
                0x10b => 96,
                other => bail!("unknown optional header magic {other:#x}"),
            };
        Ok(Self {
            bytes,
            directories,
            sections: (optional + optional_size, count),
        })
    }

    /// Data directory `index`'s RVA and size.
    fn directory(&self, index: usize) -> Result<(usize, usize)> {
        let at = self.directories + index * 8;
        Ok((
            offset(&self.bytes, at, "a data directory")?,
            offset(&self.bytes, at + 4, "a data directory")?,
        ))
    }

    /// The file offset an RVA maps to, through the section table.
    fn file_offset(&self, rva: usize) -> Result<Option<usize>> {
        let (table, count) = self.sections;
        for index in 0..count {
            let section = table + index * 40;
            let size = offset(&self.bytes, section + 8, "a section")?;
            let start = offset(&self.bytes, section + 12, "a section")?;
            let raw = offset(&self.bytes, section + 20, "a section")?;
            if (start..start + size).contains(&rva) {
                return Ok(Some(raw + (rva - start)));
            }
        }
        Ok(None)
    }

    /// The NUL-terminated ASCII string at an RVA.
    fn string_at(&self, rva: usize) -> Result<String> {
        let at = self
            .file_offset(rva)?
            .context("a name outside every section")?;
        let tail = self.bytes.get(at..).context("a name past the end")?;
        let end = tail
            .iter()
            .position(|b| *b == 0)
            .context("an unterminated name")?;
        Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
    }
}

/// The DLLs a PE file imports directly, from its import directory (the
/// second data directory): one `IMAGE_IMPORT_DESCRIPTOR` of 20 bytes per
/// DLL, its name's RVA at byte 12, ended by an all-zero descriptor.
pub(crate) fn imports(exe: &Path) -> Result<Vec<String>> {
    let image = Pe::read(exe)?;
    let (rva, _) = image.directory(1)?;
    if rva == 0 {
        return Ok(Vec::new());
    }
    let mut at = image
        .file_offset(rva)?
        .context("the import directory is in no section")?;
    let mut names = Vec::new();
    loop {
        let descriptor = image
            .bytes
            .get(at..at + 20)
            .context("an import descriptor past the end")?;
        if descriptor.iter().all(|b| *b == 0) {
            break;
        }
        names.push(image.string_at(offset(&image.bytes, at + 12, "an import name")?)?);
        at += 20;
    }
    Ok(names)
}

/// Whether a DLL a program imports is part of Windows itself, on every
/// supported version, so a clean machine has it: the API sets, and the
/// system libraries the Rust standard library, winit, glutin, cpal and
/// the crash handler link. Anything else must ship in the package or be a
/// documented prerequisite.
fn is_windows_dll(name: &str) -> bool {
    const SYSTEM: [&str; 32] = [
        "combase.dll",
        "mmdevapi.dll",
        "kernel32.dll",
        "user32.dll",
        "gdi32.dll",
        "advapi32.dll",
        "shell32.dll",
        "ole32.dll",
        "oleaut32.dll",
        "comdlg32.dll",
        "comctl32.dll",
        "ws2_32.dll",
        "ntdll.dll",
        "bcrypt.dll",
        "bcryptprimitives.dll",
        "crypt32.dll",
        "secur32.dll",
        "userenv.dll",
        "dwmapi.dll",
        "uxtheme.dll",
        "imm32.dll",
        "winmm.dll",
        "opengl32.dll",
        "dbghelp.dll",
        "shlwapi.dll",
        "setupapi.dll",
        "version.dll",
        "propsys.dll",
        "mfplat.dll",
        "mf.dll",
        "mfreadwrite.dll",
        "iphlpapi.dll",
    ];
    let lower = name.to_ascii_lowercase();
    lower.starts_with("api-ms-win-")
        || lower.starts_with("ext-ms-")
        || SYSTEM.contains(&lower.as_str())
}

/// The GUID and age in a PE file's CodeView (`RSDS`) debug record: what a
/// debugger asks a symbol store for. Reads the optional header's debug data
/// directory, maps its RVA through the section table, and finds the
/// CodeView entry.
pub(crate) fn codeview(exe: &Path) -> Result<([u8; 16], u32)> {
    let image = Pe::read(exe)?;
    let pe = &image.bytes;
    // The debug directory is the seventh data directory.
    let (debug_rva, debug_size) = image.directory(6)?;
    let debug_at = image
        .file_offset(debug_rva)?
        .context("the debug directory is in no section")?;
    for entry in (0..debug_size / 28).map(|i| debug_at + i * 28) {
        // IMAGE_DEBUG_TYPE_CODEVIEW.
        if u32_at(pe, entry + 12, "a debug entry")? != 2 {
            continue;
        }
        let record = offset(pe, entry + 24, "a debug entry")?;
        if pe.get(record..record + 4) != Some(b"RSDS") {
            bail!("the CodeView record is not RSDS");
        }
        let guid: [u8; 16] = pe
            .get(record + 4..record + 20)
            .and_then(|b| b.try_into().ok())
            .context("the CodeView GUID is cut short")?;
        return Ok((guid, u32_at(pe, record + 20, "the CodeView age")?));
    }
    bail!("no CodeView debug entry")
}

/// The GUID in a PDB's information stream (stream 1), read through the MSF
/// 7.0 container: the superblock, the block map, the stream directory.
pub(crate) fn pdb_guid(pdb: &Path) -> Result<[u8; 16]> {
    const MAGIC: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";
    let file = std::fs::read(pdb)?;
    if file.get(..MAGIC.len()) != Some(MAGIC) {
        bail!("not an MSF 7.0 PDB");
    }
    let block = offset(&file, 32, "the superblock")?;
    let directory_bytes = offset(&file, 44, "the superblock")?;
    let block_map = offset(&file, 52, "the superblock")?;
    if block == 0 {
        bail!("a block size of zero");
    }
    let block_at = |index: usize| index * block;
    // The directory's own blocks are listed at the block map.
    let directory_blocks = directory_bytes.div_ceil(block);
    let mut directory = Vec::with_capacity(directory_bytes);
    for i in 0..directory_blocks {
        let at = block_at(offset(&file, block_at(block_map) + i * 4, "the block map")?);
        directory.extend_from_slice(file.get(at..at + block).context("a directory block")?);
    }
    let streams = offset(&directory, 0, "the stream count")?;
    if streams < 2 {
        bail!("no information stream");
    }
    let sizes: Vec<usize> = (0..streams)
        .map(|i| offset(&directory, 4 + i * 4, "a stream size"))
        .collect::<Result<_>>()?;
    // Stream 1's block list follows stream 0's.
    let blocks_of = |size: usize| {
        if size == u32::MAX as usize {
            0
        } else {
            size.div_ceil(block)
        }
    };
    let first_of_stream_one = 4 + streams * 4 + blocks_of(sizes[0]) * 4;
    let first_block = offset(&directory, first_of_stream_one, "stream 1's first block")?;
    let info = block_at(first_block);
    // version (4), signature (4), age (4), then the GUID.
    file.get(info + 12..info + 28)
        .and_then(|b| b.try_into().ok())
        .context("the information stream is cut short")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_program_and_its_pdb_share_a_guid() -> Result<()> {
        // The xtask binary running these tests is a PE with a PDB beside it
        // on Windows; elsewhere there is nothing to read.
        if !cfg!(windows) {
            return Ok(());
        }
        let exe = std::env::current_exe()?;
        let pdb = exe.with_extension("pdb");
        if !pdb.is_file() {
            // Test binaries name their PDB after the crate, with a hash; the
            // check is the CodeView record alone then.
            let (guid, _) = codeview(&exe)?;
            assert!(guid.iter().any(|b| *b != 0), "a GUID was read");
            return Ok(());
        }
        let (guid, _) = codeview(&exe)?;
        assert_eq!(
            pdb_guid(&pdb)?,
            guid,
            "the PDB beside the program is its own"
        );
        Ok(())
    }

    #[test]
    fn a_file_that_is_not_a_pe_or_a_pdb_is_refused() -> Result<()> {
        let dir = scratch("not-pe");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join("x.bin");
        std::fs::write(&file, b"MZ but nothing else")?;
        let not_pe = codeview(&file).is_err();
        let not_pdb = pdb_guid(&file).is_err();
        // Scratch space; a leftover is harmless.
        let _ = std::fs::remove_dir_all(&dir);
        assert!(not_pe, "a short file is not a PE");
        assert!(not_pdb, "nor a PDB");
        Ok(())
    }

    #[test]
    fn the_expected_version_is_the_packages_own() -> Result<()> {
        assert_eq!(
            version_in_name("strobe-v0.2.1-x86_64-pc-windows-msvc.zip").as_deref(),
            Some("0.2.1"),
            "a release's name"
        );
        assert_eq!(
            version_in_name("strobe-v0.2.0-dev-x86_64-pc-windows-msvc.zip").as_deref(),
            Some("0.2.0"),
            "a dev package's suffix is not in the resource"
        );
        assert_eq!(version_in_name("other.zip"), None, "no version in the name");
        Ok(())
    }

    #[test]
    fn the_manifest_names_the_licence_file() -> Result<()> {
        let manifest = native::manifest()?;
        assert_eq!(
            licence_file(&manifest)?,
            "LICENSES.chromium.html",
            "the licence travels"
        );
        assert_eq!(native_files(&manifest)?.len(), 4, "four DLLs");
        Ok(())
    }
}
