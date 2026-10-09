# Security Policy

## Reporting a vulnerability

Please do **not** open a public issue for a security vulnerability. Instead,
report it privately:

- **Preferred:** use GitHub's private vulnerability reporting on the
  [Security tab](https://github.com/lnorton89/strobe/security/advisories/new).
- **By email:** [lnorton89@gmail.com](mailto:lnorton89@gmail.com). If you
  encrypt, ask for a key first; otherwise plain text is fine.

Include what you found, how to reproduce it (a small scene folder or script
is ideal — Strobe loads scenes and runs the code inside them), and which
version or commit you tested against. Reports are acknowledged within a few
days, and fixes land in the next release.

## Supported versions

Only the latest release is supported. Fixes ship in a new release; there is
no backport window for older versions.

## Scope

Strobe is a live-visuals engine: it opens windows, captures audio, compiles
shaders for your GPU driver, and plays scenes you chose. Doing those things
is the product, not a vulnerability. A shader that makes your GPU slow, or
that hangs a driver, is a bug worth reporting publicly; it is not a
security issue unless it escapes the sandbox described below.

What *is* in scope:

- **The localhost import listener.** Strobe can listen on the loopback
  interface to receive shaders sent to it (for example from a browser
  helper). Anything that lets another machine reach it, lets a web page in
  a browser on this machine drive it without the user's say-so (cross-origin
  or DNS-rebinding requests), or lets a crafted request write outside the
  scene library, is a real finding. So is a request that crashes the
  process, which on a projector is a denial of service.
- **Scene scripts and the QuickJS sandbox.** A scene may carry JavaScript
  (the Synesthesia scripting API), which Strobe runs in an embedded QuickJS
  engine. A script is meant to see only the audio features, the controls
  and the time the API hands it. A way for a script to reach the file
  system, the network, the process environment, or native code, to read
  another scene's state, or to run without bound and freeze the output, is
  in scope. A scene *you* choose to play being slow is not.
- **Shader import from the web.** Strobe downloads shaders from Shadertoy
  and converts them into scenes. Untrusted JSON, GLSL, and asset URLs
  arrive that way. Path traversal in a downloaded asset name, a download
  that is followed to a non-HTTPS or local address, unbounded download or
  decompression size, and a converted scene that behaves differently from
  its source in a way that hides code are all in scope.
- **Scene and playlist parsing.** `.synScene` folders, `scene.json`,
  playlists and saved state are read from disk and from shared folders. A
  malformed file that corrupts memory or writes elsewhere is a finding; one
  that merely fails to load with a clear error is correct behaviour.
- **Misuse of an `unsafe` block.** All `unsafe` lives in
  `crates/strobe-gfx/src/gl/`, one safe leaf wrapper per call. A missing
  bounds check on a pointer-taking GL call, a use of a context from the
  wrong thread, or an owning type whose `Drop` frees something twice
  belongs here.
- **The release artefacts.** The release zip, its checksums, SBOM, and
  build-provenance attestations. A way to make a published artefact differ
  from what the workflow built is in scope. Strobe is not code-signed (a
  certificate lives outside this repository), so Windows SmartScreen may
  warn on a fresh download; that is known.

Everything else — bug reports, feature requests, and questions — belongs on
the [issue tracker](https://github.com/lnorton89/strobe/issues); see
[`CONTRIBUTING.md`](CONTRIBUTING.md).
