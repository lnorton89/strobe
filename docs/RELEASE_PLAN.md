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

_Last updated: 2026-10-13._

## Where the repository stands

| Fact | Value |
|---|---|
| Git tags | **None.** `git tag` is empty, so `v0.1.0` is the first tag and the changelog generator treats it as a first release. |
| `[workspace.package] version` | `0.1.0` — already the number to tag, so no version bump is needed (see step 2). |
| `CHANGELOG.md` | Holds `Unreleased` only; `cargo xtask changelog --check` passes (verified 2026-10-13). `Unreleased` is stale by design — it is exempt from the check and is rebuilt by `--release`. |
| `.github/workflows/release.yml` | Written, `actionlint`-clean, and every command has been run in pieces locally — but it has **never executed on a tag** ([ROADMAP.md](ROADMAP.md) item 9, [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md) Q12, backlog R-05). |
| `README.md` | Status line still reads "in development, no release yet". |
| Code signing | None. Attested build provenance is the supply-chain proof, and SmartScreen will warn on a fresh download ([USER_GUIDE.md](USER_GUIDE.md)). |

## Definition of done

A published GitHub release tagged `v0.1.0`, from a green `master`, whose
`strobe-v0.1.0-x86_64-pc-windows-msvc.zip`:

- unpacks with the exact contents `release.yml` documents (exe, three ANGLE
  DLLs, `LICENSES.chromium.html`, `scenes/`, `README.md`, `LICENSE`);
- runs from outside the development tree on a clean machine, in native GL and
  in ANGLE, with a clean shutdown;
- has a `.sha256` that verifies after download and an attestation that
  validates with `gh attestation verify`;
- has its own `-symbols.zip` that symbolicates a crash from that exact build;
- ships notes that list the known limitations a tester will hit.

## The checklist, in order

### 0. Pre-flight — a green baseline

- [ ] `master` is clean and CI-green: `cargo xtask ci` passes on the commit to
  be tagged.
- [ ] `git status` is clean (no stray agent worktrees or half-committed docs).
- [ ] The build is `--locked`: `Cargo.lock` is committed and current, since the
  release build uses `--locked`.
- [ ] `cargo deny check advisories` is re-run close to the tag. Advisories move
  independently of the code, so a database entry landing between the last CI
  run and the tag can block the release.
- [ ] Optional hygiene: delete remote branches that are already merged
  (`claude/festive-ramanujan-oic192`, the dependabot ones, `feat/*`,
  `research/*`), per CONTRIBUTING's "delete a branch once it is merged".

*Accept:* a recorded `cargo xtask ci` run on the tagged commit with no skipped
step beyond the Windows cross-check's documented exclusions.

### 1. Dry-run the release workflow first (Q12)

The workflow has never run on a tag, and its own failure mode is the dangerous
one: a second push of a `v*` tag re-runs it and ends in
`gh release upload --clobber`, rebuilding an already-published release under new
checksums. Prove the machinery on a throwaway tag before `v0.1.0` exists.

- [ ] On a **fork** (so nothing public on this repository is touched), run it
  once end to end and then delete the draft and the tag. The tag must match
  the version: `release.yml`'s first check fails unless `v${version}` equals
  the tag, so a bare `v0.0.0-test` tag fails at once against
  `version = "0.1.0"`. Either push `v0.1.0` to the fork directly, or use a
  scratch commit whose `[workspace.package] version` matches a throwaway tag;
  the tag and the version move together.
- [ ] Confirm each job: the tag-vs-`Cargo.toml` check; the ANGLE fetch's
  SHA-256 verification (`scripts/fetch-angle.ps1 -Profile release`); the package
  file list; the CycloneDX SBOM (`cargo-cyclonedx 0.5.9`); the checksums; both
  attestations; the draft to publish transition.

*Accept:* `gh attestation verify <zip> -R lnorton89/strobe` and
`--predicate-type https://cyclonedx.org/bom` both validate against the test
run's artifacts, and the throwaway tag and draft are deleted afterwards.

### 2. Cut the tag

Per CONTRIBUTING, with one deviation this first time:

- [ ] On a release branch off `master`, **no version bump is needed**:
  `[workspace.package] version` is already `0.1.0` and no tag carries it. (The
  generator refuses a version that is not newer than the latest release; with no
  releases, `0.1.0` is accepted.)
- [ ] `cargo xtask changelog --release v0.1.0`, then commit its result. With no
  prior tag the section is the whole history to date.
- [ ] Open the PR, let the checks pass — the changelog check recognises a
  leading section whose tag does not exist yet and validates it as the release
  being cut.
- [ ] Merge with a **merge commit** (never a squash: the changelog pins
  individual commit hashes), then tag the merge commit:
  `git tag -a v0.1.0 -m "strobe v0.1.0"` and push. The tag must be `v0.1.0`:
  `release.yml` fails the run if `v${version}` disagrees with the tag.

*Accept:* the tag points at the merge commit, `Cargo.toml` still says `0.1.0`,
and the pushed tag starts the release workflow.

### 3. Inspect the published release (R-05)

- [ ] Download the archive **from the release page**, not from the build job's
  artifacts, and verify the `.sha256`.
- [ ] Check the zip's contents and layout against the list in CONTRIBUTING.
- [ ] Confirm the ANGLE DLL provenance: they came from the pinned Electron
  38.2.0 archive, are covered by `LICENSES.chromium.html` beside them, and were
  hash-checked during the build. Do not bump the pin as part of this release.
- [ ] Confirm the `-symbols.zip` holds the matching `strobe.pdb` for *this*
  build, and keep it: it is what symbolicates a crash from this exact release.
- [ ] Verify the attestations (`gh attestation verify`) on the downloaded
  bytes, not on the workflow's local output.

*Accept:* a completed checklist recorded in
[hardware/T480.md](hardware/T480.md), with the archive hash, the OS/GPU/driver,
and the exact commands.

### 4. Clean-machine smoke test (R-03)

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
[hardware/T480.md](hardware/T480.md); the packaged build does not depend on the
working directory (fixed in Tasks 21/22, re-confirm here).

### 5. Crash symbolication from the released archive (R-04)

- [ ] `STROBE_FAULT=crash@120` on the downloaded `strobe.exe` writes a minidump
  and report.
- [ ] `cargo xtask symbolicate` with the `-symbols.zip`'s PDB names functions and
  `file:line`.
- [ ] Audit the report text: no secret, token, API key or path leaks
  ([DEBUGGING.md](DEBUGGING.md), QA bullet at the end of this plan).

*Accept:* a symbolic thread from the *downloaded* archive, recorded in
[hardware/T480.md](hardware/T480.md).

### 6. Docs and release notes for testers

- [ ] Update `README.md`'s status line (it currently says "no release yet").
- [ ] Add a **Known limitations in v0.1.0** section to the release notes, drawn
  from the deferred list below, so testers are not surprised. The release page's
  notes are this release's `CHANGELOG.md` section, so put it in the changelog
  section before tagging, or add it to the draft before publishing.
- [ ] Check [USER_GUIDE.md](USER_GUIDE.md)'s install steps against the actual
  archive, and that [BUG_REPORTING.md](BUG_REPORTING.md) and the issue forms ask
  for what a binary-only tester can provide (no debugger, no Cargo).

*Accept:* the release page a tester lands on names what is unverified and what
to send back on a failure.

### 7. Publish and hand off

- [ ] Decide the release's visibility (see the decision point below).
- [ ] Publish, then send testers the download link, the smoke checklist from
  step 4, and where to file results.

## Decision point: pre-release or latest?

`release.yml` ends with `gh release edit "$TAG" --draft=false --latest`, so a
plain tag publishes as **latest**, publicly, on this public repository. That is
correct for a real v0.1.0. If v0.1.0 is meant to be seen as a tester preview
rather than a recommended download, the workflow needs a small change
(`--prerelease`, and `--latest=false` for a later tag) — decide this *before*
the tag, because the tag is never moved and a re-push rebuilds the assets.

## Not blocking v0.1.0 — record as known limitations

Each of these is open in [STATUS_AUDIT.md](STATUS_AUDIT.md) or
[IMPLEMENTATION_BACKLOG.md](IMPLEMENTATION_BACKLOG.md). For an internal tag they
are documented, not closed:

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

## Evidence to record when this is done

- **R-05** closed in [IMPLEMENTATION_BACKLOG.md](IMPLEMENTATION_BACKLOG.md),
  with the archive hash, checksums and attestation output.
- **Q12 resolved** in [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md) from the dry run.
- `docs/ROADMAP.md` item 9 ("A real tagged release") ticked, and Phase 0's
  "a tagged release has not been run" corrected.
- The P2 release bullets in [STATUS_AUDIT.md](STATUS_AUDIT.md) ticked with the
  same evidence.
- A [DECISIONS.md](DECISIONS.md) entry for anything non-obvious this plan
  decides (the pre-release-vs-latest call, the choice to tag while validation is
  open, and the scope of the tag).
