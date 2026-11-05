# Compatibility

This is the supported, degraded and unsupported matrix for Strobe, for
people running it and for contributors. Every status points at its
evidence: a decision in [DECISIONS.md](DECISIONS.md), a test, a hardware
record ([T480](hardware/T480.md), [TORNADO](hardware/TORNADO.md),
[AMD desktop](hardware/AMD.md)) or an
item in [STATUS_AUDIT.md](STATUS_AUDIT.md). STATUS_AUDIT.md is the
evidence log, and it wins when this file disagrees with it. Written
2026-10-13; rows brought up to date on 2026-10-26 for `textureFinalPass`,
the release workflow, ANGLE's backends and the LAN remote, and on
2026-11-04 for the release zip, the webcam and the installable pages.

## Status words

| Status | Meaning |
|---|---|
| **Supported** | Works, and the evidence column says where that was shown. |
| **Degraded** | Works, with a limitation stated in the notes. |
| **Unsupported** | Not available. |
| **Unverified** | Built, but the evidence is only synthetic (tests, llvmpipe, injected faults) or missing on the target hardware. |

A type, a parser or a UI control existing is never enough for
"Supported". Where a feature depends on real hardware or real music and
the only evidence is synthetic, it is Unverified.

The machines: **T480** is a ThinkPad with Intel UHD 620 on Windows 11
(the integrated-GPU reference). **TORNADO** is a GTX 1080 Ti on Windows 10.
**AMD desktop** (DESKTOP-30KSOCM) is a Radeon RX 580 on Windows 10.
**llvmpipe** is Mesa's software rasteriser in CI and the cloud
container, headless or under Xvfb.

## 1. SSF scene format

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Scene folder (`main.glsl`, `scene.json`, optional `script.js`) | Supported | [TORNADO T18](hardware/TORNADO.md), [T480 Task 30](hardware/T480.md) | Loaded through the app's own path by `cargo xtask corpus` ([D195](decisions/D195.md)). |
| CRLF and `\r\r\n` line endings | Supported | [D193](decisions/D193.md), [D201](decisions/D201.md), `load.rs` `windows_line_endings_are_normalised_at_load` | Every line-ending form becomes one `\n`, so error lines match editors. |
| `CONTROLS`: `slider`, `knob`, `toggle`, `bang`, `xy`, `color`, `dropdown` | Supported | `crates/strobe-scene/src/control/kind.rs` `every_documented_type_parses`, `runtime.rs` tests, `render.rs` `a_control_value_reaches_the_shader` | `color` is `vec3` ([R2](OPEN_QUESTIONS.md)). MIN above MAX is kept ([R3](OPEN_QUESTIONS.md)). |
| Control modifiers `smooth`, `speed`, `counter` | Supported | `runtime.rs` `smoothing_takes_the_same_wall_time_at_any_frame_rate`, `speed_integrates_the_target_per_second`, `counter_presses_between_frames_are_not_lost` | Smoothing is frame-rate independent ([D9](decisions/D9.md)). |
| `bang smooth` envelope | Degraded | `runtime.rs` `a_smooth_bang_rises_then_falls` | The envelope shape is a guess ([Q4](OPEN_QUESTIONS.md)). |
| `traveler` modifier | Degraded | [D197](decisions/D197.md), `runtime.rs` `a_traveler_accelerates_toward_its_speed_and_integrates_it` | Undocumented by SSF; Strobe's reading is an inference from 9 shipped scenes. |
| A control with no `TYPE` | Supported | [D193](decisions/D193.md), `scene.rs` `real_synesthesia_shapes_load` | Two default components make an `xy`, anything else a `slider`. |
| `PASSES`: `TARGET`, multipass, ping-pong | Supported | `render.rs` `a_buffer_pass_feeds_the_final_pass`, `a_double_buffered_pass_reads_its_previous_frame` | |
| Pass sizes: literal `WIDTH`/`HEIGHT` | Supported | [D194](decisions/D194.md), [R7](OPEN_QUESTIONS.md), `scene.rs` `literal_pass_sizes_are_absolute_and_expressions_follow_the_output` | Literal sizes are absolute pixels, as in Synesthesia. `$WIDTH` expressions follow the output ([D11](decisions/D11.md)). |
| `FIXED_SIZE` on a pass | Supported | [D191](decisions/D191.md), `render.rs` `a_fixed_size_pass_keeps_its_grid_when_the_output_resizes` | A Strobe extension, not SSF. |
| `FLOAT` passes | Degraded | [D13](decisions/D13.md), `render.rs` `a_full_precision_pass_keeps_counting_where_half_floats_stall` | SSF documents 32-bit; Strobe defaults to RGBA16F for speed on integrated GPUs. 32-bit is available (`FloatPrecision::Full`), not per scene. |
| A pass that samples its own target | Supported | [D203](decisions/D203.md), [R9](OPEN_QUESTIONS.md), [TORNADO T8](hardware/TORNADO.md) | Double-buffered automatically, so native and ANGLE agree. |
| Implicit final pass | Supported | [D8](decisions/D8.md), [R1](OPEN_QUESTIONS.md), `render.rs` `a_buffer_pass_feeds_the_final_pass` | `renderMain` runs once more with `PASSINDEX == PASSES.len()`. |
| `PASSINDEX` compared as int or float | Supported | [D7](decisions/D7.md), [D163](decisions/D163.md), [R4](OPEN_QUESTIONS.md), `render.rs` `passindex_compared_with_a_float_literal_compiles_in_both_dialects`, `int_passindex_comparisons_still_compile_in_both_dialects` | A compile-time constant per pass program. |
| Desktop-only GLSL idioms on ANGLE (implicit int to float, `#if` on a non-macro, initialised uniforms, function-like macros mixing int and float, a sampler chosen with `?:`) | Supported | [D199](decisions/D199.md), [D261](decisions/D261.md), `render.rs` `implicit_int_to_float_and_non_macro_if_compile_in_both_dialects`, `function_like_macros_mixing_int_and_float_compile_in_both_dialects`, `a_sampler_chosen_with_a_ternary_compiles_in_both_dialects`, [TORNADO T18](hardware/TORNADO.md) | A sampler `?:` is rewritten only as the one such argument of a function call (`texture(c ? a : b, uv)`); anywhere else the ES program fails with Strobe's own message at the user's line (`a_sampler_choice_that_cannot_be_rewritten_fails_with_strobes_message_on_es`). The macro and `?:` rewrites are checked on llvmpipe with original fixtures, not yet on ANGLE hardware or the owner's corpus. |
| `#include` | Degraded | [D198](decisions/D198.md), [D271](decisions/D271.md), `include.rs` tests | Strobe ships neither lygia nor hg_sdf. Works where Synesthesia is installed (found as the library finds it, `STROBE_SYNESTHESIA_DIR` included) or `STROBE_GLSL_PATH` points at a copy. |
| `IMAGES` | Supported | `render.rs` `image_wrap_filter_and_vflip_options_are_honoured`, `assets.rs` `a_missing_image_reports_a_problem_and_reads_black`, [D201](decisions/D201.md) | A missing picture is reported and reads black; the scene plays. Decoded by content, not extension ([D196](decisions/D196.md)). |
| `CUBEMAPS` | Degraded | [D185](decisions/D185.md), `assets.rs` `six_face_files_answer_each_direction_with_their_colour`, `a_cross_picture_gives_the_same_six_faces` | Face orientation is unverified against Synesthesia ([Q22](OPEN_QUESTIONS.md)); none of the 65 real scenes uses a cube map. Strobe's own `VFLIP` key loads the faces as Shadertoy does for a flipped cube ([D553](decisions/D553.md)). llvmpipe only. |
| `MEDIA` | Supported | See section 9 | |
| `HARD_TRANSITIONS`, `SMOOTH_TRANSITIONS` | Supported | [D489](decisions/D489.md), [D196](decisions/D196.md), `strobe_scene::transition` tests, `tests/transitions.rs` | On a song change (heard in live audio, D237, or the `new_song` command) a hard transition jumps to another listed value and a family crossfades linearly over `DURATION` (3 s when absent). Not yet compared with Synesthesia: the values at load, repeats and the default fade (Q33). Song detection is unproven on real music. |
| `TAGS` | Supported | [D205](decisions/D205.md), [D264](decisions/D264.md), `strobe_scene::tags` and `ui::tags` tests | Read leniently, normalised, written in place, for any library scene. |
| `IMAGE_PATH` | Supported | [D212](decisions/D212.md), [D222](decisions/D222.md), `ui::thumbs` and `snapshot` tests | Shown as the browser thumbnail; can be taken from the live frame. |
| Other keys (`IS_META`, `GPU`, `UNIFORMS`, `LABEL`, `X_STROBE`) | Degraded | [Q6](OPEN_QUESTIONS.md), [R11](OPEN_QUESTIONS.md) | Read and ignored. `IS_META` is Synesthesia's own per-control key (19 controls in 4 of its 87 shipped scenes, all `false`; TORNADO T25). A root `UNIFORMS` is not Synesthesia's; its own scenes use the key only inside `SMOOTH_TRANSITIONS`. |
| SSF function library (documented names) | Supported | [D492](decisions/D492.md), `strobe-ssf-lib` `every_documented_function_is_defined` (63 names), `render.rs` `the_ssf_library_compiles_in_both_dialects` and `ssf_helpers_compute_what_synesthesias_engine_computes`, [TORNADO T25](hardware/TORNADO.md) | Follows what Synesthesia 1.25.4's engine computes where it differs from the documentation (R19): `_rotate` in radians, clockwise; `_pulse` by half-width; `_toPolarTrue` in turns; its `_rand`, `_noise` and `_fbm`. The tilt-shift blur too (D494), its taps capped at 101 each side (the engine's count at the default `blurAmt` 5). Bit-exactness still depends on the driver. |
| Undocumented helpers used by real scenes | Degraded | [D197](decisions/D197.md), [D199](decisions/D199.md), [D492](decisions/D492.md) | Checked against the engine (TORNADO T25): `_hueSaturationContrastLEGACY` ([R17](OPEN_QUESTIONS.md)), `_ratioFix` ([R18](OPEN_QUESTIONS.md); Strobe always fills, the engine's default), the image family and the masks now behave as the engine's. Still not the engine's: `_loadUserImage` and family read `syn_Media` without the media meta controls' flips, invert, contrast and scale (neutral values), because Strobe has no media meta controls (C-01). |
| Scalar audio uniforms (`syn_Level`, `syn_*Hits`, `syn_*Time`, `syn_*Presence`, beat values) | Supported | `strobe_scene::audio::SCALARS`, live capture on [T480](hardware/T480.md) (script scene, 2026-10-09) and [TORNADO T19](hardware/TORNADO.md) | Strobe's own analysis; values are not compared numerically with Synesthesia's. |
| Undocumented audio names (`syn_InputLevel`, `syn_BassExpoCurve`) | Degraded | [D197](decisions/D197.md), [Q25](OPEN_QUESTIONS.md) | Stand-ins. The engine lists both, with `syn_PrimeBang`, among its audio analyser's outputs (TORNADO T25); what they measure is still unknown (S4). |
| `syn_Spectrum`, `syn_LevelTrail` textures | Unverified | `render.rs` `uploaded_audio_texels_reach_the_spectrum_and_trail_samplers` | `sampler1D` on desktop is confirmed ([R10](OPEN_QUESTIONS.md)). The spectrum is 1024 texels, as Synesthesia's ([D269](decisions/D269.md)); the channels' meanings and order match the bundled SSF reference (TORNADO T25). Frequency axes and trail direction are assumptions ([Q10](OPEN_QUESTIONS.md)). |
| `syn_FinalPass` | Supported | `render.rs` `syn_final_pass_is_last_frames_output` | llvmpipe, both dialects. |
| `textureFinalPass` (script) | Supported | [D162](decisions/D162.md), [D245](decisions/D245.md), `render.rs` readback tests, `script_uniforms.rs` `texture_final_pass_reads_the_picture_a_few_frames_late`, [T480 Task 33](hardware/T480.md) | 64x36, two frames late, never a stall. On the T480's window context, native and ANGLE both had exactly 2 frames of latency for about 0.24 ms a frame ([C-04](IMPLEMENTATION_BACKLOG.md)). |
| Meta controls (master, invert, brightness, gamma, contrast, hue, saturation, mirrors, Limit Colors) | Degraded | [D180](decisions/D180.md), [D207](decisions/D207.md), `crates/strobe-gfx/tests/post.rs`, [T480 Task 15](hardware/T480.md), [T480 Task 25](hardware/T480.md) | **Differs from Synesthesia's, now known ([R16](OPEN_QUESTIONS.md), TORNADO T25):** its order is mirrors, Limit Colors, invert, hue/saturation, contrast, gamma, brightness, master; its brightness multiplies; its contrast and gamma curve above 1; hue and saturation work in HSV; Limit Colors runs black, low, high, white; its mirror names are the other way round from Strobe's. Strobe's set also lacks Transition, Alpha Channel and the media group. Reopened as [C-01](IMPLEMENTATION_BACKLOG.md). About 1.1 ms GPU at 1440p on the T480; HD 4000 unmeasured. |
| Meta `Alpha Channel` | Unsupported | [D207](decisions/D207.md) | Waits for an output that carries alpha. |
| `_mouse`, `_muv`, `_click` | Unverified | [Q2](OPEN_QUESTIONS.md), [R12](OPEN_QUESTIONS.md) | Inside the preview Strobe matches the bundled SSF reference (TORNADO T25): `iMouse` pixels with `zw` negative while no button is down, buttons left/right/middle, `_muv` 0..1. What the mouse values hold outside the preview is unverified. |

## 2. SSF scripts (`script.js`)

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| `setup()` once, `update(dt)` each frame | Supported | `crates/strobe-script/src/conformance.rs` `setup_runs_once_and_its_control_writes_apply_on_the_first_frame`, [T480](hardware/T480.md) (`11-script-orbits`), [TORNADO T18](hardware/TORNADO.md) (scripts run) | QuickJS, built on Windows by CI's `windows` job and the T480. |
| A top-level `transition` function | Supported | [TORNADO T23](hardware/TORNADO.md), [Q23](OPEN_QUESTIONS.md) | Not part of SSF's documented script lifecycle (only `setup` and `update(dt)`). The 6 real scenes that define one leave it empty and never call it, so not calling it changes nothing. |
| `setUniform` uniforms, created at load | Supported | [D62](decisions/D62.md), [D161](decisions/D161.md), `script_uniforms.rs`, `conformance.rs` `uniforms_made_in_setup_are_listed_when_sealed` | The first frame's uniforms are compiled in before the show plays. |
| A uniform first set after load | Degraded | [D161](decisions/D161.md), `script_uniforms.rs` `a_uniform_that_appears_later_is_refused_and_nothing_compiles_after_load` | Refused with one console error, never recompiled. Create every uniform in `setup()`. |
| Legacy `uniforms` object and `inputs` reads | Supported | `conformance.rs` `deprecated_uniforms_object_is_scanned_after_update`, `deprecated_inputs_reads_the_same_values_and_is_read_only` | Both appear in real scenes ([Q23](OPEN_QUESTIONS.md)). |
| `setControl`, `setControlNormalized`, randomize and default (control and group) | Supported | `conformance.rs` `set_control_*`, `randomize_and_default_control_and_group` | Names match ignoring case, `_`, `-` and spaces. |
| Event handlers (`onChange`, `onOffToOn`, `onOnToOff`, `whileOn`, `whileOff`) | Supported | `conformance.rs` `every_event_type_fires_on_its_condition_across_frames` | "On" is `>= 0.5`. |
| Writes to `meta/` controls | Supported | [D160](decisions/D160.md), `conformance.rs` `meta_*`, `crates/strobe-app/tests/script_meta.rs`, `docs/images/post/` (Xvfb) | Scripts cannot read meta values; Synesthesia documents writes only, and its shaders read them as `_Brightness` and the like ([R16](OPEN_QUESTIONS.md)). |
| `syn_PrimeBang` | Degraded | [D199](decisions/D199.md), `conformance.rs` `syn_prime_bang_is_defined_and_never_fires` | Defined as 0; what fires it is unknown ([Q25](OPEN_QUESTIONS.md)). |
| Runaway and faulting scripts | Supported | `conformance.rs` `an_endless_loop_in_update_is_interrupted_and_the_frame_returns`, `three_faulted_frames_in_a_row_disable_the_script`, `a_memory_limit_breach_faults_cleanly_and_recovers` | 4 ms a frame, 64 MiB heap; the output keeps running. |
| Script cost in the benchmark | Unsupported | [D239](decisions/D239.md) | `strobe-bench` runs no scripts and says so. Quote the corpus harness's `frame_ms` instead. |

## 3. Real Synesthesia scenes

The owner's scenes are private and never committed. The counts come from
`cargo xtask corpus`, which loads each scene the way the app does,
scripts included, and renders it in both dialects ([D195](decisions/D195.md)).

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| The owner's 65 scenes on NVIDIA native GL 3.3 | Supported | [TORNADO T18](hardware/TORNADO.md) (commit 8c93df5) | 63 of 65. The 2 failures are the scenes' own bugs (a function that never returns, an output declared twice). |
| The same 65 on ANGLE/D3D11 (NVIDIA) | Supported | [TORNADO T18](hardware/TORNADO.md) | 63 of 65, the same two failures. |
| Native and ANGLE drawing the same picture | Degraded | [TORNADO T8](hardware/TORNADO.md), [TORNADO T9](hardware/TORNADO.md), [TORNADO T18](hardware/TORNADO.md) | Two scenes differ between backends from undefined behaviour in their own code (`pow(0,0)`, `fract(sin)` hashes). Strobe does not fix those. |
| The 65 scenes on Intel (T480) | Unverified | [T480 Task 30](hardware/T480.md) | The T480 has run only Strobe's 12 library scenes through the harness (12 of 12 in both dialects). |
| The 90 default scenes from Synesthesia 1.25.4, llvmpipe | Supported | [D268](decisions/D268.md) | 90 of 90 in both dialects, scripts on, at commit ed8ce70 plus D268. One condition: `STROBE_GLSL_PATH` points at Synesthesia's `lib` folder, which Strobe also finds when Synesthesia is installed (D198), because SkaiWalkerOG includes `hg_sdf.glsl`. |
| The same 90 on AMD RX 580, native GL and ANGLE | Supported | [AMD Tasks A4 and A5](hardware/AMD.md) | 90 of 90 in both dialects at 8dbc1bb (A5), with no notes. Before D268 it was 88 of 90 (A4). All 48 scripted scenes ran. |
| The 90 drawing the same picture on native and ANGLE (AMD) | Degraded | [AMD Task A4](hardware/AMD.md) | Of the 47 scenes the bench can render, 61 frames match or nearly match. Three differ: `PLUS` (`pow` of a negative base, undefined in GLSL; llvmpipe draws it black; since D301 the ES dialect spells `pow` out as the native drivers run it, which should bring ANGLE in line, not yet re-run on the RX 580), `canvas` (its feedback background) and `turbo_encabulator_fluid` (banding on ANGLE). Both of the last two subtract or seed with `fract(sin(…))` hashes, whose precision differs between drivers. The banding stays with RGBA32F passes, so the storage format is not the cause (A5). Synesthesia's own thumbnails of both scenes show what ANGLE draws, so AMD's native `sin` is the one that differs (A6). Four simulations diverge in detail. |
| A picture matching Synesthesia's own render | Degraded | [AMD Task A6](hardware/AMD.md) | Compared with Synesthesia's shipped thumbnails for 17 scenes, since the free tier has no OSC input to drive it. Same: 3; close (colour, camera or time only): 8; different: 5 (neon_city, a Strobe bug with literal-size passes under a lowered render scale, A9, fixed by D280 and no longer different in framing; beamers, colour and time only, A8; PLUS without demo media; canvas and turbo on AMD native); inconclusive: 2. |
| Scenes with root-sized absolute buffer passes under any render scale | Supported | [D280](decisions/D280.md), `render.rs` `a_final_pass_reading_a_root_sized_literal_pass_covers_it_at_any_render_scale` (both dialects: an original gradient fixture covers the whole frame at 960x540, 480x270 and 240x135) and `a_scene_without_a_texel_read_root_sized_literal_pass_still_follows_the_render_scale`; `pin.rs` tests; `neon_city`, `neuron_proximitors` and `bouncy_hexagons` from the private corpus on Xvfb/llvmpipe, native, at render scales 0.5 and 1.0 ([AMD.md](hardware/AMD.md)) | A scene that writes a root `WIDTH` x `HEIGHT`, has a literal buffer pass of exactly that size, and reads such a pass with `texelFetch` renders its final pass at that size, so `texelFetch(pass, ivec2(_xy), 0)` reads the whole buffer; the output stage scales it to the window. Two of the 90 are pinned (`neon_city`, `entombed_silicon_dreams`). The nine others with a root-sized literal pass sample it by uv and keep the render-scale lever on their final pass (`SkaiWalkerOG`, `audio_indicators`, `bouncy_hexagons`, `fractilian_foundation`, `hextunnel`, `logo_grid_pulses`, `neuron_proximitors`, `thresholder`, `videofx_1`). Adaptive resolution leaves a pinned scene at the ceiling, and for any scene stops lowering for a while when two drops in a row do not cut the GPU time. Not yet seen on a GPU: the RX 580 re-run is open. |
| Running Synesthesia's built-in scenes for a show | Unverified | [Q7](OPEN_QUESTIONS.md) | The EULA question is open. Strobe ships none of them; it reads the person's own install (D271). |
| Synesthesia's installed scenes in the library | Unverified | [D271](decisions/D271.md), `library.rs` `a_fake_install_is_listed_after_the_library_read_only`, `switching_synesthesia_off_lists_only_the_library`; `synesthesia.rs` tests; [TORNADO T25](hardware/TORNADO.md) | On by default where installed; Settings > "Show Synesthesia's scenes" or `--no-synesthesia` turns it off. The install is found at `Program Files (x86)\Synesthesia` or `Program Files\Synesthesia`, or `STROBE_SYNESTHESIA_DIR`. `release\` and `default\` are listed after the library folder. On TORNADO's real 1.25.4 install: 87 + 1 = 88 scenes on disk, and every one loads, compiles and renders read in place in both dialects (T25). The app's own listing, playing one, the refusal and *Copy to my library* on a real install still need a person at the keyboard ([I-05](IMPLEMENTATION_BACKLOG.md), S13). |
| Synesthesia's own scene folders (Settings > Custom Scene Folder) | Unverified | [D271](decisions/D271.md), `synesthesia.rs` `the_persons_scene_folders_come_from_synesthesias_settings`, `broken_or_missing_settings_mean_no_folders_of_the_persons_and_say_why` | Read from `vs.json` (`sceneFolders`, `primarySceneFolder`) and `preferences.json` (`PRIMARY_SCENE_FOLDER`, recursive search to `RECURSIVE_SCENE_SEARCH_DEPTH`, 10 by default) in `%LOCALAPPDATA%\Synesthesia\Data` or `STROBE_SYNESTHESIA_DATA`. Writable, as they are the person's. Fake files only so far. |
| Writing into Synesthesia's install | Unsupported | [D271](decisions/D271.md), `app.rs` `a_write_into_synesthesias_folder_is_refused_and_offers_a_copy`, `copy_to_my_library_makes_a_copy_strobe_may_write` | On purpose: tags, a picture, Editor saves and presets are refused with "Copy to my library", which copies the scene into the library under a free name. |

### Every scene (C-03, TORNADO T24)

One row per scene of the owner's 65, from T22's run at commit 9b6e046 (scripts on, NVIDIA native GL 3.3 and ANGLE/D3D11), classified in T24. Names only: the scenes themselves are private.

Constructs: `int2float` relies on implicit int-to-float conversion (D199); `images` declares IMAGES; `userimage` reads `syn_UserImage`; `multipass` has buffer passes; `selfread` has a pass that samples its own target (D203); `finalpass` reads `syn_FinalPass`; `media` has a MEDIA key; `if` uses `#if` on a non-macro; `inituniform` initialises a uniform; `script` has a `script.js`.

| Scene | Native | ANGLE | Category | Constructs |
|---|---|---|---|---|
| \[sh17a\]_rainbow_twister | compile_error | compile_error | source bug | - |
| `abstract_terrain_objects` | ok | ok | - | - |
| `abstract_terrain_objects_dup` | ok | ok | - | - |
| `alloy_plated_voronoi` | ok | ok | - | - |
| `bentorus` | ok | ok | - | - |
| `boreal` | ok | ok | - | int2float, images, userimage, finalpass |
| `bumped_sinusoidal_warp` | ok | ok | - | images |
| `cellular_lattice` | ok | ok | - | images |
| `chroa_cascade_dup` | ok | ok | - | int2float, userimage, script |
| `chroma_cascade` | ok | ok | - | int2float, userimage, script |
| `cinders` | ok | ok | - | int2float, userimage, multipass, selfread |
| `circuit_bending_dup` | ok | ok | unavailable asset | images, userimage, script |
| `cosine_palette_generator` | ok | ok | - | script |
| `cube_circle_sketch` | ok | ok | - | - |
| `cubic_truchet_pattern` | ok | ok | - | - |
| `curvature_-_parametric_3d` | ok | ok | - | - |
| `curvature_-_parametric_3d_1` | ok | ok | - | - |
| `diffusion` | ok | ok | needs user media | int2float, userimage, multipass, selfread |
| `digital_rain` | ok | ok | - | int2float, images |
| `dual_3d_truchet_tiles` | ok | ok | - | images, if |
| `dust` | ok | ok | - | int2float, images, userimage, multipass, selfread |
| `exoplanet_dup` | ok | ok | - | int2float, userimage, multipass, inituniform |
| `extruded_truchet_pattern` | ok | ok | - | int2float, if |
| `faux_layered_extrusion` | ok | ok | - | int2float, images |
| `fluid_body_dup` | ok | ok | - | int2float, userimage, multipass, script |
| `fork_materialma_deviltiger_729` | ok | ok | driver | int2float |
| `hello_world_end` | ok | ok | - | - |
| `hello_world_start` | ok | ok | - | - |
| `hexagonal_interlacing` | ok | ok | - | - |
| `hexagonal_maze_flow` | ok | ok | - | int2float, script |
| `hexagonal_maze_flow_larry` | ok | ok | - | int2float |
| `hyperbolic_poincare_weave` | ok | ok | - | - |
| `industrial_complex` | ok | ok | - | images |
| `kifs_walls_` | ok | ok | - | int2float |
| `magnetismic` | ok | ok | - | int2float, images, multipass, selfread |
| `mana` | ok | ok | - | int2float, userimage, multipass, script |
| `materialmakertest2` | ok | ok | driver | - |
| `maze_lattice` | ok | ok | - | - |
| `menger_sponge_variation` | ok | ok | - | images |
| `menger_variation_2` | ok | ok | - | images |
| `neon_lit_hexagons` | ok | ok | - | images |
| `new_scene` | compile_error | compile_error | source bug | - |
| `new_scene_1` | ok | ok | - | - |
| `new_scene_2` | ok | ok | - | media |
| `noise_flow_lines` | ok | ok | - | multipass |
| `overload` | ok | ok | - | int2float |
| `panspermia` | ok | ok | - | int2float, userimage |
| `quadtree_truchet` | ok | ok | - | - |
| `rainbow_planet` | ok | ok | - | media |
| `random_isometric_blocks` | ok | ok | - | - |
| `raymarched_hexagonal_truchet` | ok | ok | - | - |
| `round_voronoi_borders` | ok | ok | - | - |
| `scorched_earth` | ok | ok | - | int2float, images, userimage, multipass, finalpass |
| `smooth_noise_contours` | ok | ok | - | - |
| `smooth_voronoi_contours` | ok | ok | - | - |
| `synesthesia-uniforms-display` | ok | ok | - | images |
| `test_card_dup` | ok | ok | - | int2float |
| `transparent_lattice` | ok | ok | - | - |
| `triangle_grid_contouring` | ok | ok | - | - |
| `triangle_mesh_with_incircles` | ok | ok | - | - |
| `vein_melter_dup` | ok | ok | - | int2float, images, userimage, multipass, script |
| `warped_extruded_skewed_grid` | ok | ok | driver | images |
| `welcome_scene_dup` | ok | ok | - | int2float, images, multipass, script |
| `wet_stone` | ok | ok | - | - |
| `wet_stone_1` | ok | ok | - | - |

Per dialect: 63 ok, 2 `compile_error`. Categories: no issue 58, source bug 2, driver 3, needs user media 1, unavailable asset 1, Strobe gap 0. What the corpus exercises: int2float 23, images 18, userimage 13, multipass 11, script 9. None use cubemaps, `#include`, the `syn_Media` helpers or `textureFinalPass`.

## 4. Shadertoy import

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Exported JSON to a scene folder (`strobe-import`, the Import pane, a dropped `.json`) | Supported | [D90](decisions/D90.md)-[D95](decisions/D95.md), `crates/strobe-shadertoy/tests/convert.rs` and `crates/strobe-shadertoy/tests/render.rs`, [T480 Task 11](hardware/T480.md), [TORNADO T1](hardware/TORNADO.md) | Eleven original fixtures render on llvmpipe; the first nine also on Intel and NVIDIA, native and ANGLE. |
| Buffers A to D, multipass order and feedback | Supported | [D95](decisions/D95.md), [T480 Task 11](hardware/T480.md) (four-buffer chain) | Every buffer is double-buffered, even one that never reads itself. |
| Texture channels from local files | Supported | [D95](decisions/D95.md), fixtures | A texture not supplied is a warning, a black channel and a `pending-assets.json` entry. |
| Audio channels (`music`, `mic`) | Unverified | [D93](decisions/D93.md), [D112](decisions/D112.md), `render.rs` `the_shadertoy_audio_texture_is_512_by_2_and_resampled_from_the_spectrum` | Not compared with Shadertoy on the same track ([Q18](OPEN_QUESTIONS.md)). |
| `cubemap` channels | Supported | [D553](decisions/D553.md), [D185](decisions/D185.md), `tests/render.rs` `a_cube_map_input_shows_each_picture_where_shadertoy_puts_it`, `tests/convert.rs` `a_cube_map_with_its_six_faces_becomes_a_scene_cube_map` | Six faces named and ordered as Shadertoy's `effect.js` does, `vflip` honoured; a missing face reads a black cube and is pending. The sampler's filter is not (always mipmapped). llvmpipe only. |
| `keyboard` channel | Degraded | [D553](decisions/D553.md), `tests/render.rs` `a_keyboard_input_is_256_by_3_with_every_key_up` | Shadertoy's 256x3 layout with every key up, and a warning: no key reaches a scene, by decision. |
| `video`, `webcam` channels | Degraded | [D93](decisions/D93.md) | Read black, with a warning. |
| `volume` channels, `sound` and `cubemap` passes | Unsupported | [D553](decisions/D553.md) | Import errors naming the pass and channel: no 3D textures, no audio synthesis, no cube render target. |
| `srgb` textures | Degraded | [Q16](OPEN_QUESTIONS.md) | Not decoded; the importer warns. |
| `iDate`, `iMouse`, `iChannelTime`, `iSampleRate`, `iFrameRate` | Unverified | [D93](decisions/D93.md), [D112](decisions/D112.md), `render.rs` `strobe_date_is_the_date_the_host_set` | Semantics not checked against Shadertoy's page ([Q15](OPEN_QUESTIONS.md)). |
| A resize | Supported | [D91](decisions/D91.md), [D495](decisions/D495.md), `tests/render.rs` `a_resize_does_not_make_itime_jump_at_a_non_default_speed` | `iTime` runs on through a resize at any Speed; `iFrame` restarts when the resize wipes a buffer that reads it, so a buffer initialising on `iFrame == 0` starts again. |
| Import by URL or id (REST API, needs a key) | Unverified | [Q19](OPEN_QUESTIONS.md), [T480 Task 11](hardware/T480.md), `crates/strobe-shadertoy/tests/acquire.rs` (mock) | The live API answered the T480 with a Cloudflare 403. Tested only against a loopback mock. |
| "Send to Strobe" browser userscript | Unverified | [D115](decisions/D115.md), [I-01](IMPLEMENTATION_BACKLOG.md) | Its route to the page's JSON is a guess and has never run on shadertoy.com. |
| Local import listener (`--listen`) | Supported | [D114](decisions/D114.md), `crates/strobe-shadertoy/tests/listener.rs`, [T480 Task 11](hardware/T480.md), [T480 Task 21](hardware/T480.md) | Loopback only, token checked: 200, 409 on a re-import, 401 on a bad token. |
| A Shadertoy URL typed on a playlist cue | Unverified | [D229](decisions/D229.md) | Uses the same import path as above, so it has the same live-site gap. |
| Author, source and licence kept in `SOURCE` | Supported | [D95](decisions/D95.md), `crates/strobe-shadertoy/tests/golden` | Records CC BY-NC-SA 3.0 unless the author says otherwise. Not a legal determination; licences are the user's responsibility ([Q15](OPEN_QUESTIONS.md)). |
| Generated controls for an import | Supported | [D123](decisions/D123.md)-[D126](decisions/D126.md), [D202](decisions/D202.md), [D219](decisions/D219.md), `crates/strobe-autocontrol/tests/engine.rs`, [T480 Task 23](hardware/T480.md) | Heuristics have only seen Strobe's own scenes and fixtures. Not yet clicked in the app by a person on Windows. |

## 5. Graphics backends and platforms

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Native OpenGL 3.3 core, Intel UHD 620 (Windows 11) | Supported | [T480](hardware/T480.md) (app on the real GPU, Tasks 14, 17, 30) | Driver 30.0.101.1122. Heavy scenes need adaptive resolution at 1440p. |
| Native OpenGL 3.3 core, NVIDIA GTX 1080 Ti (Windows 10) | Supported | [TORNADO T1](hardware/TORNADO.md), [TORNADO T18](hardware/TORNADO.md), [TORNADO T19](hardware/TORNADO.md) | Driver 582.78. |
| Native OpenGL 3.3 core, AMD Radeon RX 580 (Windows 10) | Supported | [AMD Task A1](hardware/AMD.md) | Driver 31.0.21925.1001. All 12 scenes, both dialects, 0 KHR_debug messages. No robust context: see context loss. |
| Intel HD 4000 (the stated GL 3.3 floor) | Unverified | [D3](decisions/D3.md) | No run on an HD 4000 is recorded ([P-03](IMPLEMENTATION_BACKLOG.md)). |
| ANGLE, GLES 3.0 on Direct3D 11 (`--gl angle`) | Supported | [T480 Task 30](hardware/T480.md), [T480 Task 21](hardware/T480.md), [TORNADO T1](hardware/TORNADO.md), [TORNADO T18](hardware/TORNADO.md), [AMD Task A1](hardware/AMD.md) | DLLs from a pinned Electron release ([D12](decisions/D12.md)). Compiles are slower on heavy scenes (3.2 s against 275 ms native on the T480). |
| ANGLE on Vulkan or desktop GL (`--gl angle-vulkan`, `angle-gl`) | Supported, one machine | [D485](decisions/D485.md), [BENCHMARKS.md, 2026-10-26](BENCHMARKS.md) | On TORNADO (GTX 1080 Ti) the 17 library scenes matched ANGLE-D3D11's images within run-to-run frame time. Vulkan needs the `vulkan-1.dll` the release ships. Not yet on the T480, the RX 580 or the owner's scenes. A backend ANGLE cannot start on fails with a message (`angle-metal` on Windows). |
| Context loss: rebuild, then fail over to ANGLE | Unverified | [D66](decisions/D66.md), [T480 Task 8](hardware/T480.md), `STROBE_FAULT` policy tests, [AMD Task A1](hardware/AMD.md) | Injected losses recovered on real WGL. No real driver reset has been observed. AMD's native WGL refuses a robust context, so a reset there is not reported; ANGLE on AMD is robust. |
| Adaptive render scale | Supported | [D64](decisions/D64.md), [T480](hardware/T480.md) | Thresholds are validated on the T480 only. |
| Headless llvmpipe (CI, Linux) | Supported | `crates/strobe-gfx/tests/render.rs`, CI `linux` job | For tests and frames. Timings are CPU rasterisation, not GPU numbers. |
| Windows release zip | Supported | [D270](decisions/D270.md), [D544](decisions/D544.md), [R14](OPEN_QUESTIONS.md), [the v0.1.0 release](https://github.com/lnorton89/strobe/releases/tag/v0.1.0), [the v0.2.0 release](https://github.com/lnorton89/strobe/releases/tag/v0.2.0), [T480 Task 21](hardware/T480.md), [T480 Task 50](hardware/T480.md) | `release.yml` built and published v0.1.0 and v0.2.0 from their tags (zip, symbols, SBOM, checksums, attestations). The downloaded v0.1.0 zip was run on the venue mini-PC from its own folder against a fresh data directory ([RELEASE_PLAN.md](RELEASE_PLAN.md) step 4, [MINIPC](hardware/MINIPC.md)), and the downloaded v0.2.0 zip on the T480, where a crash drill symbolicated with the release's own `strobe.pdb` ([R-03](IMPLEMENTATION_BACKLOG.md), [R-04](IMPLEMENTATION_BACKLOG.md)). Since D544 the C runtime is linked statically, so no Visual C++ redistributable is needed, and `package-check` fails a package whose program imports a DLL that is neither Windows' nor in the zip. |
| Crash reports and symbolication (Windows) | Supported | [T480 Task 22](hardware/T480.md), [R5](OPEN_QUESTIONS.md) | Minidump and report written by the packaged build; `cargo xtask symbolicate` named the frames. A bare `abort()` is not caught. |
| Linux desktop app, X11 and Wayland, on Mesa's software GL | Supported | CI `linux` job (Xvfb smoke test); [D295](decisions/D295.md): clean Ubuntu 24.04, the README's three build packages, `cargo build --release -p strobe-app`, `strobe --diagnose`, `strobe-bench` headless, a control and an output screenshot on X11 (Xvfb) and on Wayland (headless Weston) | llvmpipe only, no real Linux GPU. The Wayland and X11 libraries are `dlopen`ed, so a build on a machine without them succeeds and the app fails at launch; README's Linux section lists them. |
| Linux desktop app on a real GPU, with real audio and MIDI | Unverified | [STATUS_AUDIT P1: Linux](STATUS_AUDIT.md) | No Intel, AMD or NVIDIA Linux driver, no ALSA, PipeWire or PulseAudio input, no MIDI device and no GNOME or KDE session has run it. No release artifact. |
| Linux via Nix (`flake.nix`: dev shell, package, `nix run`) | Supported | [D295](decisions/D295.md) | x86_64-linux, Nix 2.34, nixpkgs-unstable pinned in `flake.lock`: the dev shell built the app and bench, `nix build` and `nix run` built and ran the package, which drew a window on X11 (Xvfb) and Wayland (Weston) and rendered headless, all on nixpkgs' Mesa llvmpipe. aarch64-linux evaluates, not built. Not run on NixOS or with a real GPU. The GPU driver is the system's (`hardware.graphics.enable` on NixOS); on another distro use `nix develop .#software`. |
| Linux under WSL2 with WSLg (Mesa d3d12, Wayland or X11) | Verified (2026-10-15) | [hardware/AMD.md](hardware/AMD.md) A14 | Debian 13 on the RX 580: builds, bench and X11 and Wayland screenshots pass; `GALLIUM_DRIVER=d3d12` for the GPU; audio needs ALSA's default pointed at PulseAudio. For development, not for a show: use the Windows release. |
| macOS | Unsupported | [D55](decisions/D55.md) | No CGL context provider; CI builds the portable crates only. |
| A renderer of Strobe's own on Vulkan, wgpu, Direct3D 12 or Metal | Unsupported | [ROADMAP item 10](ROADMAP.md), [GFX_API_RESEARCH.md](GFX_API_RESEARCH.md) | Research only. ANGLE's Vulkan backend, above, is the way onto Vulkan today. |

## 6. Audio

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Live capture from the default recording device | Supported | [D61](decisions/D61.md), [T480](hardware/T480.md) (Realtek microphone at 48 kHz), [TORNADO T19](hardware/TORNADO.md), [D266](decisions/D266.md) | Windows microphone privacy settings must allow desktop apps. A device that reports buffer glitches keeps its stream (D266). On the AMD desktop it reopened 70 times a second before that fix; after it, 188 s ran with no reopen and no glitch (Task A2). |
| Capturing what the computer plays (loopback, Stereo Mix) | Unverified | [USER_GUIDE](USER_GUIDE.md) | Output devices are listed as loopback; no run is recorded. |
| Synthetic 120 BPM fallback | Supported | [D61](decisions/D61.md), [STATUS_AUDIT P1](STATUS_AUDIT.md) (missing and synthetic audio states) | Used when no device opens, and by the bench and tests. |
| Device removed and reconnected | Unverified | [R-01](IMPLEMENTATION_BACKLOG.md), [T480 Task 20](hardware/T480.md) | Capture retries every 2 s by design. Never run on hardware: it waits for the owner's approval. |
| Tempo estimate and beat grid | Unverified | [D20](decisions/D20.md), [D21](decisions/D21.md), `strobe-audio` `tempo` tests | Synthetic signals only. |
| Tap tempo and nudge | Supported | `tempo.rs` `tap_tempo_averages_the_intervals`, `nudge_overrides_and_clear_restores` | Not hardware-dependent. |
| Downbeat estimate | Unverified | [D233](decisions/D233.md), `strobe-audio` `downbeat` tests | Synthetic kicks only. Unsure means bars count from the grid's origin. |
| Breakdowns and drops | Unverified | [D226](decisions/D226.md), `strobe-audio` `events` tests | Synthetic structure and a kick/hiss/kick signal. Thresholds are first estimates. |
| Song changes | Unverified | [D237](decisions/D237.md), `strobe-audio` `song` tests | A gap or a new held tempo. A tempo-matched DJ mix with no gap is not heard. |

## 7. Playlists

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| TOML format, sections, inheritance, generator | Supported | [D73](decisions/D73.md)-[D77](decisions/D77.md), `crates/strobe-playlist/tests/example.rs`, unit tests | |
| Precompile at load, zero shader compiles during playback | Supported | [D151](decisions/D151.md), [D152](decisions/D152.md), `crates/strobe-app/tests/show.rs`, [T480 Task 14](hardware/T480.md), [TORNADO T19](hardware/TORNADO.md) | A scene that fails to compile is skipped and shown. |
| Transitions: cut, crossfade, fade through black, custom GLSL | Supported | [D154](decisions/D154.md), `crates/strobe-gfx/tests/compositor.rs`, [T480 Task 14](hardware/T480.md), [TORNADO T19](hardware/TORNADO.md) | Two scenes render during a transition; cost on an HD 4000 unmeasured. |
| 30-minute show with scene changes | Supported | [TORNADO T19](hardware/TORNADO.md), [T480 Task 17](hardware/T480.md) | Flat memory and handles. A 4-hour show has run only as the pure clock ([P-03](IMPLEMENTATION_BACKLOG.md)). |
| Beat-locked cue ends | Unverified | [D75](decisions/D75.md), clock tests | Depends on the tempo estimate, which has not been checked on real music. |
| Audio-event advance (`audio-drop`, `audio-breakdown`, `audio-song`) | Unverified | [D226](decisions/D226.md), [D237](decisions/D237.md), clock tests | Needs live audio; not heard on real music. Ignored during a transition. |
| Resume after a close, and `--no-resume` | Supported | [D155](decisions/D155.md), [STATUS_AUDIT P2](STATUS_AUDIT.md) (Xvfb logs) | |
| Wall-clock anchor | Supported | [D155](decisions/D155.md), [STATUS_AUDIT P2](STATUS_AUDIT.md) | |
| File watcher and lenient reload | Supported | [D158](decisions/D158.md), [STATUS_AUDIT P2](STATUS_AUDIT.md) | A broken file keeps the playing playlist and names the line. |
| Cue editing in LIST (one or several cues, drag in, disable) | Unverified | [D213](decisions/D213.md), [D224](decisions/D224.md), [D230](decisions/D230.md), `show::edit` tests | Not yet used by a person or with a real pointer drag. |
| Save that keeps the file's comments | Supported | [D231](decisions/D231.md), `merge` and `file` tests | Falls back to canonical text when a merge would not read back exactly. |
| A cue's `preset` and `media` | Supported | [D176](decisions/D176.md), [D217](decisions/D217.md), `tests/show.rs`, [T480 Task 26](hardware/T480.md) | Precedence for media: cue, then operator, then preset, then scene. |

## 8. Show control

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Built-in OSC input (`/strobe/control`, `/meta`, `/cmd`, `/cue`, `/fav`, `/preset`, `/macro`) | Supported | [D172](decisions/D172.md), [D173](decisions/D173.md), `crates/strobe-io/src/osc.rs` tests, [T480 Task 25](hardware/T480.md) | Off until turned on; loopback only unless chosen. |
| Synesthesia OSC input aliases | Supported | [D234](decisions/D234.md), `osc.rs` `synesthesias_addresses_are_aliases_and_the_rest_is_ignored`, [T480 Task 31](hardware/T480.md) | `/controls/scene\|meta/<name>`, `/controls/global/<type>/<n>`, two or three values to a pad or colour, `/favslots/<n>`, `/playlist/next\|previous\|toggleplay`. Ignored: `/raw` and the other suffixes, scene-specific addresses, string arguments. |
| OSC from another machine on the LAN | Unverified | [STATUS_AUDIT P2](STATUS_AUDIT.md) | `--osc-all-interfaces` was bound, never reached from a LAN. |
| OSC feedback output | Supported | [D209](decisions/D209.md), `feedback` tests, [T480 Task 25](hardware/T480.md) | Changes only, on `/strobe/...`. Synesthesia's output layout is not produced. Not tried with a real TouchOSC layout. |
| MIDI input and mapping | Unverified | [D170](decisions/D170.md), [D171](decisions/D171.md), parser tests from recorded bytes | No MIDI controller has been used ([P-04](IMPLEMENTATION_BACKLOG.md)). |
| MIDI feedback output (faders, LED rings, pad lights) | Unverified | [D257](decisions/D257.md), `midi_feedback` tests, `mapping.rs` `unshape_finds_the_travel_that_shape_turned_into_a_value` | Off by default. CC, pitch bend (14-bit) and note lights, for mappings on a named device. No controller has received it yet ([P-04](IMPLEMENTATION_BACKLOG.md)). |
| Importing Synesthesia's MIDI mappings | Unsupported | [D234](decisions/D234.md), [Q23](OPEN_QUESTIONS.md) | They are internal identifiers with no export. |
| Learn | Degraded | [D174](decisions/D174.md), [STATUS_AUDIT P2](STATUS_AUDIT.md) (Xvfb click, OSC) | Verified with OSC under Xvfb only; MIDI Learn and Windows are unverified. |
| Macros: reach, colour, pinning, glide, key | Supported | [D210](decisions/D210.md), [D227](decisions/D227.md), [D228](decisions/D228.md), `strobe-io` `macros` and engine tests, [T480 Task 29](hardware/T480.md) | The key and the reach were checked on the T480 with posted keys. |
| Presets: save, recall, rename, delete, media channel | Supported | [D176](decisions/D176.md), [D214](decisions/D214.md), [D223](decisions/D223.md), `strobe-scene` `preset` tests, `tests/show.rs` | |
| Fav slots | Supported | [D176](decisions/D176.md), `preset` tests, [T480 Task 31](hardware/T480.md) | Recall of a filled slot over OSC on hardware is not recorded; an empty slot says so. |
| Keyboard shortcuts and the shortcut editor | Supported | [D178](decisions/D178.md), [D208](decisions/D208.md), `commands.rs` and `ui::shell_tests`, [T480](hardware/T480.md) (Right, B) | Ctrl chords were not testable on the T480 with posted keys. |
| Blackout on, off and toggle | Supported | [D181](decisions/D181.md), [D201](decisions/D201.md), Xvfb OSC run in [STATUS_AUDIT P2](STATUS_AUDIT.md), [T480](hardware/T480.md) (B key) | Blackout gates the shown output only; `master` fades the picture. |
| Command palette | Supported | [D200](decisions/D200.md), [D262](decisions/D262.md), `command_palette` and `ui::shell_tests` | Commands, panes, layouts, library scenes (Load, Preview) and the playing scene's presets. Not yet used by a person on Windows. |

### Synesthesia's preset, meta and control behaviours (C-06)

Each row is a behaviour Synesthesia documents (its manual's OSC, controls
and presets pages, read on TORNADO in T20), classified against Strobe, with
the test that holds it.

| Behaviour | Status | Evidence | Notes |
|---|---|---|---|
| Meta values stay when the scene changes | Supported | [D207](decisions/D207.md), `meta.rs` `a_new_bank_carries_what_it_has_not_set_itself` | A cue's or preset's own meta values win over the carried ones. |
| A playlist cue's preset and meta apply when the cue starts | Supported | `tests/show.rs` `a_cues_preset_is_applied_when_the_cue_starts_and_an_override_beats_it`, `the_operators_output_settings_carry_across_cues_under_a_cues_own_meta` | |
| A preset holds the scene's, the meta and the media values | Supported | [D176](decisions/D176.md), [D223](decisions/D223.md), `preset.rs` tests | `master` is never saved: it is the operator's fader. |
| Recalling only one part of a preset (`/presets/scene`, `/meta`, `/media`) | Degraded | [D176](decisions/D176.md), [D247](decisions/D247.md) | Which parts a preset holds is chosen when it is saved, not when it is recalled. Synesthesia's `/presets/...` addresses take a string, which the OSC decoder drops. |
| Preset names | Degraded | `preset.rs` `find_name` | Synesthesia's are case-sensitive. Strobe ignores case and reads `_` as a space, because names also arrive in OSC addresses. |
| Fav slots recall a scene and a preset | Supported | `preset.rs` `fav_slots_are_eight_and_a_short_file_is_padded`, [T480 Task 31](hardware/T480.md) | Eight slots. |
| One control to its default or a random value (`.../default`, `.../random`) | Supported | [D247](decisions/D247.md), `target.rs` `default_and_random_cover_a_control_a_group_or_a_bank`, `osc.rs` `synesthesias_addresses_are_aliases_and_the_rest_is_ignored`, `engine.rs` `synesthesia_addresses_reach_the_same_controls` | Also from the Inspector's buttons. A bang is never randomized. |
| A group or a whole bank to default or random (`/controls/groups/...`, `/controls/banks/...`) | Supported | Same tests | Scene and meta banks. A group is matched by name as controls are. |
| A script's writes to meta controls (`setControl("meta/...")`) | Supported | [D160](decisions/D160.md), `tests/script_meta.rs` `a_script_write_to_a_meta_control_reaches_the_sessions_meta_bank` | |
| A script's randomize and default on a control, a group and meta | Supported | `strobe-script` conformance `randomize_and_default_control_and_group`, `meta_default_randomize_and_groups_use_the_same_operations`, `randomize_is_reproducible_from_the_seed` | |
| Audio reactivity and audio speed as meta controls | Supported | `audio.rs` `meta_gains_multiply_the_panels_and_stay_in_range` | They multiply the Audio panel's own gains and carry across scenes like any meta value. |
| Locking a control, a group or a bank (`/lock`) | Unsupported | [D247](decisions/D247.md) | Nothing in Strobe protects a value from a preset, a cue or a randomize yet. |
| Bank undo (`/controls/banks/.../undo`) | Unsupported | [D247](decisions/D247.md) | |
| Back to the preset's value (`.../preset`) | Unsupported | [D247](decisions/D247.md) | |
| Saving a new preset over OSC (`/presets/new`) | Unsupported | [D247](decisions/D247.md) | Saving is the Presets panel's, with a name a person types. |
| Alpha Channel meta | Unsupported | [D248](decisions/D248.md) | Waits for an output that carries alpha (NDI or Spout, deferred). |

## 9. Media

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| A still picture through `syn_Media` | Supported | [D186](decisions/D186.md), `crates/strobe-gfx/tests/assets.rs` `a_still_image_is_read_through_syn_media_upright`, [T480 Task 26](hardware/T480.md) | PNG or JPEG. Chosen in the Inspector, per cue, by `--media` or a drop. |
| An image sequence (a folder) | Supported | `assets.rs` `an_image_sequence_plays_at_its_rate_and_loops`, [T480 Task 27](hardware/T480.md), [T480 Task 28](hardware/T480.md) | Decoded off the frame thread. |
| A long sequence over 192 MiB | Supported | [D225](decisions/D225.md), [T480 Task 28](hardware/T480.md) | Shrunk as a whole to fit, not cut; below 256 px it is cut, with a message. |
| SSF `MEDIA` slot (`[{FILTER, WRAP}]`) | Supported | [D193](decisions/D193.md), `scene.rs` `real_synesthesia_shapes_load`, [TORNADO T21](hardware/TORNADO.md) | Strobe also accepts its own path form, a divergence from SSF. |
| The slot's `WRAP` and `FILTER` applied to chosen media | Unverified | [D238](decisions/D238.md), `scene.rs` `chosen_media_is_sampled_as_the_media_slot_says` | The settings are parsed and tested; no render test checks the sampler. |
| `syn_MediaType` 0 (none) and 1 (image) | Supported | `assets.rs` `no_media_is_black_and_type_zero`, [TORNADO T21](hardware/TORNADO.md) | Matches SSF's documented values. |
| A webcam (`syn_MediaType` 3, Windows) | Unverified | [D528](decisions/D528.md), [D529](decisions/D529.md), [M-04](IMPLEMENTATION_BACKLOG.md), `strobe-capture` supervisor and conversion tests (a fake camera), `crates/strobe-gfx/tests/live.rs` `a_camera_shows_upright_as_type_3_in_one_texture`, `a_failing_camera_reads_black_says_so_once_and_comes_back` | Chosen as `camera:<name>` in the Inspector's Media row, by `--media`, a cue or a preset; `--list-cameras` prints the names. No real camera has been opened yet: the camera checklist in M-04 is open. No backend on Linux or macOS. Shows only in scenes that read `syn_Media` (the Media Overlay waits on [C-01](IMPLEMENTATION_BACKLOG.md)). |
| Video files (`syn_MediaType` 2) | Unsupported | [M-04](IMPLEMENTATION_BACKLOG.md) | Not built. A video reads as no media. |
| NDI and Spout, in or out | Unsupported | [M-06](IMPLEMENTATION_BACKLOG.md), [TORNADO T21](hardware/TORNADO.md) | NDI and Spout have no documented `syn_MediaType` value. |

## 10. LAN remote

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Protocol version 1 (`strobe-remote`) | Supported | [D236](decisions/D236.md), `crates/strobe-remote/src/lib.rs` golden tests | Library only. Bounded, strict decoding; another version gets `version_mismatch`. |
| App-thread command and state bridge | Supported | [D232](decisions/D232.md), [D254](decisions/D254.md), `crates/strobe-remote/src/bridge.rs` and `strobe_app::lan` tests | Drained once a frame with `try` only; commands checked against the app's state, then run as their own `Action`. Frame cost not yet measured on the T480. |
| Network listener (opt-in, LAN only) | Unverified | [D250](decisions/D250.md), [D252](decisions/D252.md), `crates/strobe-remote/tests/server.rs` | Built and tested over loopback sockets in CI, including Host/Origin, size, flood, slow-client and idle cases. Never reached from a phone over Wi-Fi; the Windows firewall prompt not seen. Plain HTTP: trusted networks only. |
| Pairing, tokens, rotation, revocation | Unverified | [D251](decisions/D251.md), [D468](decisions/D468.md), [D469](decisions/D469.md), `strobe_remote::{pairing, clients}` and `tests/server.rs` | An approval at the host with no code: a phone asks under a name, a desktop as itself, bounded at four waiting, one per address and 30 s after a Deny. Token digests on disk; secrets never logged (a test captures the log). No real phone has paired. |
| Phone or tablet controller page | Unverified | [D253](decisions/D253.md), [D469](decisions/D469.md), `strobe_remote::server::page` tests | One embedded file, no CDN. Rendered in a desktop browser against a local host, where it asked for a name and was approved (D469); no phone or tablet browser yet. No macros (protocol v2), no live preview. |
| Finding Strobes on the LAN (UDP 47813) | Unverified | [D468](decisions/D468.md), `strobe_remote::discovery` tests | Two Strobes on one machine (TORNADO) found each other and one approved the other; not yet across two machines, or through the Windows firewall's prompt. |
| HTTPS through the operator's own reverse proxy | Unverified | [D258](decisions/D258.md), `tests/server.rs` `an_https_proxy_on_this_computer_is_accepted_by_its_name_only` | `--remote-proxy NAME`. The proxy and its certificate are the operator's. Not yet tried on a phone. |
| Installable PWA over HTTPS | Unverified | [D517](decisions/D517.md), [L-07](IMPLEMENTATION_BACKLOG.md), [D258](decisions/D258.md) | The remote and projector pages serve a manifest, icons and a network-first service worker. The worker runs only in a secure context (the operator's HTTPS proxy), so over plain HTTP a page installs as a home-screen shortcut that needs Strobe reachable. Not yet tried on a real phone. |

## Keeping this file true

- Change a row only with evidence: a test, a decision, or a hardware
  record with its machine, backend and commit.
- When STATUS_AUDIT.md gains or loses evidence, update the row here in
  the same change.
- "Works under llvmpipe" does not prove Windows driver behaviour, and a
  synthetic signal does not prove behaviour on real music.
