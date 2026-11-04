// ============================================================================
// Module:       xtask::native
// Description:  `cargo xtask sbom-native`: adds the native files the release
//               zip carries (ANGLE, the D3D compiler, the Vulkan loader) to
//               the CycloneDX SBOM, each checked against
//               scripts/native-deps.json.
//
// Dependencies: anyhow, clap, serde_json, sha2
// ============================================================================

//! The SBOM describes the whole archive, not just the Rust in it (D536).
//!
//! `cargo cyclonedx` lists every crate in `strobe.exe`, but the release zip
//! also carries four DLLs from an Electron release, which no Cargo metadata
//! knows about. This adds one CycloneDX component per DLL: its name, the
//! pinned version, supplier, licence, the SHA-256 of the file actually in
//! the build folder, and where it was downloaded from. A file whose hash is
//! not the pinned one fails the command, so a release cannot describe a DLL
//! it does not ship.
//!
//! `scripts/native-deps.json` is the one list of these files: the fetch
//! script downloads and checks against it, strobe-gfx's build script
//! refreshes a stale copy from it, and `strobe --diagnose` reports against
//! it.

use anyhow::{Context as _, Result, anyhow, bail};
use clap::Args as ClapArgs;
use serde_json::{Value, json};
use sha2::Digest as _;
use std::path::{Path, PathBuf};

/// The manifest, relative to the workspace root.
pub(crate) const MANIFEST: &str = "scripts/native-deps.json";

/// Options for `cargo xtask sbom-native`.
#[derive(ClapArgs, Debug)]
pub(crate) struct Args {
    /// The CycloneDX JSON SBOM to add the components to, in place.
    #[arg(long, value_name = "FILE")]
    sbom: PathBuf,
    /// The folder holding the native files, as built or staged.
    #[arg(long, value_name = "DIR")]
    dir: PathBuf,
}

impl Args {
    /// The same options with relative paths taken from where the command
    /// was typed rather than the workspace root it runs in.
    pub(crate) fn anchored(self, invoked_from: &Path) -> Self {
        Self {
            sbom: invoked_from.join(self.sbom),
            dir: invoked_from.join(self.dir),
        }
    }
}

/// Reads the manifest.
pub(crate) fn manifest() -> Result<Value> {
    let path = crate::workspace_root()?.join(MANIFEST);
    let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {MANIFEST}"))?;
    serde_json::from_str(&text).with_context(|| format!("{MANIFEST} is not JSON"))
}

/// The SHA-256 of a file, as lower-case hex.
pub(crate) fn sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    Ok(sha2::Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// The pinned files: `(name, sha256)` pairs from the manifest's `angle`.
#[cfg(test)]
fn pinned_files(manifest: &Value) -> Result<Vec<(String, String)>> {
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

/// Runs `cargo xtask sbom-native`.
pub(crate) fn run(args: &Args) -> Result<()> {
    let manifest = manifest()?;
    let text = std::fs::read_to_string(&args.sbom)
        .with_context(|| format!("cannot read {}", args.sbom.display()))?;
    let mut sbom: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not JSON", args.sbom.display()))?;
    let added = add_components(&mut sbom, &manifest, &args.dir)?;
    std::fs::write(&args.sbom, serde_json::to_string_pretty(&sbom)?)
        .with_context(|| format!("cannot write {}", args.sbom.display()))?;
    println!(
        "added {added} native component(s) to {}",
        args.sbom.display()
    );
    Ok(())
}

/// Adds one component per pinned file in `dir` to `sbom`, after checking
/// each file's hash against the pin. Returns how many were added.
pub(crate) fn add_components(sbom: &mut Value, manifest: &Value, dir: &Path) -> Result<usize> {
    let angle = &manifest["angle"];
    let version = angle["version"].as_str().context("angle.version")?;
    let source = angle["source"].as_str().context("angle.source")?;
    let archive = angle["archive_sha256"]
        .as_str()
        .context("angle.archive_sha256")?;
    let files = angle["files"].as_array().context("angle.files")?;
    let mut components = Vec::new();
    for file in files {
        let name = file["name"].as_str().context("a file without a name")?;
        let pinned = file["sha256"].as_str().context("a file without a sha256")?;
        let actual = sha256(&dir.join(name))?;
        if actual != pinned {
            bail!(
                "{name} in {} is {actual}, not the pinned {pinned}",
                dir.display()
            );
        }
        let licence = file["licence"].as_str().unwrap_or("NOASSERTION");
        // An SPDX identifier goes in `id`; anything else, such as a
        // LicenseRef for Microsoft's redistributable, is a `name`.
        let licence = if licence.starts_with("LicenseRef-") {
            json!({ "license": { "name": licence } })
        } else {
            json!({ "license": { "id": licence } })
        };
        components.push(json!({
            "type": "library",
            "bom-ref": format!("native:{name}"),
            "name": file["component"].as_str().unwrap_or(name),
            "version": version,
            "supplier": { "name": file["supplier"].as_str().unwrap_or("unknown") },
            "hashes": [{ "alg": "SHA-256", "content": actual }],
            "licenses": [licence],
            "externalReferences": [{ "type": "distribution", "url": source }],
            "properties": [
                { "name": "strobe:file", "value": name },
                { "name": "strobe:archive-sha256", "value": archive },
            ],
        }));
    }
    let list = sbom
        .as_object_mut()
        .ok_or_else(|| anyhow!("the SBOM is not a JSON object"))?
        .entry("components")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow!("the SBOM's components is not a list"))?;
    let added = components.len();
    list.extend(components);
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_for(hash: &str) -> Value {
        json!({ "angle": {
            "version": "1.2.3",
            "source": "https://example.invalid/electron.zip",
            "archive_sha256": "00",
            "files": [{
                "name": "libEGL.dll", "sha256": hash, "component": "ANGLE libEGL",
                "supplier": "Google", "licence": "BSD-3-Clause"
            }, {
                "name": "d3dcompiler_47.dll", "sha256": hash, "component": "D3D compiler",
                "supplier": "Microsoft", "licence": "LicenseRef-Microsoft-Redistributable"
            }]
        }})
    }

    fn folder(name: &str) -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("xtask-native-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("libEGL.dll"), b"same bytes")?;
        std::fs::write(dir.join("d3dcompiler_47.dll"), b"same bytes")?;
        Ok(dir)
    }

    #[test]
    fn each_pinned_file_becomes_a_component_with_its_hash() -> Result<()> {
        let dir = folder("ok")?;
        let hash = sha256(&dir.join("libEGL.dll"))?;
        let mut sbom = json!({ "bomFormat": "CycloneDX", "components": [{ "name": "glow" }] });
        let added = add_components(&mut sbom, &manifest_for(&hash), &dir)?;
        // Scratch space; a leftover is harmless.
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(added, 2, "two files, two components");
        let list = sbom["components"].as_array().cloned().unwrap_or_default();
        assert_eq!(list.len(), 3, "the crates are kept");
        assert_eq!(
            list[1]["hashes"][0]["content"],
            json!(hash),
            "the file's own hash"
        );
        assert_eq!(
            list[1]["licenses"][0]["license"]["id"],
            json!("BSD-3-Clause"),
            "SPDX as id"
        );
        assert_eq!(
            list[2]["licenses"][0]["license"]["name"],
            json!("LicenseRef-Microsoft-Redistributable"),
            "a LicenseRef as name"
        );
        Ok(())
    }

    #[test]
    fn a_file_that_is_not_the_pinned_one_fails() -> Result<()> {
        let dir = folder("stale")?;
        let mut sbom = json!({});
        let result = add_components(&mut sbom, &manifest_for(&"0".repeat(64)), &dir);
        // Scratch space; a leftover is harmless.
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            result.is_err_and(|e| e.to_string().contains("not the pinned")),
            "a release cannot describe a DLL it does not ship"
        );
        Ok(())
    }

    #[test]
    fn the_real_manifest_pins_four_files() -> Result<()> {
        let files = pinned_files(&manifest()?)?;
        assert_eq!(
            files.len(),
            4,
            "libEGL, libGLESv2, d3dcompiler_47, vulkan-1: {files:?}"
        );
        assert!(
            files.iter().all(|(_, hash)| hash.len() == 64),
            "every pin is a full SHA-256"
        );
        Ok(())
    }
}
