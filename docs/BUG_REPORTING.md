# Bug reporting and crash reporting research

**Status:** research and implementation proposal; no runtime behavior changes in this document-only change.

## Executive recommendation

Strobe already has a stronger local crash-diagnostics foundation than many early desktop apps. `strobe-diag` writes rotating session logs, structured failures, panic reports with backtraces, and (where the platform permits) native minidumps from a separate monitor process. `--diagnose` produces a machine/build/graphics/audio report. The missing layer is a simple, privacy-respecting path from that evidence to a useful report the maintainer can act on.

**Ship a local-first “Report a problem” workflow before adding always-on telemetry.** The first useful version can have no new service, no API keys, and no recurring infrastructure bill:

1. Let the user create or open a diagnostic bundle from the app.
2. Show what will be included and let the user review/redact it.
3. Open a GitHub issue form with a short, prefilled summary and environment details.
4. Let the user explicitly attach the generated bundle in GitHub's UI. Never upload it in the background.
5. Keep crash capture and logs working when the network is unavailable or the reporting destination is down.

Then consider optional, consent-based event reporting to a Sentry-compatible service if manual reports prove insufficient. Do not make a hosted crash service a prerequisite for Strobe to start or render.

## What Strobe already has

The repository's current design and code already cover much of the hard local-capture work:

- `strobe-diag` owns session logging, a bounded in-memory log ring, structured failures, session context, panic hooks, system reports, and crash-report rendering.
- `crash-handler` plus `minidumper` provide out-of-process native crash capture, so the faulting process does not need to write the minidump itself.
- Reports can include build/version and git metadata, OS/CPU/RAM, GL vendor/renderer/version/backend, scene, render size, frame/pass information, recent failures, log tail, and a Rust backtrace for panics.
- `strobe-app` has a Windows-friendly failure dialog and points the user to report/minidump files.
- `docs/DEBUGGING.md` documents log locations, environment variables, context-loss recovery, and the crash drill.

This means the next PR should not bolt on another panic hook or replace the native crash mechanism. It should make the existing evidence discoverable, reviewable, and easy to submit.

Known capture limits must remain visible in the UI and docs: a crash before the monitor connection is established may not produce a minidump; native crash support is platform-dependent; and a Windows fast-fail/abort can bypass the exception filter. A text report or last session log may still exist.

## Proposed user experience

### Entry points

- **Help / About → Report a problem** for reproducible bugs and feedback.
- **After a crash or fatal startup failure:** keep the existing explanation, then offer actions to open the report, create a diagnostic bundle, or open the issue form.
- **Diagnostics page or dialog:** show the active log path, latest crash report, and whether native crash capture is available. Do not require the user to find hidden AppData directories.

Use plain labels and explain the difference between a report and a minidump. A minidump can contain process memory and should be treated as sensitive; it must never be attached automatically.

### Diagnostic bundle

Generate a deterministic archive (for example, `strobe-report-<version>-<timestamp>.zip`) containing only selected diagnostics:

- A concise `report.txt` with Strobe version, commit/build metadata, OS, CPU/RAM, graphics backend and driver, audio device *type* where known, display/render dimensions, and relevant feature flags.
- The most recent session log, capped at a reasonable size.
- The selected panic/native crash text report, if present.
- A short `README.txt` explaining the contents and how to attach the bundle.
- Optionally include a minidump only after an explicit, separate checkbox with a warning that it may contain sensitive memory.

Do not bundle the entire configuration or scene library by default. Do not include audio samples, captured audio, arbitrary scene source, imported textures, playlist files, environment variables, access tokens, or the full process environment. Include the active scene's name only if it is useful and clearly shown to the user; a scene name can reveal private performance content.

### Review and privacy controls

Before saving or opening a submission page, show a preview of the exact text files and an inventory of attachments. Offer “Open folder” and “Copy summary” even if the user chooses not to report.

Redact or normalize likely sensitive values before the preview, including home-directory prefixes, usernames in paths, URL query strings, bearer/token-like values, and known secret-bearing configuration fields. Redaction must be conservative and visible: never claim a bundle is sanitized perfectly. Let the user inspect the final artifacts, exclude individual files, and cancel.

The app must say clearly that **nothing is sent until the user takes an explicit submission action**. Opening a GitHub page does not upload the local ZIP; the user attaches it in their browser after reviewing it.

## Lowest-cost delivery options

| Option | Recurring cost | Strengths | Limitations | Recommendation |
|---|---:|---|---|---|
| Local bundle + GitHub issue form | $0 additional service cost | Transparent, auditable, user-controlled, no backend or secrets | User must attach the bundle; GitHub account/browser required for a public issue | **Phase 1 default** |
| Optional hosted GlitchTip using the Sentry-compatible Rust SDK | A free hosted tier currently advertises up to 1,000 events/month; paid tiers are available | Aggregated errors, release grouping, alerting, low operations burden | Requires consent and a data policy; hosted quotas/pricing can change; SDK compatibility does not prove every native minidump feature works | Possible Phase 2, after a small compatibility spike |
| Self-host GlitchTip | Software is open source; infrastructure and maintenance are not literally free | Data stays on infrastructure you control; Sentry-compatible SDKs | Requires HTTPS, updates, backups, storage monitoring, and a reachable intake endpoint. Official install guidance recommends 512 MB RAM and says storage can reach ~30 GB per million events/month | Good if an existing VPS is available and operational burden is acceptable |
| Full self-hosted Sentry | Usually much more operationally expensive | Broad features and mature ecosystem | Higher infrastructure and maintenance complexity than Strobe needs initially | Do not start here |

Sources and current references are listed at the end. Prices, free quotas, and service terms should be rechecked immediately before launch.

### Important distinction: handled errors versus native minidumps

The official Sentry Rust SDK supports error and panic events, and the separate `sentry-minidump` crate describes native minidump capture/upload on Linux, macOS, and Windows. GlitchTip documents the Sentry-compatible Rust SDK, but that alone is **not** proof that its hosted or self-hosted endpoint fully supports Strobe's native minidump attachment path.

Before choosing GlitchTip for native crash uploads, build a throwaway compatibility test for each supported OS: send a synthetic panic, send a handled error, upload a synthetic minidump, verify event grouping/release metadata, and confirm the user's consent and redaction settings. Until that test passes, treat hosted error events and local native minidumps as separate capabilities. Do not replace the working local minidumper with a remote SDK.

## Suggested architecture

Keep all reporting policy and bundle creation in `strobe-diag`; keep the UI in `strobe-app`.

- Add a pure/testable bundle manifest and selection model to `strobe-diag`.
- Add bounded archive creation with explicit file allowlists, size limits, stable filenames, and no network code.
- Keep file collection separate from redaction and from opening external URLs so each part can be tested independently.
- Expose the generated bundle path and warnings to the app; the app presents the review screen and opens the configured issue URL.
- Use the repository's GitHub issue form and template chooser for structured reports. Prefill only short fields in the URL; URLs have practical length limits and can leak into browser history. Do not place full logs, dumps, or sensitive details in URL query parameters.
- If optional remote telemetry is added later, put it behind a separate Cargo feature and an explicit opt-in setting, make it disabled by default, cap events, set release/environment metadata, scrub payloads before send, and ensure network failures never block startup, rendering, or shutdown.
- Do not put a write-capable GitHub token or any secret in the desktop binary. A public issue form opened in the browser is safer than an app-side GitHub API token.

### Reliability and performance requirements

- No disk archive work, HTTP request, compression, symbolication, or waiting on a network operation on the render/audio callback threads.
- Crash-path handlers must remain minimal and avoid allocation, blocking I/O, ordinary mutexes, and network operations in compromised context. Create bundles later from the healthy app or on next launch.
- Bound log and archive sizes. Report missing/unreadable files instead of failing the whole bundle.
- All report features are best effort and must not prevent Strobe from opening, playing a scene, or saving local evidence.
- If a previous-run crash report exists on startup, offer to review it, but do not automatically open a browser or submit anything.

## Implementation plan

### P0 — local-first report flow

1. Add a “Report a problem” entry point to the app and a clear post-crash action.
2. Add a deterministic bundle builder in `strobe-diag`, with allowlisted files, size limits, a manifest, and tests for missing files and oversized logs.
3. Build a review UI that previews the text, lists every included file, lets users exclude the minidump, and saves the archive to a user-visible location.
4. Add a GitHub bug-report issue form that asks for expected/actual behavior, reproduction steps, Strobe version, OS, graphics backend/renderer, and whether a crash report is attached.
5. Open the issue form with a short template and let the user manually attach the bundle. If the browser or network is unavailable, preserve the bundle and show the repository URL as a fallback.
6. Document exactly what is collected and what is not, including the minidump warning and platform capture limitations.

### P1 — make reports actionable

1. Add a report ID shared by the bundle manifest and any app-generated issue text.
2. Add a “Copy diagnostic summary” action that excludes logs and dumps by default.
3. Test redaction against fixtures containing Windows and Unix home paths, URLs with query parameters, token-like strings, and multiline logs.
4. Add integration tests for panic report, native crash report, no-crash startup failure, missing log file, and disabled crash monitor.
5. Add an opt-in setting only if a remote service is selected and the privacy copy, data retention, quota handling, and disable/delete path are settled.

### P2 — optional hosted error aggregation

1. Spike Sentry Rust SDK + GlitchTip for handled errors/panics only.
2. Separately verify native minidump compatibility; do not assume protocol compatibility means feature parity.
3. Decide whether a hosted DSN in a public binary is acceptable and add abuse/quota protections before shipping.
4. Keep remote reporting disabled until the user opts in; expose the endpoint and collected event types in the privacy dialog.
5. Add release grouping, sampling/rate limits, retention, alerting, and an operational owner before enabling it in stable releases.

## Acceptance criteria

- A user who does not know where Strobe stores its logs can create a useful bug report from the app.
- The user sees the exact files and text being shared before any upload or browser handoff.
- No crash dump, log, scene source, audio data, or machine detail is transmitted automatically.
- A generated bundle contains enough version, OS, graphics, scene/pass, failure, and recent-log context to reproduce common issues without collecting unrelated user files.
- Bundle creation handles missing files, permissions errors, and large logs without crashing or blocking playback.
- A reporting service outage, no network, or missing GitHub login has no effect on the app's core operation.
- Tests cover redaction, size limits, archive contents, and no-network behavior.
- Native crash capture remains local and out-of-process until a remote minidump path has been proven end-to-end.

## Research references

- [Strobe diagnostics crate](../crates/strobe-diag/README.md) — current local logging and crash capture.
- [Strobe debugging guide](DEBUGGING.md) — report locations and capture limitations.
- [Official Sentry Rust SDK](https://github.com/getsentry/sentry-rust) — Rust error/panic SDK and integrations.
- [Sentry minidump crate documentation](https://docs.rs/sentry-minidump/latest/sentry_minidump/) — native crash minidumps; supported platforms and process model.
- [GlitchTip Rust SDK documentation](https://glitchtip.com/sdkdocs/rust/) — Sentry-compatible Rust setup.
- [GlitchTip pricing](https://beta.glitchtip.com/pricing/) — current hosted event tiers; recheck before launch.
- [GlitchTip installation guide](https://glitchtip.com/documentation/install/) — self-hosted Docker Compose and resource guidance.
- [GitHub issue template configuration](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/configuring-issue-templates-for-your-repository) — issue forms and external contact links.
- [GitHub issue form syntax](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms) — structured report fields.
