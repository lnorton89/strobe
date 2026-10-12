# LLM-assisted shader creation and iteration

**Status:** research and implementation plan; no runtime code or API integration is introduced by this document-only PR.

## Executive summary

**This is feasible and fits Strobe well, provided the LLM is treated as an untrusted shader author, not as a renderer or agent with unrestricted access to the machine.** Strobe already has the key ingredients: a scene folder format, SSF GLSL and Shadertoy import, deterministic scene parsing, a renderer that can compile/render scenes, and diagnostic logs. A prompt-to-scene workflow can reuse these rather than inventing a separate shader runtime.

Recommended first release:

- A dedicated **Create with AI** panel where a user describes a visual effect.
- A provider-neutral API interface, with one cloud provider implemented first and user-supplied API key/model settings.
- A strict structured response containing a scene name, description, GLSL source, optional controls and a short rationale.
- A safe pipeline that validates scene structure, compiles and renders a candidate off the main frame path, then inserts it as a new scene only after the user previews and accepts it.
- An iterative chat tied to that scene. Each change produces a new candidate/revision; the user can accept, reject, compare, or roll back.
- No automatic overwrite of existing scenes, no arbitrary shell/file/tool access for the model, and no API call during normal playback.

The hard part is not getting an LLM to output GLSL. It is producing **valid, visually interesting shaders that follow Strobe's actual SSF conventions**, making the result testable on both native GL and ANGLE, keeping feedback iterations cheap, and preventing malformed output from disrupting a live show.

## Fit with the current codebase

Repository architecture suggests these boundaries:

- `strobe-scene`: the canonical `.synScene` structure, `scene.json`, shader assembly, and scene validation.
- `strobe-gfx`: actual GLSL compilation and rendering. It must remain the source of truth for whether a candidate works on the current context; a text parser alone is not enough.
- `strobe-app`: prompt panel, conversation/revision UI, preview/accept/reject actions, provider settings and status.
- `strobe-diag`: structured errors and diagnostics for failed generation, validation, compilation and rendering.
- `strobe-autocontrol`: potential later integration to discover useful parameters from a successful shader. Do not make it mandatory for the first version.
- `strobe-shadertoy`: useful examples of converting an external shader into the existing scene model; not a dependency the renderer should take on.

Preserve the architecture rule that `strobe-gfx` must not depend on the app, LLM client, or shader-generation feature. Keep network/API code and secrets out of the render thread and out of the graphics crate. The generation workflow should orchestrate existing crate interfaces from the app/service layer.

## User experience proposal

### 1. Create a scene

A user opens **Create with AI**, enters a prompt such as:

> Make an audio-reactive liquid chrome tunnel with slowly morphing geometric reflections, bright cyan highlights, and a restrained amount of motion. It should stay readable on a projector.

Optional structured settings:

- **Style:** abstract, geometric, organic, fractal, retro, minimal, user-defined.
- **Audio reactivity:** none, subtle, medium, strong.
- **Complexity / GPU budget:** lightweight, balanced, ambitious.
- **Controls:** none, a few useful controls, or more exploratory controls.
- **Compatibility:** default to Strobe's existing lowest supported shader dialect and SSF conventions.

The user should not have to choose shader dialect or API details for ordinary use. Advanced settings can expose provider/model, token budget, iteration cap, and whether the prompt may use a reference image if that capability is added later.

### 2. Generate and validate

Show explicit stages rather than a spinner with no information:

1. Planning the scene.
2. Generating GLSL and scene metadata.
3. Checking the scene structure and shader source.
4. Compiling on the active backend.
5. Rendering a preview and checking for obvious failures.
6. Ready to preview.

If compilation fails, return the compiler diagnostic and a compact failure summary to the model for a bounded repair attempt. Never run an unbounded “fix until it works” loop. A failed candidate remains a draft; it must not replace the current scene.

### 3. Preview and accept

Preview the candidate in a separate scene state or isolated preview slot. The preview must not reset the live show, replace the active playlist item, change global controls, or commandeer the output/projector window.

Actions:

- **Use this scene** — save as a new `.synScene` directory in the user's library.
- **Ask for changes** — continue the conversation using the accepted candidate and relevant diagnostics as context.
- **Try again** — request a new candidate from the same original prompt.
- **Discard** — remove the draft and its temporary assets.
- **Compare / revisions** — switch between accepted and draft versions.
- **Undo / restore revision** — return to a known-good generated revision.

Default to previewing in the control window, not fullscreen output. An explicit “send to output” action is a separate performance decision.

### 4. Iterate conversationally

Each user message should be interpreted as a patch request against a specific revision, e.g. “make the tunnel wider and reduce the strobing, but keep the color palette.” Send the model:

- Original creative goal and current user instruction.
- Current shader source and relevant scene metadata.
- Strobe's compact SSF/GLSL contract and approved helper functions.
- Previous revision summary.
- The most recent compiler/runtime diagnostic if the last candidate failed.
- Constraints: target GLSL profile, render budget, available audio uniforms, and known supported features.

Request a complete replacement shader or an explicit structured patch. **For the first version, complete replacement source is simpler and more reliable than free-form diffs.** Patch generation can be evaluated later, but must be applied and checked by deterministic code, never trusted as raw text edits.

Persist the prompt history and revisions locally so the user can reopen and continue an idea without another service. Offer an option to clear history; it may contain private creative work.

## Model/API architecture

### Provider-neutral boundary

Define a small internal trait/interface such as `ShaderGenerationProvider` with operations to generate a scene and revise a candidate. Its input/output should be Strobe-owned data types, not provider SDK types.

Suggested logical request:

- task: `create_scene` or `revise_scene`
- user prompt and bounded conversation history
- scene contract/version
- current source and metadata for revisions
- target profile and constraints
- relevant validation errors
- output token limit and cancellation signal

Suggested structured response:

- `title`
- `description`
- `shader_source`
- `controls[]` with name, type, default, min/max, and purpose
- `visual_intent` / concise change summary
- optional warnings and unsupported-feature notes

Use provider-side structured outputs / JSON-schema-constrained generation where available, then independently deserialize and validate locally. Schema adherence guarantees structure, **not correct GLSL or an attractive image**. OpenAI and Anthropic both document structured-output features; provider capability, model names and SDK versions should be selected at implementation time rather than hard-coded into this research plan. See references.

### API key and cost policy

- User supplies their own provider API key in a settings screen; no Strobe-owned proxy or server is needed for the MVP.
- Store secrets using the platform's credential store if a suitable cross-platform crate and target support are verified. Do not store raw keys in `scene.json`, TOML settings, logs, crash reports, URLs, or prompt history. If a secure credential store is unavailable, make the fallback explicit and avoid pretending plain-text storage is secure.
- Send only the current scene and the context needed for the task. Never upload the whole scene library, audio stream, microphone samples, logs, or crash dumps by default.
- Show estimated token usage/cost when the provider supports a reasonable estimate; show the selected model and max output budget.
- Set per-request token limits, a maximum automatic repair count, cancellation, and an explicit user-triggered retry. No background generation or periodic API requests.
- Treat provider errors, rate limits, timeouts, and offline mode as ordinary recoverable UI states. Existing scene playback must continue.
- A BYOK model avoids hosting an API backend, but does not mean inference is free: the user pays the provider's current usage rates. Local inference can be a later provider implementation, not a prerequisite.

### Do not give the model tools in v1

The model does not need shell access, arbitrary file reads, internet browsing, package installation, or general-purpose code execution. It only needs to return the constrained scene payload. This reduces both risk and complexity. The host app owns file writes and only writes a new scene after validation and user acceptance.

## Validation pipeline and quality gates

A successful HTTP response must not be treated as a successful shader.

1. **Response validation:** schema, required fields, maximum lengths, and supported enum values.
2. **Scene validation:** parse through `SceneDef::parse`; reject unsafe or unsupported paths/assets and validate control names/ranges.
3. **Source policy:** enforce size and pass limits, require the expected SSF entry point and supported GLSL profile, reject unexpected external includes or undeclared dependencies. Do not attempt to create a perfect GLSL security sandbox with regexes.
4. **Real driver compilation:** use the existing renderer's actual compile path. Naga is not a drop-in validation path for this GLSL corpus; Strobe's current architecture deliberately uses native GL/ANGLE because real SSF and Shadertoy shaders rely on GLSL features the Naga frontend rejects.
5. **Render smoke test:** render a bounded number of preview frames at a conservative resolution and verify the framebuffer/readback is not entirely NaN/Inf or a known failure color, where practical. A black image is not always a bug, and image heuristics must not automatically reject artistic intent.
6. **Performance budget:** measure GPU/CPU frame time and warn on expensive candidates. Never promise an absolute frame rate across unknown hardware. If a candidate exceeds the preview budget, offer lower resolution or a lighter revision.
7. **Compatibility:** compile and render on the active backend first; use CI/headless Mesa plus both GLSL dialects where available. A shader passing native GL is not proof it works on ANGLE.
8. **Commit only after approval:** write into a new temporary directory, validate the written scene, then atomically move it to a unique destination. Never overwrite an existing scene. Failed drafts are easy to inspect or discard.

### Keep the output thread safe

- Network requests, JSON processing, disk writes, retry waits and prompt assembly happen on a worker thread.
- All GL work stays on the thread owning the current context and goes through existing render-job/offscreen mechanisms; do not move a `Gl` value to the network worker.
- The live show keeps rendering while a model call is in flight.
- Cancellation must invalidate late responses so a previous prompt cannot unexpectedly overwrite a newer candidate.
- Bound generated source length, number of passes, render duration, and memory use. A GPU shader can hang or trigger driver problems even when it contains no unsafe CPU code; preview isolation and timeout/reset behavior need explicit testing.

## Iteration strategy

Use a deterministic state machine, not an autonomous agent loop:

`Idle → Generating → Validating → PreviewReady | NeedsRepair | Failed → Accepted`

- One user action produces one candidate revision.
- Allow at most 1–2 automatic repair attempts per user action, configurable in advanced settings.
- After that, show the actual diagnostic and let the user choose whether to ask for another revision.
- Every revision records its parent ID, prompt, model/provider ID, timestamp, shader hash, validation result, and user acceptance state.
- The model gets the current accepted source plus the latest relevant failure, not the entire conversation forever. Summarize old turns to limit cost while preserving user intent.
- If a repair changes the visual direction, let the user compare it against the prior draft before accepting.

## Initial implementation stages

### Phase 0 — prove the creative loop

Build a developer-only prototype that calls one provider and writes candidate JSON into a temporary scene directory. Use a small hand-curated prompt set covering gradients, geometric tunnels, SDF/raymarching, audio-reactive effects, feedback/multipass, and user-requested revisions. Measure compile success, visual quality, and average repair count.

### Phase 1 — safe scene generation

- Provider interface and configuration.
- One cloud provider with BYOK.
- Structured scene response and hard limits.
- Candidate creation through `strobe-scene`.
- Real GL compile + preview using existing renderer mechanisms.
- Accept/reject and save-as-new-scene only.
- Local prompt/revision history.
- Tests with canned model responses; CI must not require network or a secret API key.

### Phase 2 — polished iteration UX

- Conversation tied to selected scene.
- Revision history, visual comparison, undo and restore.
- Feed compiler errors into bounded repair.
- Copy/export prompts and scene bundles.
- Per-provider usage/cost information and cancellation.

### Phase 3 — performance and quality improvements

- Automated visual regression and representative prompt evals.
- Optional auto-control generation/scoring after a valid shader exists.
- Local model provider if a stable API-compatible runtime is worthwhile.
- Optional reference-image prompting only with an explicit provider/privacy disclosure.
- Optional preset prompt templates for common VJ looks, without locking the UI into a marketplace or cloud account.

## Evaluation plan

Do not evaluate only on “did the model return code?” Maintain a versioned, permissively licensed prompt suite with expected behavior and render snapshots.

For every prompt, record:

- response parse/schema success
- scene parse success
- shader compile success on native GL and ANGLE where available
- first-preview success and time to preview
- number of repair attempts
- visual similarity/stability between revisions (not a single universal aesthetic score)
- GPU timing at fixed preview resolution
- useful-control rate and control ranges
- token usage and estimated cost
- whether user acceptance/rejection matches human review

Include regression prompts for shader dialect, multipass/feedback, audio uniforms, texture bindings, aspect correction, and low-end hardware. Have human review for aesthetics; no automated metric can decide if a shader is beautiful or appropriate for a live show.

## Main risks and mitigations

| Risk | Mitigation |
|---|---|
| Plausible but invalid GLSL | Real compiler gate and bounded diagnostic repair |
| Valid but visually dull output | Iterative prompt UI, reference prompts, eval suite and human feedback |
| Wrong SSF API or nonexistent audio uniform | Provide a concise versioned contract and examples from Strobe's own licensed scenes |
| Very expensive shader | Low-resolution bounded preview, GPU timing and user-visible warnings |
| Candidate disrupts the show | Isolated draft state, control-window preview, explicit accept/send-to-output |
| API spend surprises user | BYOK, model selection, output limits, retry cap, optional cost estimate |
| API key leaks | OS credential store, redacted logs, never include in scene/report |
| Provider unavailable | Local scenes keep working; save prompt and allow retry |
| Prompt history exposes creative work | Local-only by default, explicit clear/export controls |
| Shader causes GPU stall/driver reset | Conservative preview budgets and existing context-loss recovery; investigate stronger isolation before shipping automatic full-resolution preview |

## References

- [OpenAI Structured Outputs guide](https://developers.openai.com/api/docs/guides/structured-outputs) — JSON Schema-constrained responses; useful for typed scene payloads. Structured output does not guarantee semantic correctness.
- [Anthropic structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs) — JSON outputs and strict tool use; potential alternative provider.
- [Naga documentation](https://wgpu.rs/doc/naga/index.html) — shader parsing/validation/transpilation; useful context, but not a replacement for Strobe's current real-GL compile gate.
- [wgpu shader support](https://wgpu.rs/doc/wgpu/documentation/shaders/index.html) — GLSL/SPIR-V/WGSL support in wgpu; Strobe currently avoids this frontend for its compatibility-sensitive GLSL corpus.
- [Strobe architecture](ARCHITECTURE.md) — crate boundaries and render-thread ownership.
- [Strobe plan](PLAN.md) — SSF conventions, shader APIs, audio uniforms, multipass and Shadertoy compatibility.
- [Strobe diagnostics](../crates/strobe-diag/README.md) — structured failures and local logs to feed into repair safely.
- [Strobe scene library](../scenes/) — source of examples for prompt context and regression fixtures; use only content with known project-compatible licensing.
