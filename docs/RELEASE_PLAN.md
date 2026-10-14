# Release plan: v0.1.0, the first tag

**Scope.** A first, *internal* tag of Strobe for the owner and a small set of
testers on the T480 and TORNADO. It is not a "release-ready" claim: the
validation that stays open (below) is recorded as known limitations, not
closed. The full product-facing bar is gate **G6** in
[IMPLEMENTATION_BACKLOG.md](IMPLEMENTATION_BACKLOG.md); this plan exists to put
a real archive in a tester's hands and to prove the release machinery on a tag
for the first time.

This plan does not replace the release order in
[CONTRIBUTING.md](../CONTRIBUTING.md) ("Releasing"). It sequences that order
around the checks the first tag has never exercised.

_Last updated: 2026-10-14. **v0.1.0 is cut and published.** Steps 0-3 and the
publish are done; steps 4 and 5 await a clean machine and the T480._

## Where the repository stands

| Fact | Value |
|---|---|
| Git tags | `v0.1.0`, on the release PR's merge commit `578ea34` (2026-10-14). |
| Release | Published as **latest** at [releases/tag/v0.1.0](https://github.com/lnorton89/strobe/releases/tag/v0.1.0), with a "Known limitations" section in its notes. |
| `[workspace.package] version` | `0.1.0` — the tagged version. No bump was needed (see step 2). |
| `CHANGELOG.md` | Generated: 318 entries under `[0.1.0]`, `Unreleased` empty. |
| `release.yml` | Ran end to end for the first time: draft, Windows build and publish all green (run 38495718734). Q12 is resolved. |
| Decision record | [DECISIONS.md](DECISIONS.md) D270. |
| Code signing | None. Attested build provenance is the supply-chain proof, and SmartScreen will warn on a fresh download ([USER_GUIDE.md](USER_GUIDE.md)). |

## Definition of done

A published GitHub release tagged `v0.1.0`, from a green `master`, whose
`strobe-v0.1.0-x86_64-pc-windows-msvc.zip`:

- [x] unpacks with the exact contents `release.yml` documents (exe, three ANGLE
  DLLs, `LICENSES.chromium.html`, `scenes/`, `README.md`, `LICENSE`);
- [ ] runs from outside the development tree on a clean machine, in native GL
  and in ANGLE, with a clean shutdown;
- [x] has a `.sha256` that verifies after download and an attestation that
  validates with `gh attestation verify`;
- [ ] has its own `-symbols.zip` that symbolicates a crash from that exact
  build;
- [x] ships notes that list the known limitations a tester will hit.

## The checklist, in order

### 0. Pre-flight — a green baseline ✅

- [x] `master` is clean and CI-green: `cargo xtask ci` passed on the commit to
  be tagged (136s, all 8 steps; the Windows cross-check steps skip on a Windows
  host, and 108 tests passed by skipping for want of a headless GL driver,
  which only CI's Linux job provides).
- [x] `git status` is clean (no stray agent worktrees or half-committed docs).
- [x] The build is `--locked`: `Cargo.lock` is current — the release build
  itself ran `cargo build --release --locked` and succeeded.
- [x] `cargo deny check advisories` re-run close to the tag: `advisories ok`.
  (CI skips this step when `cargo-deny` is not installed, so it was installed
  and run by hand.)
- [ ] Optional hygiene: delete remote branches that are already merged
  (`claude/festive-ramanujan-oic192`, the dependabot ones, `feat/*`,
  `research/*`), per CONTRIBUTING's "delete a branch once it is merged".

### 1. Dry-run the release workflow first (Q12) ✅

The workflow has never run on a tag, and its own failure mode is the dangerous
one: a second push of a `v*` tag re-runs it and ends in
`gh release upload --clobber`, rebuilding an already-published release under new
checksums. Prove the machinery on a throwaway tag before `v0.1.0` exists.

- [x] Run it once end to end somewhere disposable, then delete the release and
  the tag. **Not a fork:** GitHub refuses to fork your own repository, so this
  was a throwaway public mirror, `lnorton89/strobe-release-dryrun`, deleted
  after (its release and tag are gone; the empty repository needed the
  `delete_repo` scope to remove and still exists). The tag must match the
  version: `release.yml`'s first check fails unless `v${version}` equals the
  tag, so a bare `v0.0.0-test` tag fails at once against
  `version = "0.1.0"`. The mirror was tagged `v0.1.0` directly.
  **Also found:** a fresh repository did not register `release.yml` from a push
  alone — `ci.yml` appeared and it did not, until the workflow file itself was
  changed. Worth knowing before pointing the workflow at a new repository.
- [x] Confirm each job: the tag-vs-`Cargo.toml` check; the ANGLE fetch's
  SHA-256 verification (`scripts/fetch-angle.ps1 -Profile release`); the package
  file list; the CycloneDX SBOM (`cargo-cyclonedx 0.5.9`); the checksums; both
  attestations; the draft to publish transition. All three jobs green (run
  38494313118).

### 2. Cut the tag ✅

Per CONTRIBUTING, with one deviation this first time:

- [x] On a release branch off `master`, **no version bump was needed**:
  `[workspace.package] version` was already `0.1.0` and no tag carried it. (The
  generator refuses a version that is not newer than the latest release; with no
  releases, `0.1.0` is accepted.)
- [x] `cargo xtask changelog --release v0.1.0`, then commit its result. With no
  prior tag the section was the whole history to date: 318 entries.
- [x] Open the PR (#16) and let the checks pass — the changelog check
  recognises a leading section whose tag does not exist yet and validates it as
  the release being cut. All six jobs passed.
- [x] Merge with a **merge commit** (never a squash: the changelog pins
  individual commit hashes) — `578ea34` — then tag the merge commit:
  `git tag -a v0.1.0 -m "strobe v0.1.0"` and push.

One ordering rule earned the hard way: every non-changelog commit must land
**before** the section is generated. The check recomputes a section from the
tagged range, so a commit added afterwards shows as drift. The README's status
line was committed first, then the generated section on its own.

### 3. Inspect the published release (R-05) ✅

- [x] Download the archive **from the release page**, not from the build job's
  artifacts, and verify the `.sha256` — all three verified.
- [x] Check the zip's contents and layout against the list in CONTRIBUTING:
  exactly `strobe.exe`, `libEGL.dll`, `libGLESv2.dll`, `d3dcompiler_47.dll`,
  `LICENSES.chromium.html`, `README.md`, `LICENSE` and twelve `*.synScene`
  folders.
- [x] Confirm the ANGLE DLL provenance: they come from the pinned Electron
  38.2.0 archive, are covered by `LICENSES.chromium.html` beside them, and were
  hash-checked during the build, which fails the release on a mismatch. Do not
  bump the pin as part of this release.
- [x] Confirm the `-symbols.zip` holds the matching `strobe.pdb` for *this*
  build, and keep it: it is what symbolicates a crash from this exact release.
- [x] Verify the attestations (`gh attestation verify`) on the downloaded
  bytes, not on the workflow's local output — 0 for build provenance, and 0 for
  the CycloneDX predicate against the program zip (the SBOM's *subject* is the
  zip, not the SBOM file).

### 4. Clean-machine smoke test (R-03) — **not done**

Needs a machine without the toolchain, ideally the T480.

- [ ] Unzip outside the development tree (for example `C:\Strobe`), with no
  Cargo, toolchain or developer DLLs available.
- [ ] First run: the data directories under `%LOCALAPPDATA%\strobe` are created;
  the library loads; audio works or reports truthfully; the last-played scene
  reopens.
- [ ] Native GL, then `--gl angle`: the same scenes load and play.
- [ ] Scripts run; the Shadertoy import listener binds loopback; diagnostics
  and `--diagnose` write sensible output; shutdown is clean.
- [ ] Relaunch over an existing install (upgrade-in-place) works.

*Accept:* commands, logs and screenshots attached to
[hardware/T480.md](hardware/T480.md).

### 5. Crash symbolication from the released archive (R-04) — **not done**

- [ ] `STROBE_FAULT=crash@120` on the downloaded `strobe.exe` writes a minidump
  and report.
- [ ] `cargo xtask symbolicate` with the `-symbols.zip`'s PDB names functions and
  `file:line`.
- [ ] Audit the report text: no secret, token, API key or path leaks
  ([DEBUGGING.md](DEBUGGING.md)).

*Accept:* a symbolic thread from the *downloaded* archive, recorded in
[hardware/T480.md](hardware/T480.md).

### 6. Docs and release notes for testers — partially done

- [x] Update `README.md`'s status line (it said "in development, no release
  yet"; it now says v0.1.0 is the first release, for testing, and points at
  what is still unverified).
- [x] Add a **Known limitations in v0.1.0** section to the release notes. It is
  in the published release body, which is the right place: `CHANGELOG.md` is
  generated and its released sections are rewritten, so prose does not survive
  there.
- [ ] Check [USER_GUIDE.md](USER_GUIDE.md)'s install steps against the actual
  archive, and that [BUG_REPORTING.md](BUG_REPORTING.md) and the issue forms ask
  for what a binary-only tester can provide (no debugger, no Cargo).

### 7. Publish and hand off — published, hand-off pending

- [x] Decide the release's visibility: published as **latest**, matching
  `release.yml`'s `--draft=false --latest`. D270 records the reasoning.
- [ ] Send testers the download link, the smoke checklist from step 4, and
  where to file results.

## Not blocking v0.1.0 — recorded as known limitations

Each of these is open in [STATUS_AUDIT.md](STATUS_AUDIT.md) or
[IMPLEMENTATION_BACKLOG.md](IMPLEMENTATION_BACKLOG.md), and each is named in
the published release's "Known limitations" section.

| Item | Backlog | Why it can wait |
|---|---|---|
| Audio device removed and reconnected | R-01 | Live capture and the reopen-backoff fix (D266) are verified; the unplug/replug path is a tester-visible rough edge, not a data-loss one. |
| MIDI on real hardware | P-04 | The parser and mapper are tested from recorded bytes; only a physical controller is missing. |
| Live Shadertoy import (path A and B) | I-01 | Cloudflare blocks the REST path from containers; the userscript path is built and is the tester's job to confirm. |
| Remote from a real phone, the Windows firewall prompt, the T480 soak | L-02, L-08 | Loopback and Node-client tests pass; real devices are outside a cloud agent's reach. |
| A 4-hour show with real scenes; HD 4000 cost of transitions and the post stage | P-03 | Only the pure clock and the UHD 620 have been exercised. |
| A real OS file drop (Windows and X11) | I-02 | The typed path runs the same job. |
| Title-bar drag/Snap/DPI, a maximised undecorated window, the second monitor | U-09 | Needs a real window manager and a person. |
| Explorer/taskbar look at the shipped icon | R-06 | The icon and version block are embedded and were read back on the T480; only the visual once-over is left. |

## Evidence recorded

- **R-05** closed in [IMPLEMENTATION_BACKLOG.md](IMPLEMENTATION_BACKLOG.md),
  with the archive hash, checksums and attestation output.
- **Q12 resolved** in [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md) (as R14).
- [ROADMAP.md](ROADMAP.md) item 9 ticked; Phase 0's "a tagged release has not
  been run" corrected.
- The P2 release bullet in [STATUS_AUDIT.md](STATUS_AUDIT.md) ticked with the
  same evidence.
- **[DECISIONS.md](DECISIONS.md) D270** for the choices this release made
  (publish now as `latest` with the open work named, no version bump, the
  section written before the tag, the mirror instead of a fork).

## Housekeeping left behind

- `lnorton89/strobe-release-dryrun` still exists as an empty public repository.
  Its release and tag were deleted; removing the repository itself needs the
  `delete_repo` scope (`gh auth refresh -h github.com -s delete_repo`, then
  `gh repo delete lnorton89/strobe-release-dryrun --yes`).
