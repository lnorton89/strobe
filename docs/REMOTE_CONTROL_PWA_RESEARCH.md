# Research: LAN remote control with an installable PWA

**Status:** LAN web remote v1 built (D250 to D254): `crates/strobe-remote` has the protocol, the app-thread bridge, pairing and paired clients, and an opt-in listener serving the controller page; the app drains it once a frame and shows "Remote on" with a Stop button. Not yet tested on real phones, the Windows firewall prompt or the T480; PWA installation still waits on HTTPS (L-07). The HTTPS route is decided (D258): the operator's own reverse proxy, which `--remote-proxy` lets in. It is untried on phones.  
**Scope:** control a running Strobe show from a phone or tablet on the same local network, without cloud accounts, a hosted service, or opening router ports.

## Executive recommendation

Build a **local remote-control web app served by Strobe itself**, with a small, authenticated HTTP + WebSocket API. Treat the phone UI as a second operator surface, not as a second renderer. Commands must enter the existing app command path and be applied on the winit/event-loop thread; the web server must never touch OpenGL, scene sessions, playlist clocks, audio capture, or egui state directly.

There is one important qualification: **a web page served over plain HTTP from a desktop's LAN IP is not a reliable installable PWA origin**. Service workers and other PWA capabilities require a secure context. `http://localhost` is treated specially only on the device running that browser; `http://192.168.x.x` on a phone is not its localhost. Therefore split the feature into two deliverables:

1. **LAN Remote v1:** a responsive, touch-first controller served from the Strobe host, same-origin with its API. Make it usable in a normal mobile browser over the LAN. Keep the first release useful even if the user never installs it.
2. **PWA installation:** enable service worker, offline app-shell caching, and install prompts only after the project has a supportable HTTPS/certificate story for phones and tablets. Do not claim full PWA support merely because the UI has a manifest or can be pinned to a home screen.

The best UX is one Strobe menu action: **Enable Remote Control** → select a network interface → show a QR code and short pairing code → phone opens the page and pairs → user explicitly approves the device. Remote control is disabled by default and stops when disabled or Strobe exits.

## Why this is worth building

A phone/tablet is a practical second surface for a live VJ set: next/previous cue, hold/resume, restart/extend/skip section, blackout, tap tempo, current/next cue and remaining time, plus a small set of named performance macros. The main monitor remains dedicated to the output; the operator does not have to reach for the keyboard or expose the desktop UI to the audience.

Strobe already has a command table that defines playback actions and their focus rules, and the playlist runtime already owns cue/hold/transition state. Remote control should reuse those concepts instead of inventing a parallel playback implementation. Current docs explicitly say MIDI/OSC are not implemented and the planned `strobe-io` crate does not yet exist ([architecture](ARCHITECTURE.md), [app README](../crates/strobe-app/README.md)).

## Product scope

### V1: the stage-safe essentials

- Connection state, Strobe instance name, current scene/cue, next cue, remaining time, Hold status, output/blackout status, and a clear stale/disconnected indicator.
- Large touch targets for Next, Previous, Hold/Resume, Restart Cue, Extend Cue, Skip Section, Blackout, and Tap Tempo.
- Confirmations for destructive or high-impact actions where useful; **Blackout should be immediate**, always visible, and have strong pressed/active feedback.
- A configurable small set of performance macros only after the existing macro target model can report resolved/unavailable/broken/disabled/conflict states.
- Reconnect after phone sleep or Wi-Fi roam; resync from a complete state snapshot after reconnect instead of assuming every event was received.
- QR code plus short human-readable pairing code; revoke paired clients from the desktop.
- Responsive portrait and landscape layouts, safe-area insets, keyboard accessibility, reduced motion, high contrast, and no critical action dependent on hover.
- No cloud, telemetry, third-party CDN, account, or external JavaScript at runtime.

### Explicitly out of scope for v1

- Editing shader source, arbitrary script execution, filesystem browsing, importing arbitrary URLs, app configuration, or a general-purpose remote desktop.
- Remote access over the public internet, port forwarding, UPnP, cloud relays, or automatic router changes.
- Video streaming or a live GPU preview over the network. Start with status and controls; a low-rate thumbnail is a separate measured feature.
- MIDI/OSC implementation. The API should align with the existing command model and leave room for future transports without blocking this feature on them.

## Proposed architecture

```text
Phone / tablet browser
  └─ same-origin HTML/CSS/JS app
       ├─ HTTP: bootstrap, pair, initial state, health
       └─ WebSocket: authenticated commands + state/events
                 │
                 v
Strobe LAN service (dedicated server thread/runtime)
  ├─ pairing and client authorization
  ├─ origin/host validation, input validation, rate limits
  ├─ bounded command channel ───────────────┐
  └─ read-only state subscription           │
                                            v
                              App event-loop thread
                              ├─ dispatch existing commands
                              ├─ owns playlist/session changes
                              ├─ produces a compact state snapshot
                              └─ never blocks on network I/O
```

### Integration boundary

- Add a transport-independent remote command/event model near the app's command layer. Reuse existing command semantics for playback operations; do not send raw key codes through the network.
- The server publishes messages into a **bounded channel**. If the queue is full, reject the request with a clear busy response; never block a frame. Validate commands again on the app thread and return an explicit accepted/rejected result.
- Publish a compact read-only snapshot at a modest cadence (start at 5–10 Hz) and immediately after meaningful state changes. Coalesce updates so a slow phone cannot build an unbounded queue. The phone does not need a 60/144 Hz stream.
- Keep all GL and scene operations on the existing GL-owning thread. No network callback may call `Gl`, `ScenePlayer`, egui, or mutate playlist/session state.
- Prefer one service lifecycle managed by the app: disabled by default, start/stop idempotently, report bind errors in Diagnostics, and shut down cleanly.
- Serve versioned API messages (e.g. protocol version 1) and static assets from the same origin. Unknown message types and fields fail closed.

### Dependency choice

The current workspace has no general HTTP/WebSocket server dependency; the app uses `winit`/egui and the Shadertoy import path uses blocking `ureq`. Do not put a blocking listener in the frame loop. Compare a small maintained Rust HTTP/WebSocket stack (for example Axum + Tokio) against a smaller dedicated-thread option before implementation. The choice should be made with a measured dependency/build-size cost, Windows release packaging, clean shutdown, and testability in mind. If an async runtime is selected, create it on a dedicated server thread and bridge through bounded channels; do not turn the render loop async.

## Network and PWA constraints

### Same-origin hosting is the simplest v1

Serve the HTML, CSS, JavaScript, manifest, and API from the same origin and port. This avoids CORS configuration and avoids having an externally hosted HTTPS page reach into the user's LAN. It also keeps the controller usable without internet access and means the controller talks to the exact Strobe instance it was opened from.

Use an explicit opt-in listener on a selected LAN interface or a clearly displayed LAN address. Do not bind all interfaces silently. Do not expose the service on the internet, create firewall rules automatically without consent, or attempt UPnP. The UI must show the exact address, port, and listening interface.

### The HTTPS decision is a release gate for true PWA support

- Secure contexts are required for service workers and PWA capabilities. A LAN IP over plain HTTP does not qualify just because it is private.
- HTTPS on a LAN IP is not free: certificates must match the host name/IP, and mobile clients must trust the issuer. A self-signed certificate commonly produces warnings and is a poor onboarding path; do not make "ignore the warning" part of normal setup.
- Candidate approaches to investigate before promising installability:
  1. **User-managed local HTTPS/reverse proxy** with a trusted certificate and local DNS name. Powerful for advanced users, but not a zero-config default.
  2. **Pairing through a separately trusted HTTPS origin** that connects to the local host. This adds browser Local Network Access permission, mixed-content, CORS/origin, and browser-support complications; it is not the preferred v1 architecture.
  3. **A future supported local certificate enrollment/trust workflow**, only if it can be secure, comprehensible, reversible, and tested on Android and iOS without asking users to bypass warnings.
- Do not depend on experimental browser APIs such as `Request.targetAddressSpace` as a universal solution. Browser support and local-network permission behavior vary and change over time.
- Until HTTPS is solved and tested, describe v1 as a **LAN web remote**. A manifest or home-screen shortcut alone does not make it a full PWA.

## Security model (must be designed before opening a socket)

Treat the LAN as untrusted. A shared Wi-Fi network, compromised guest device, malicious browser tab, or DNS rebinding attempt must not be enough to take over a live show.

1. **Opt-in and visible lifetime.** Remote server is off by default. Show an unmistakable enabled indicator and a one-action Stop button in the desktop app. Stop the listener on exit and provide a "revoke all" action.
2. **Pairing, not a shared permanent password.** Generate a cryptographically random pairing secret using the OS CSPRNG. Display it only while pairing is open, expire it quickly, rate-limit attempts, and require local confirmation on the desktop. Pairing grants a separate high-entropy per-client bearer token; never put long-lived tokens in QR URLs, query strings, logs, browser history, or referrers. Revoke individual clients and all clients.
3. **Authenticate every state-changing request.** The pairing page may be unauthenticated, but commands and sensitive state require an authorized client. Do not treat an unguessable port or local IP as authentication.
4. **Validate Host and Origin.** Accept only expected host values and the same-origin browser Origin for WebSocket upgrades and state-changing HTTP requests. Reject unexpected origins; do not use wildcard CORS. Protect against DNS rebinding and cross-site WebSocket hijacking. A token alone is not a substitute for origin validation.
5. **Limit scope.** Expose a strict allowlist of typed show-control commands and a read-only status schema. No arbitrary command strings, shell, filesystem, script evaluation, shader source injection, or arbitrary URLs. Validate all numeric values and lengths.
6. **Transport confidentiality.** Plain HTTP/WebSocket on a LAN does not provide confidentiality or integrity against a hostile Wi-Fi participant. Pairing does not solve eavesdropping after the token is issued. Resolve TLS for a general-purpose release, or explicitly document the residual risk and keep the plain-LAN mode restricted to trusted networks with no claim of secure transport. Prefer HTTPS/WSS for production use.
7. **Abuse controls.** Rate-limit pairing and commands; cap WebSocket message size and client count; bound outbound queues; set idle timeouts; handle malformed JSON without panics; redact credentials from logs. Require no internet connectivity.
8. **Firewall behavior.** Windows Firewall may block incoming LAN connections. Explain the required user-approved private-network rule. Never automatically expose the port on public networks; ideally detect/report the active network profile and refuse to start when it is Public.
9. **No hidden privilege escalation.** A remote client can control the show only. Pairing must not grant local OS permissions or broad access to Strobe's data directory.

## API sketch (not a finalized contract)

Keep messages small, typed, and versioned. Example conceptual messages:

- Client → server: `hello { protocolVersion }`, `pair { pairingCode }`, `command { requestId, command, args }`, `ping`.
- Server → client: `paired { clientId }`, `state { revision, currentCue, nextCue, remainingMs, held, blackout, outputVisible, audioStatus }`, `commandResult { requestId, accepted, reason }`, `error { code, message }`, `pong`.
- Commands should use stable identifiers such as `next_scene`, `prev_scene`, `toggle_hold`, `restart_cue`, `extend_cue`, `skip_section`, `toggle_blackout`, and `tap_tempo`, but only enable commands that are valid in the current app state. Scene navigation without a playlist and cue navigation with a playlist need explicit semantics rather than silently doing the wrong thing.
- Include a monotonically increasing state revision. On reconnect the client requests a fresh snapshot; revisions let the UI ignore stale updates.
- Avoid sending the whole scene library or shader metadata by default. If scene selection is later added, expose a paginated, read-only library endpoint with bounded search.

## Current implementation status

Phase 1 is built (D250 to D254): an opt-in listener on its own threads, with our own small HTTP and WebSocket and `sha1` the only new crate; pairing with explicit desktop approval (a one-use six-digit code first, until D468 and D469 made it an approval alone, for phones under a name); 256-bit per-client tokens kept as digests, rotated on every authentication and revocable; Host (IP literal only) and Origin checks; message, connection, rate, idle and slow-client limits; and the one-file controller page under a hash-based CSP. Adversarial tests run over real sockets in CI (`crates/strobe-remote/tests/server.rs`). Phase 2's manifest, app icons, service worker and update behaviour are built too (D517): the worker is network first, keeps only the page and icons, and cannot issue a command, and an update needs no prompt. What is **not** done: real Android and iOS devices on Wi-Fi (installed or not), the Windows firewall prompt, the T480 soak, Performance Macros (protocol v2), a QR code, and the rest of Phase 2.

## Delivery plan

### Phase 0 — resolve the hard questions

- [ ] Decide whether first release is LAN web remote only or includes true PWA installation.
- [ ] Prototype and document HTTPS/trust options on Android Chrome and iOS Safari; do not settle for desktop-only validation.
- [ ] Confirm app command routing and current authoritative state sources for playlist/no-playlist mode.
- [ ] Compare server dependencies, binary size, Windows firewall behavior, and a clean stop/start lifecycle.
- [ ] Write a threat model and protocol schema before implementing the listener.

### Phase 1 — LAN remote MVP

- [ ] Implement a typed remote protocol and bounded app command channel.
- [ ] Serve embedded static assets and HTTP endpoints on an explicitly enabled LAN listener.
- [ ] Implement pairing, token storage/revocation, Origin/Host checks, rate limits, message limits, and credential redaction.
- [ ] Build the touch-first controller and connection/reconnect states.
- [ ] Add desktop settings for enable/disable, interface/address, port, paired clients, revoke, and current connection count.
- [ ] Add a Diagnostics record for bind errors and server lifecycle without logging secrets.
- [ ] Add tests for authorization, expired pairing, revocation, origin rejection, malformed/oversized messages, rate limiting, bounded queue saturation, reconnect/resync, and disabled-by-default behavior.

### Phase 2 — PWA and hardening

- [ ] Implement and test a supported HTTPS story before advertising service-worker-backed PWA installation.
- [ ] Add manifest, app icons, service worker, versioned app-shell caching, and explicit update behavior.
- [ ] Ensure cached UI cannot continue issuing commands after the server is disabled or the session is revoked.
- [ ] Test Android Chrome and iOS Safari on the same Wi-Fi, including sleep/resume, rotation, network changes, browser restarts, and pairing recovery.
- [ ] Test Windows Firewall prompts, public-network refusal, multiple clients, hostile Origin/Host headers, and network loss during a command.
- [ ] Measure idle CPU/memory and frame-time impact on the T480/Intel UHD 620; target negligible impact when idle and no render-thread blocking under load.

## Acceptance criteria

- Remote control is off by default, cannot be enabled accidentally, and has an obvious desktop kill switch.
- A phone on the same Wi-Fi can pair and control a running playlist without cloud access.
- Every command is authorized, validated, acknowledged, and applied only on the app thread; rejected commands are visible to the operator.
- Reconnect restores the current snapshot without stale or duplicated actions.
- A malformed client, slow client, disconnect, or full command queue cannot crash Strobe or stall a frame.
- No API permits arbitrary code execution, arbitrary filesystem access, or external network control.
- Android Chrome and iOS Safari are tested on real devices before release claims; PWA installation is claimed only after HTTPS, service-worker, and installability checks pass.
- The T480 soak shows no meaningful frame-time regression and no unbounded memory growth.

## Research sources

1. **MDN — Local network access.** Documents local/loopback permissions, secure-context restrictions, and their impact on fetch, WebSocket, and other network requests. This is particularly relevant if considering an externally hosted controller that reaches into the LAN.  
   https://developer.mozilla.org/en-US/docs/Web/Security/Defenses/Local_network_access
2. **Google Chrome — Local Network Access guidance.** Explains the secure-context requirement, same-origin/LAN request implications, and why deprecated Private Network Access preflight headers are not a solution. Browser support continues to evolve, so verify target versions during implementation.  
   https://github.com/GoogleChrome/modern-web-guidance/blob/main/skills/modern-web-guidance/guides/security/local-network-access.md
3. **MDN — Mixed content.** Explains why active mixed content such as network requests and WebSockets from HTTPS pages to insecure endpoints is generally blocked.  
   https://developer.mozilla.org/en-US/docs/Web/Security/Defenses/Mixed_content
4. **OBS Project — obs-websocket.** A mature adjacent example of controlling live audiovisual software over WebSocket; its documentation explicitly recommends authentication and describes phone/tablet control on the same LAN. Use as a product/operational reference, not as a security proof for Strobe's design.  
   https://github.com/obsproject/obs-websocket
5. **OBS Project — Remote Control Guide.** Describes configuration and authentication expectations for WebSocket remote control.  
   https://obsproject.com/kb/remote-control-guide

## Decision to carry forward

**Proceed with a same-origin, opt-in LAN remote and typed WebSocket command bridge. Do not promise a full installable PWA until HTTPS and mobile certificate trust are solved.** This preserves the desired phone/tablet workflow without basing the feature on browser workarounds or weakening Strobe's security model.
