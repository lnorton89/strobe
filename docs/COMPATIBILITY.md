# Compatibility

This is the supported, degraded and unsupported matrix for Strobe, for
people running it and for contributors. Every status points at its
evidence: a decision in [DECISIONS.md](DECISIONS.md), a test, a hardware
record ([T480](hardware/T480.md), [TORNADO](hardware/TORNADO.md),
[AMD desktop](hardware/AMD.md)) or an
item in [STATUS_AUDIT.md](STATUS_AUDIT.md). STATUS_AUDIT.md is the
evidence log, and it wins when this file disagrees with it. Written
2026-10-13.

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
(the old-laptop reference). **TORNADO** is a GTX 1080 Ti on Windows 10.
**AMD desktop** (DESKTOP-30KSOCM) is a Radeon RX 580 on Windows 10.
**llvmpipe** is Mesa's software rasteriser in CI and the cloud
container, headless or under Xvfb.

## 1. SSF scene format

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Scene folder (`main.glsl`, `scene.json`, optional `script.js`) | Supported | [TORNADO T18](hardware/TORNADO.md), [T480 Task 30](hardware/T480.md) | Loaded through the app's own path by `cargo xtask corpus` ([D195](DECISIONS.md)). |
| CRLF and `\r\r\n` line endings | Supported | [D193](DECISIONS.md), [D201](DECISIONS.md), `load.rs` `windows_line_endings_are_normalised_at_load` | Every line-ending form becomes one `\n`, so error lines match editors. |
| `CONTROLS`: `slider`, `knob`, `toggle`, `bang`, `xy`, `color`, `dropdown` | Supported | `crates/strobe-scene/src/control/kind.rs` `every_documented_type_parses`, `runtime.rs` tests, `render.rs` `a_control_value_reaches_the_shader` | `color` is `vec3` ([R2](OPEN_QUESTIONS.md)). MIN above MAX is kept ([R3](OPEN_QUESTIONS.md)). |
| Control modifiers `smooth`, `speed`, `counter` | Supported | `runtime.rs` `smoothing_takes_the_same_wall_time_at_any_frame_rate`, `speed_integrates_the_target_per_second`, `counter_presses_between_frames_are_not_lost` | Smoothing is frame-rate independent ([D9](DECISIONS.md)). |
| `bang smooth` envelope | Degraded | `runtime.rs` `a_smooth_bang_rises_then_falls` | The envelope shape is a guess ([Q4](OPEN_QUESTIONS.md)). |
| `traveler` modifier | Degraded | [D197](DECISIONS.md), `runtime.rs` `a_traveler_accelerates_toward_its_speed_and_integrates_it` | Undocumented by SSF; Strobe's reading is an inference from 9 shipped scenes. |
| A control with no `TYPE` | Supported | [D193](DECISIONS.md), `scene.rs` `real_synesthesia_shapes_load` | Two default components make an `xy`, anything else a `slider`. |
| `PASSES`: `TARGET`, multipass, ping-pong | Supported | `render.rs` `a_buffer_pass_feeds_the_final_pass`, `a_double_buffered_pass_reads_its_previous_frame` | |
| Pass sizes: literal `WIDTH`/`HEIGHT` | Supported | [D194](DECISIONS.md), [R7](OPEN_QUESTIONS.md), `scene.rs` `literal_pass_sizes_are_absolute_and_expressions_follow_the_output` | Literal sizes are absolute pixels, as in Synesthesia. `$WIDTH` expressions follow the output ([D11](DECISIONS.md)). |
| `FIXED_SIZE` on a pass | Supported | [D191](DECISIONS.md), `render.rs` `a_fixed_size_pass_keeps_its_grid_when_the_output_resizes` | A Strobe extension, not SSF. |
| `FLOAT` passes | Degraded | [D13](DECISIONS.md), `render.rs` `a_full_precision_pass_keeps_counting_where_half_floats_stall` | SSF documents 32-bit; Strobe defaults to RGBA16F for speed on integrated GPUs. 32-bit is available (`FloatPrecision::Full`), not per scene. |
| A pass that samples its own target | Supported | [D203](DECISIONS.md), [R9](OPEN_QUESTIONS.md), [TORNADO T8](hardware/TORNADO.md) | Double-buffered automatically, so native and ANGLE agree. |
| Implicit final pass | Supported | [D8](DECISIONS.md), [R1](OPEN_QUESTIONS.md), `render.rs` `a_buffer_pass_feeds_the_final_pass` | `renderMain` runs once more with `PASSINDEX == PASSES.len()`. |
| `PASSINDEX` compared as int or float | Supported | [D7](DECISIONS.md), [D163](DECISIONS.md), [R4](OPEN_QUESTIONS.md), `render.rs` `passindex_compared_with_a_float_literal_compiles_in_both_dialects`, `int_passindex_comparisons_still_compile_in_both_dialects` | A compile-time constant per pass program. |
| Desktop-only GLSL idioms on ANGLE (implicit int to float, `#if` on a non-macro, initialised uniforms, function-like macros mixing int and float, a sampler chosen with `?:`) | Supported | [D199](DECISIONS.md), [D261](DECISIONS.md), `render.rs` `implicit_int_to_float_and_non_macro_if_compile_in_both_dialects`, `function_like_macros_mixing_int_and_float_compile_in_both_dialects`, `a_sampler_chosen_with_a_ternary_compiles_in_both_dialects`, [TORNADO T18](hardware/TORNADO.md) | A sampler `?:` is rewritten only as the one such argument of a function call (`texture(c ? a : b, uv)`); anywhere else the ES program fails with Strobe's own message at the user's line (`a_sampler_choice_that_cannot_be_rewritten_fails_with_strobes_message_on_es`). The macro and `?:` rewrites are checked on llvmpipe with original fixtures, not yet on ANGLE hardware or the owner's corpus. |
| `#include` | Degraded | [D198](DECISIONS.md), [D271](DECISIONS.md), `include.rs` tests | Strobe ships neither lygia nor hg_sdf. Works where Synesthesia is installed (found as the library finds it, `STROBE_SYNESTHESIA_DIR` included) or `STROBE_GLSL_PATH` points at a copy. |
| `IMAGES` | Supported | `render.rs` `image_wrap_filter_and_vflip_options_are_honoured`, `assets.rs` `a_missing_image_reports_a_problem_and_reads_black`, [D201](DECISIONS.md) | A missing picture is reported and reads black; the scene plays. Decoded by content, not extension ([D196](DECISIONS.md)). |
| `CUBEMAPS` | Degraded | [D185](DECISIONS.md), `assets.rs` `six_face_files_answer_each_direction_with_their_colour`, `a_cross_picture_gives_the_same_six_faces` | Face orientation is unverified against Synesthesia ([Q22](OPEN_QUESTIONS.md)); none of the 65 real scenes uses a cube map. llvmpipe only. |
| `MEDIA` | Supported | See section 9 | |
| `HARD_TRANSITIONS`, `SMOOTH_TRANSITIONS` | Degraded | [ROADMAP phase 7](ROADMAP.md), [D196](DECISIONS.md) | Parsed (`null` accepted) and declared; the uniforms keep their first value. Song changes ([D237](DECISIONS.md)) drive playlists only, not these uniforms. |
| `TAGS` | Supported | [D205](DECISIONS.md), [D264](DECISIONS.md), `strobe_scene::tags` and `ui::tags` tests | Read leniently, normalised, written in place, for any library scene. |
| `IMAGE_PATH` | Supported | [D212](DECISIONS.md), [D222](DECISIONS.md), `ui::thumbs` and `snapshot` tests | Shown as the browser thumbnail; can be taken from the live frame. |
| Other keys (`IS_META`, `GPU`, `UNIFORMS`, `LABEL`, `X_STROBE`) | Degraded | [Q6](OPEN_QUESTIONS.md), [R11](OPEN_QUESTIONS.md) | Read and ignored. A root `UNIFORMS` is not Synesthesia's; its own scenes use the key only inside `SMOOTH_TRANSITIONS`. |
| SSF function library (documented names) | Supported | `strobe-ssf-lib` `every_documented_function_is_defined` (63 names), `render.rs` `the_ssf_library_compiles_in_both_dialects`, [TORNADO T18](hardware/TORNADO.md) | Re-implemented from the documented behaviour. Numbers are not bit-identical to Synesthesia's. |
| Undocumented helpers used by real scenes | Degraded | [D197](DECISIONS.md), [D199](DECISIONS.md) | The image family, older names (`_loadUserImage` and family as aliases of `syn_Media`), `_ratioFix` ([Q27](OPEN_QUESTIONS.md)), `_hueSaturationContrastLEGACY` ([Q26](OPEN_QUESTIONS.md)) and `_Media_Contrast` ([D268](DECISIONS.md)) are inferred. |
| Scalar audio uniforms (`syn_Level`, `syn_*Hits`, `syn_*Time`, `syn_*Presence`, beat values) | Supported | `strobe_scene::audio::SCALARS`, live capture on [T480](hardware/T480.md) (script scene, 2026-10-09) and [TORNADO T19](hardware/TORNADO.md) | Strobe's own analysis; values are not compared numerically with Synesthesia's. |
| Undocumented audio names (`syn_InputLevel`, `syn_BassExpoCurve`) | Degraded | [D197](DECISIONS.md) | Stand-ins until their meaning is known. |
| `syn_Spectrum`, `syn_LevelTrail` textures | Unverified | `render.rs` `uploaded_audio_texels_reach_the_spectrum_and_trail_samplers` | `sampler1D` on desktop is confirmed ([R10](OPEN_QUESTIONS.md)). The spectrum is 1024 texels, as Synesthesia's ([D269](DECISIONS.md)); frequency axes and trail direction are assumptions ([Q10](OPEN_QUESTIONS.md)). |
| `syn_FinalPass` | Supported | `render.rs` `syn_final_pass_is_last_frames_output` | llvmpipe, both dialects. |
| `textureFinalPass` (script) | Unverified | [D162](DECISIONS.md), `render.rs` readback tests, `script_uniforms.rs` `texture_final_pass_reads_the_picture_a_few_frames_late` | 64x36, two frames late, never a stall. Latency and cost unmeasured on the T480 and ANGLE/D3D11: the test skips on Windows ([Q14](OPEN_QUESTIONS.md), [C-04](IMPLEMENTATION_BACKLOG.md)). |
| Meta controls (master, invert, brightness, gamma, contrast, hue, saturation, mirrors, Limit Colors) | Degraded | [D180](DECISIONS.md), [D207](DECISIONS.md), `crates/strobe-gfx/tests/post.rs`, [T480 Task 15](hardware/T480.md), [T480 Task 25](hardware/T480.md) | Order, ranges and mirror conventions are Strobe's guesses ([Q21](OPEN_QUESTIONS.md)). About 1.1 ms GPU at 1440p on the T480; HD 4000 unmeasured. |
| Meta `Alpha Channel` | Unsupported | [D207](DECISIONS.md) | Waits for an output that carries alpha. |
| `_mouse`, `_muv`, `_click` | Unverified | [Q2](OPEN_QUESTIONS.md), [R12](OPEN_QUESTIONS.md) | `_click` is the buttons held, confirmed by a real script; what the mouse values hold outside the preview is unverified. |

## 2. SSF scripts (`script.js`)

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| `setup()` once, `update(dt)` each frame | Supported | `crates/strobe-script/src/conformance.rs` `setup_runs_once_and_its_control_writes_apply_on_the_first_frame`, [T480](hardware/T480.md) (`11-script-orbits`), [TORNADO T18](hardware/TORNADO.md) (scripts run) | QuickJS, built on Windows by CI's `windows` job and the T480. |
| A top-level `transition` function | Supported | [TORNADO T23](hardware/TORNADO.md), [Q23](OPEN_QUESTIONS.md) | Not part of SSF's documented script lifecycle (only `setup` and `update(dt)`). The 6 real scenes that define one leave it empty and never call it, so not calling it changes nothing. |
| `setUniform` uniforms, created at load | Supported | [D62](DECISIONS.md), [D161](DECISIONS.md), `script_uniforms.rs`, `conformance.rs` `uniforms_made_in_setup_are_listed_when_sealed` | The first frame's uniforms are compiled in before the show plays. |
| A uniform first set after load | Degraded | [D161](DECISIONS.md), `script_uniforms.rs` `a_uniform_that_appears_later_is_refused_and_nothing_compiles_after_load` | Refused with one console error, never recompiled. Create every uniform in `setup()`. |
| Legacy `uniforms` object and `inputs` reads | Supported | `conformance.rs` `deprecated_uniforms_object_is_scanned_after_update`, `deprecated_inputs_reads_the_same_values_and_is_read_only` | Both appear in real scenes ([Q23](OPEN_QUESTIONS.md)). |
| `setControl`, `setControlNormalized`, randomize and default (control and group) | Supported | `conformance.rs` `set_control_*`, `randomize_and_default_control_and_group` | Names match ignoring case, `_`, `-` and spaces. |
| Event handlers (`onChange`, `onOffToOn`, `onOnToOff`, `whileOn`, `whileOff`) | Supported | `conformance.rs` `every_event_type_fires_on_its_condition_across_frames` | "On" is `>= 0.5`. |
| Writes to `meta/` controls | Supported | [D160](DECISIONS.md), `conformance.rs` `meta_*`, `crates/strobe-app/tests/script_meta.rs`, `docs/images/post/` (Xvfb) | Scripts cannot read meta values ([Q21](OPEN_QUESTIONS.md)). |
| `syn_PrimeBang` | Degraded | [D199](DECISIONS.md), `conformance.rs` `syn_prime_bang_is_defined_and_never_fires` | Defined as 0; what fires it is unknown ([Q25](OPEN_QUESTIONS.md)). |
| Runaway and faulting scripts | Supported | `conformance.rs` `an_endless_loop_in_update_is_interrupted_and_the_frame_returns`, `three_faulted_frames_in_a_row_disable_the_script`, `a_memory_limit_breach_faults_cleanly_and_recovers` | 4 ms a frame, 64 MiB heap; the output keeps running. |
| Script cost in the benchmark | Unsupported | [D239](DECISIONS.md) | `strobe-bench` runs no scripts and says so. Quote the corpus harness's `frame_ms` instead. |

## 3. Real Synesthesia scenes

The owner's scenes are private and never committed. The counts come from
`cargo xtask corpus`, which loads each scene the way the app does,
scripts included, and renders it in both dialects ([D195](DECISIONS.md)).

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| The owner's 65 scenes on NVIDIA native GL 3.3 | Supported | [TORNADO T18](hardware/TORNADO.md) (commit 12a9ddf) | 63 of 65. The 2 failures are the scenes' own bugs (a function that never returns, an output declared twice). |
| The same 65 on ANGLE/D3D11 (NVIDIA) | Supported | [TORNADO T18](hardware/TORNADO.md) | 63 of 65, the same two failures. |
| Native and ANGLE drawing the same picture | Degraded | [TORNADO T8](hardware/TORNADO.md), [TORNADO T9](hardware/TORNADO.md), [TORNADO T18](hardware/TORNADO.md) | Two scenes differ between backends from undefined behaviour in their own code (`pow(0,0)`, `fract(sin)` hashes). Strobe does not fix those. |
| The 65 scenes on Intel (T480) | Unverified | [T480 Task 30](hardware/T480.md) | The T480 has run only Strobe's 12 library scenes through the harness (12 of 12 in both dialects). |
| The 90 default scenes from Synesthesia 1.25.4, llvmpipe | Supported | [D268](DECISIONS.md) | 90 of 90 in both dialects, scripts on, at commit 776ddf3 plus D268. One condition: `STROBE_GLSL_PATH` points at Synesthesia's `lib` folder, which Strobe also finds when Synesthesia is installed (D198), because SkaiWalkerOG includes `hg_sdf.glsl`. |
| The same 90 on AMD RX 580, native GL and ANGLE | Supported | [AMD Tasks A4 and A5](hardware/AMD.md) | 90 of 90 in both dialects at e47c44f (A5), with no notes. Before D268 it was 88 of 90 (A4). All 48 scripted scenes ran. |
| The 90 drawing the same picture on native and ANGLE (AMD) | Degraded | [AMD Task A4](hardware/AMD.md) | Of the 47 scenes the bench can render, 61 frames match or nearly match. Three differ: `PLUS` (`pow` of a negative base, undefined in GLSL; llvmpipe draws it black), `canvas` (its feedback background) and `turbo_encabulator_fluid` (banding on ANGLE). Both of the last two subtract or seed with `fract(sin(…))` hashes, whose precision differs between drivers. The banding stays with RGBA32F passes, so the storage format is not the cause (A5). Synesthesia's own thumbnails of both scenes show what ANGLE draws, so AMD's native `sin` is the one that differs (A6). Four simulations diverge in detail. |
| A picture matching Synesthesia's own render | Degraded | [AMD Task A6](hardware/AMD.md) | Compared with Synesthesia's shipped thumbnails for 17 scenes, since the free tier has no OSC input to drive it. Same: 3; close (colour, camera or time only): 8; different: 5 (neon_city, a Strobe bug with literal-size passes under a lowered render scale, A9, fixed by D280 and no longer different in framing; beamers, colour and time only, A8; PLUS without demo media; canvas and turbo on AMD native); inconclusive: 2. |
| Scenes with root-sized absolute buffer passes under any render scale | Supported | [D280](DECISIONS.md), `render.rs` `a_final_pass_reading_a_root_sized_literal_pass_covers_it_at_any_render_scale` (both dialects: an original gradient fixture covers the whole frame at 960x540, 480x270 and 240x135); `neon_city` from the private corpus on Xvfb/llvmpipe, native: the same 1920x1080 picture, byte for byte, with `--no-adaptive --render-scale 1.0`, `--render-scale 0.5` and adaptive on | A scene that writes a root `WIDTH` x `HEIGHT` and has a literal buffer pass of exactly that size renders its final pass at that size, so `texelFetch(pass, ivec2(_xy), 0)` reads the whole buffer; the output stage scales it to the window. Eleven of the 90 meet the rule (`SkaiWalkerOG`, `audio_indicators`, `bouncy_hexagons`, `entombed_silicon_dreams`, `fractilian_foundation`, `hextunnel`, `logo_grid_pulses`, `neon_city`, `neuron_proximitors`, `thresholder`, `videofx_1`); three of them read buffers by pixel (`neon_city`, `entombed_silicon_dreams`, `neuron_proximitors`). They no longer get lighter with the render scale. Adaptive resolution leaves them at the ceiling, and for any scene stops lowering when a drop does not cut the GPU time. Not yet seen on a GPU: the RX 580 re-run is open. |
| Running Synesthesia's built-in scenes for a show | Unverified | [Q7](OPEN_QUESTIONS.md) | The EULA question is open. Strobe ships none of them; it reads the person's own install (D271). |
| Synesthesia's installed scenes in the library | Unverified | [D271](DECISIONS.md), `library.rs` `a_fake_install_is_listed_after_the_library_read_only`, `switching_synesthesia_off_lists_only_the_library`; `synesthesia.rs` tests; a fake install over the private corpus on Xvfb (90 listed, SkaiWalkerOG played by `--scene` with its `#include` found) | On by default where installed; Settings > "Show Synesthesia's scenes" or `--no-synesthesia` turns it off. The install is found at `Program Files (x86)\Synesthesia` or `Program Files\Synesthesia`, or `STROBE_SYNESTHESIA_DIR`. `release\` and `default\` are listed after the library folder. Not yet run against a real Windows install. |
| Synesthesia's own scene folders (Settings > Custom Scene Folder) | Unverified | [D271](DECISIONS.md), `synesthesia.rs` `the_persons_scene_folders_come_from_synesthesias_settings`, `broken_or_missing_settings_mean_no_folders_of_the_persons_and_say_why` | Read from `vs.json` (`sceneFolders`, `primarySceneFolder`) and `preferences.json` (`PRIMARY_SCENE_FOLDER`, recursive search to `RECURSIVE_SCENE_SEARCH_DEPTH`, 10 by default) in `%LOCALAPPDATA%\Synesthesia\Data` or `STROBE_SYNESTHESIA_DATA`. Writable, as they are the person's. Fake files only so far. |
| Writing into Synesthesia's install | Unsupported | [D271](DECISIONS.md), `app.rs` `a_write_into_synesthesias_folder_is_refused_and_offers_a_copy`, `copy_to_my_library_makes_a_copy_strobe_may_write` | On purpose: tags, a picture, Editor saves and presets are refused with "Copy to my library", which copies the scene into the library under a free name. |

### Every scene (C-03, TORNADO T24)

One row per scene of the owner's 65, from T22's run at commit 5eaddb0 (scripts on, NVIDIA native GL 3.3 and ANGLE/D3D11), classified in T24. Names only: the scenes themselves are private.

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
| Exported JSON to a scene folder (`strobe-import`, the Import pane, a dropped `.json`) | Supported | [D90](DECISIONS.md)-[D95](DECISIONS.md), `crates/strobe-shadertoy/tests/convert.rs` and `crates/strobe-shadertoy/tests/render.rs`, [T480 Task 11](hardware/T480.md), [TORNADO T1](hardware/TORNADO.md) | Nine original fixtures render on llvmpipe, Intel and NVIDIA, native and ANGLE. |
| Buffers A to D, multipass order and feedback | Supported | [D95](DECISIONS.md), [T480 Task 11](hardware/T480.md) (four-buffer chain) | Every buffer is double-buffered, even one that never reads itself. |
| Texture channels from local files | Supported | [D95](DECISIONS.md), fixtures | A texture not supplied is a warning, a black channel and a `pending-assets.json` entry. |
| Audio channels (`music`, `mic`) | Unverified | [D93](DECISIONS.md), [D112](DECISIONS.md), `render.rs` `the_shadertoy_audio_texture_is_512_by_2_and_resampled_from_the_spectrum` | Not compared with Shadertoy on the same track ([Q18](OPEN_QUESTIONS.md)). |
| `keyboard`, `video`, `webcam` channels | Degraded | [D93](DECISIONS.md) | Read black, with a warning. |
| `cubemap` and `volume` channels, `sound` and `cubemap` passes | Unsupported | [D93](DECISIONS.md) | Import errors naming the pass and channel. |
| `srgb` textures | Degraded | [Q16](OPEN_QUESTIONS.md) | Not decoded; the importer warns. |
| `iDate`, `iMouse`, `iChannelTime`, `iSampleRate`, `iFrameRate` | Unverified | [D93](DECISIONS.md), [D112](DECISIONS.md), `render.rs` `strobe_date_is_the_date_the_host_set` | Semantics not checked against Shadertoy's page ([Q15](OPEN_QUESTIONS.md)). |
| A resize | Degraded | [D91](DECISIONS.md), [ROADMAP](ROADMAP.md) | Restarts `iTime` and `iFrame`, a visible jump at a non-default speed. |
| Import by URL or id (REST API, needs a key) | Unverified | [Q19](OPEN_QUESTIONS.md), [T480 Task 11](hardware/T480.md), `crates/strobe-shadertoy/tests/acquire.rs` (mock) | The live API answered the T480 with a Cloudflare 403. Tested only against a loopback mock. |
| "Send to Strobe" browser userscript | Unverified | [D115](DECISIONS.md), [I-01](IMPLEMENTATION_BACKLOG.md) | Its route to the page's JSON is a guess and has never run on shadertoy.com. |
| Local import listener (`--listen`) | Supported | [D114](DECISIONS.md), `crates/strobe-shadertoy/tests/listener.rs`, [T480 Task 11](hardware/T480.md), [T480 Task 21](hardware/T480.md) | Loopback only, token checked: 200, 409 on a re-import, 401 on a bad token. |
| A Shadertoy URL typed on a playlist cue | Unverified | [D229](DECISIONS.md) | Uses the same import path as above, so it has the same live-site gap. |
| Author, source and licence kept in `SOURCE` | Supported | [D95](DECISIONS.md), `crates/strobe-shadertoy/tests/golden` | Records CC BY-NC-SA 3.0 unless the author says otherwise. Not a legal determination; licences are the user's responsibility ([Q15](OPEN_QUESTIONS.md)). |
| Generated controls for an import | Supported | [D123](DECISIONS.md)-[D126](DECISIONS.md), [D202](DECISIONS.md), [D219](DECISIONS.md), `crates/strobe-autocontrol/tests/engine.rs`, [T480 Task 23](hardware/T480.md) | Heuristics have only seen Strobe's own scenes and fixtures. Not yet clicked in the app by a person on Windows. |

## 5. Graphics backends and platforms

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Native OpenGL 3.3 core, Intel UHD 620 (Windows 11) | Supported | [T480](hardware/T480.md) (app on the real GPU, Tasks 14, 17, 30) | Driver 30.0.101.1122. Heavy scenes need adaptive resolution at 1440p. |
| Native OpenGL 3.3 core, NVIDIA GTX 1080 Ti (Windows 10) | Supported | [TORNADO T1](hardware/TORNADO.md), [TORNADO T18](hardware/TORNADO.md), [TORNADO T19](hardware/TORNADO.md) | Driver 582.78. |
| Native OpenGL 3.3 core, AMD Radeon RX 580 (Windows 10) | Supported | [AMD Task A1](hardware/AMD.md) | Driver 31.0.21925.1001. All 12 scenes, both dialects, 0 KHR_debug messages. No robust context: see context loss. |
| Intel HD 4000 (the stated GL 3.3 floor) | Unverified | [D3](DECISIONS.md) | No run on an HD 4000 is recorded ([P-03](IMPLEMENTATION_BACKLOG.md)). |
| ANGLE, GLES 3.0 on Direct3D 11 (`--gl angle`) | Supported | [T480 Task 30](hardware/T480.md), [T480 Task 21](hardware/T480.md), [TORNADO T1](hardware/TORNADO.md), [TORNADO T18](hardware/TORNADO.md), [AMD Task A1](hardware/AMD.md) | DLLs from a pinned Electron release ([D12](DECISIONS.md)). Compiles are slower on heavy scenes (3.2 s against 275 ms native on the T480). |
| Context loss: rebuild, then fail over to ANGLE | Unverified | [D66](DECISIONS.md), [T480 Task 8](hardware/T480.md), `STROBE_FAULT` policy tests, [AMD Task A1](hardware/AMD.md) | Injected losses recovered on real WGL. No real driver reset has been observed. AMD's native WGL refuses a robust context, so a reset there is not reported; ANGLE on AMD is robust. |
| Adaptive render scale | Supported | [D64](DECISIONS.md), [T480](hardware/T480.md) | Thresholds are validated on the T480 only. |
| Headless llvmpipe (CI, Linux) | Supported | `crates/strobe-gfx/tests/render.rs`, CI `linux` job | For tests and frames. Timings are CPU rasterisation, not GPU numbers. |
| Windows release zip | Unverified | [T480 Task 21](hardware/T480.md), [T480 Task 22](hardware/T480.md), [Q12](OPEN_QUESTIONS.md) | Packaged by hand with the workflow's steps and run from `%TEMP%`. `release.yml` has never run from a tag ([R-05](IMPLEMENTATION_BACKLOG.md)). |
| Crash reports and symbolication (Windows) | Supported | [T480 Task 22](hardware/T480.md), [R5](OPEN_QUESTIONS.md) | Minidump and report written by the packaged build; `cargo xtask symbolicate` named the frames. A bare `abort()` is not caught. |
| Linux desktop app | Unverified | CI `linux` job (Xvfb smoke test), [STATUS_AUDIT P1](STATUS_AUDIT.md) | Run only under Xvfb on llvmpipe. No release artifact, no real Linux GPU, no Wayland check. |
| macOS | Unsupported | [D55](DECISIONS.md) | No CGL context provider; CI builds the portable crates only. |
| Vulkan, wgpu, Direct3D 12, Metal | Unsupported | [ROADMAP item 10](ROADMAP.md), [GFX_API_RESEARCH.md](GFX_API_RESEARCH.md) | Research only. |

## 6. Audio

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Live capture from the default recording device | Supported | [D61](DECISIONS.md), [T480](hardware/T480.md) (Realtek microphone at 48 kHz), [TORNADO T19](hardware/TORNADO.md), [D266](DECISIONS.md) | Windows microphone privacy settings must allow desktop apps. A device that reports buffer glitches keeps its stream (D266). On the AMD desktop it reopened 70 times a second before that fix; after it, 188 s ran with no reopen and no glitch (Task A2). |
| Capturing what the computer plays (loopback, Stereo Mix) | Unverified | [USER_GUIDE](USER_GUIDE.md) | Output devices are listed as loopback; no run is recorded. |
| Synthetic 120 BPM fallback | Supported | [D61](DECISIONS.md), [STATUS_AUDIT P1](STATUS_AUDIT.md) (missing and synthetic audio states) | Used when no device opens, and by the bench and tests. |
| Device removed and reconnected | Unverified | [R-01](IMPLEMENTATION_BACKLOG.md), [T480 Task 20](hardware/T480.md) | Capture retries every 2 s by design. Never run on hardware: it waits for the owner's approval. |
| Tempo estimate and beat grid | Unverified | [D20](DECISIONS.md), [D21](DECISIONS.md), `strobe-audio` `tempo` tests | Synthetic signals only. |
| Tap tempo and nudge | Supported | `tempo.rs` `tap_tempo_averages_the_intervals`, `nudge_overrides_and_clear_restores` | Not hardware-dependent. |
| Downbeat estimate | Unverified | [D233](DECISIONS.md), `strobe-audio` `downbeat` tests | Synthetic kicks only. Unsure means bars count from the grid's origin. |
| Breakdowns and drops | Unverified | [D226](DECISIONS.md), `strobe-audio` `events` tests | Synthetic structure and a kick/hiss/kick signal. Thresholds are first estimates. |
| Song changes | Unverified | [D237](DECISIONS.md), `strobe-audio` `song` tests | A gap or a new held tempo. A tempo-matched DJ mix with no gap is not heard. |

## 7. Playlists

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| TOML format, sections, inheritance, generator | Supported | [D73](DECISIONS.md)-[D77](DECISIONS.md), `crates/strobe-playlist/tests/example.rs`, unit tests | |
| Precompile at load, zero shader compiles during playback | Supported | [D151](DECISIONS.md), [D152](DECISIONS.md), `crates/strobe-app/tests/show.rs`, [T480 Task 14](hardware/T480.md), [TORNADO T19](hardware/TORNADO.md) | A scene that fails to compile is skipped and shown. |
| Transitions: cut, crossfade, fade through black, custom GLSL | Supported | [D154](DECISIONS.md), `crates/strobe-gfx/tests/compositor.rs`, [T480 Task 14](hardware/T480.md), [TORNADO T19](hardware/TORNADO.md) | Two scenes render during a transition; cost on an HD 4000 unmeasured. |
| 30-minute show with scene changes | Supported | [TORNADO T19](hardware/TORNADO.md), [T480 Task 17](hardware/T480.md) | Flat memory and handles. A 4-hour show has run only as the pure clock ([P-03](IMPLEMENTATION_BACKLOG.md)). |
| Beat-locked cue ends | Unverified | [D75](DECISIONS.md), clock tests | Depends on the tempo estimate, which has not been checked on real music. |
| Audio-event advance (`audio-drop`, `audio-breakdown`, `audio-song`) | Unverified | [D226](DECISIONS.md), [D237](DECISIONS.md), clock tests | Needs live audio; not heard on real music. Ignored during a transition. |
| Resume after a close, and `--no-resume` | Supported | [D155](DECISIONS.md), [STATUS_AUDIT P2](STATUS_AUDIT.md) (Xvfb logs) | |
| Wall-clock anchor | Supported | [D155](DECISIONS.md), [STATUS_AUDIT P2](STATUS_AUDIT.md) | |
| File watcher and lenient reload | Supported | [D158](DECISIONS.md), [STATUS_AUDIT P2](STATUS_AUDIT.md) | A broken file keeps the playing playlist and names the line. |
| Cue editing in LIST (one or several cues, drag in, disable) | Unverified | [D213](DECISIONS.md), [D224](DECISIONS.md), [D230](DECISIONS.md), `show::edit` tests | Not yet used by a person or with a real pointer drag. |
| Save that keeps the file's comments | Supported | [D231](DECISIONS.md), `merge` and `file` tests | Falls back to canonical text when a merge would not read back exactly. |
| A cue's `preset` and `media` | Supported | [D176](DECISIONS.md), [D217](DECISIONS.md), `tests/show.rs`, [T480 Task 26](hardware/T480.md) | Precedence for media: cue, then operator, then preset, then scene. |

## 8. Show control

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Built-in OSC input (`/strobe/control`, `/meta`, `/cmd`, `/cue`, `/fav`, `/preset`, `/macro`) | Supported | [D172](DECISIONS.md), [D173](DECISIONS.md), `crates/strobe-io/src/osc.rs` tests, [T480 Task 25](hardware/T480.md) | Off until turned on; loopback only unless chosen. |
| Synesthesia OSC input aliases | Supported | [D234](DECISIONS.md), `osc.rs` `synesthesias_addresses_are_aliases_and_the_rest_is_ignored`, [T480 Task 31](hardware/T480.md) | `/controls/scene\|meta/<name>`, `/controls/global/<type>/<n>`, two or three values to a pad or colour, `/favslots/<n>`, `/playlist/next\|previous\|toggleplay`. Ignored: `/raw` and the other suffixes, scene-specific addresses, string arguments. |
| OSC from another machine on the LAN | Unverified | [STATUS_AUDIT P2](STATUS_AUDIT.md) | `--osc-all-interfaces` was bound, never reached from a LAN. |
| OSC feedback output | Supported | [D209](DECISIONS.md), `feedback` tests, [T480 Task 25](hardware/T480.md) | Changes only, on `/strobe/...`. Synesthesia's output layout is not produced. Not tried with a real TouchOSC layout. |
| MIDI input and mapping | Unverified | [D170](DECISIONS.md), [D171](DECISIONS.md), parser tests from recorded bytes | No MIDI controller has been used ([P-04](IMPLEMENTATION_BACKLOG.md)). |
| MIDI feedback output (faders, LED rings, pad lights) | Unverified | [D257](DECISIONS.md), `midi_feedback` tests, `mapping.rs` `unshape_finds_the_travel_that_shape_turned_into_a_value` | Off by default. CC, pitch bend (14-bit) and note lights, for mappings on a named device. No controller has received it yet ([P-04](IMPLEMENTATION_BACKLOG.md)). |
| Importing Synesthesia's MIDI mappings | Unsupported | [D234](DECISIONS.md), [Q23](OPEN_QUESTIONS.md) | They are internal identifiers with no export. |
| Learn | Degraded | [D174](DECISIONS.md), [STATUS_AUDIT P2](STATUS_AUDIT.md) (Xvfb click, OSC) | Verified with OSC under Xvfb only; MIDI Learn and Windows are unverified. |
| Macros: reach, colour, pinning, glide, key | Supported | [D210](DECISIONS.md), [D227](DECISIONS.md), [D228](DECISIONS.md), `strobe-io` `macros` and engine tests, [T480 Task 29](hardware/T480.md) | The key and the reach were checked on the T480 with posted keys. |
| Presets: save, recall, rename, delete, media channel | Supported | [D176](DECISIONS.md), [D214](DECISIONS.md), [D223](DECISIONS.md), `strobe-scene` `preset` tests, `tests/show.rs` | |
| Fav slots | Supported | [D176](DECISIONS.md), `preset` tests, [T480 Task 31](hardware/T480.md) | Recall of a filled slot over OSC on hardware is not recorded; an empty slot says so. |
| Keyboard shortcuts and the shortcut editor | Supported | [D178](DECISIONS.md), [D208](DECISIONS.md), `commands.rs` and `ui::shell_tests`, [T480](hardware/T480.md) (Right, B) | Ctrl chords were not testable on the T480 with posted keys. |
| Blackout on, off and toggle | Supported | [D181](DECISIONS.md), [D201](DECISIONS.md), Xvfb OSC run in [STATUS_AUDIT P2](STATUS_AUDIT.md), [T480](hardware/T480.md) (B key) | Blackout gates the shown output only; `master` fades the picture. |
| Command palette | Supported | [D200](DECISIONS.md), [D262](DECISIONS.md), `command_palette` and `ui::shell_tests` | Commands, panes, layouts, library scenes (Load, Preview) and the playing scene's presets. Not yet used by a person on Windows. |

### Synesthesia's preset, meta and control behaviours (C-06)

Each row is a behaviour Synesthesia documents (its manual's OSC, controls
and presets pages, read on TORNADO in T20), classified against Strobe, with
the test that holds it.

| Behaviour | Status | Evidence | Notes |
|---|---|---|---|
| Meta values stay when the scene changes | Supported | [D207](DECISIONS.md), `meta.rs` `a_new_bank_carries_what_it_has_not_set_itself` | A cue's or preset's own meta values win over the carried ones. |
| A playlist cue's preset and meta apply when the cue starts | Supported | `tests/show.rs` `a_cues_preset_is_applied_when_the_cue_starts_and_an_override_beats_it`, `the_operators_output_settings_carry_across_cues_under_a_cues_own_meta` | |
| A preset holds the scene's, the meta and the media values | Supported | [D176](DECISIONS.md), [D223](DECISIONS.md), `preset.rs` tests | `master` is never saved: it is the operator's fader. |
| Recalling only one part of a preset (`/presets/scene`, `/meta`, `/media`) | Degraded | [D176](DECISIONS.md), [D247](DECISIONS.md) | Which parts a preset holds is chosen when it is saved, not when it is recalled. Synesthesia's `/presets/...` addresses take a string, which the OSC decoder drops. |
| Preset names | Degraded | `preset.rs` `find_name` | Synesthesia's are case-sensitive. Strobe ignores case and reads `_` as a space, because names also arrive in OSC addresses. |
| Fav slots recall a scene and a preset | Supported | `preset.rs` `fav_slots_are_eight_and_a_short_file_is_padded`, [T480 Task 31](hardware/T480.md) | Eight slots. |
| One control to its default or a random value (`.../default`, `.../random`) | Supported | [D247](DECISIONS.md), `target.rs` `default_and_random_cover_a_control_a_group_or_a_bank`, `osc.rs` `synesthesias_addresses_are_aliases_and_the_rest_is_ignored`, `engine.rs` `synesthesia_addresses_reach_the_same_controls` | Also from the Inspector's buttons. A bang is never randomized. |
| A group or a whole bank to default or random (`/controls/groups/...`, `/controls/banks/...`) | Supported | Same tests | Scene and meta banks. A group is matched by name as controls are. |
| A script's writes to meta controls (`setControl("meta/...")`) | Supported | [D160](DECISIONS.md), `tests/script_meta.rs` `a_script_write_to_a_meta_control_reaches_the_sessions_meta_bank` | |
| A script's randomize and default on a control, a group and meta | Supported | `strobe-script` conformance `randomize_and_default_control_and_group`, `meta_default_randomize_and_groups_use_the_same_operations`, `randomize_is_reproducible_from_the_seed` | |
| Audio reactivity and audio speed as meta controls | Supported | `audio.rs` `meta_gains_multiply_the_panels_and_stay_in_range` | They multiply the Audio panel's own gains and carry across scenes like any meta value. |
| Locking a control, a group or a bank (`/lock`) | Unsupported | [D247](DECISIONS.md) | Nothing in Strobe protects a value from a preset, a cue or a randomize yet. |
| Bank undo (`/controls/banks/.../undo`) | Unsupported | [D247](DECISIONS.md) | |
| Back to the preset's value (`.../preset`) | Unsupported | [D247](DECISIONS.md) | |
| Saving a new preset over OSC (`/presets/new`) | Unsupported | [D247](DECISIONS.md) | Saving is the Presets panel's, with a name a person types. |
| Alpha Channel meta | Unsupported | [D248](DECISIONS.md) | Waits for an output that carries alpha (NDI or Spout, deferred). |

## 9. Media

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| A still picture through `syn_Media` | Supported | [D186](DECISIONS.md), `crates/strobe-gfx/tests/assets.rs` `a_still_image_is_read_through_syn_media_upright`, [T480 Task 26](hardware/T480.md) | PNG or JPEG. Chosen in the Inspector, per cue, by `--media` or a drop. |
| An image sequence (a folder) | Supported | `assets.rs` `an_image_sequence_plays_at_its_rate_and_loops`, [T480 Task 27](hardware/T480.md), [T480 Task 28](hardware/T480.md) | Decoded off the frame thread. |
| A long sequence over 192 MiB | Supported | [D225](DECISIONS.md), [T480 Task 28](hardware/T480.md) | Shrunk as a whole to fit, not cut; below 256 px it is cut, with a message. |
| SSF `MEDIA` slot (`[{FILTER, WRAP}]`) | Supported | [D193](DECISIONS.md), `scene.rs` `real_synesthesia_shapes_load`, [TORNADO T21](hardware/TORNADO.md) | Strobe also accepts its own path form, a divergence from SSF. |
| The slot's `WRAP` and `FILTER` applied to chosen media | Unverified | [D238](DECISIONS.md), `scene.rs` `chosen_media_is_sampled_as_the_media_slot_says` | The settings are parsed and tested; no render test checks the sampler. |
| `syn_MediaType` 0 (none) and 1 (image) | Supported | `assets.rs` `no_media_is_black_and_type_zero`, [TORNADO T21](hardware/TORNADO.md) | Matches SSF's documented values. |
| Video (`syn_MediaType` 2) and webcam (3) | Unsupported | [M-04](IMPLEMENTATION_BACKLOG.md) | Deferred research. They read as no media. |
| NDI and Spout, in or out | Unsupported | [M-06](IMPLEMENTATION_BACKLOG.md), [TORNADO T21](hardware/TORNADO.md) | NDI and Spout have no documented `syn_MediaType` value. |

## 10. LAN remote

| Feature | Status | Evidence | Notes |
|---|---|---|---|
| Protocol version 1 (`strobe-remote`) | Supported | [D236](DECISIONS.md), `crates/strobe-remote/src/lib.rs` golden tests | Library only. Bounded, strict decoding; another version gets `version_mismatch`. |
| App-thread command and state bridge | Supported | [D232](DECISIONS.md), [D254](DECISIONS.md), `crates/strobe-remote/src/bridge.rs` and `strobe_app::lan` tests | Drained once a frame with `try` only; commands checked against the app's state, then run as their own `Action`. Frame cost not yet measured on the T480. |
| Network listener (opt-in, LAN only) | Unverified | [D250](DECISIONS.md), [D252](DECISIONS.md), `crates/strobe-remote/tests/server.rs` | Built and tested over loopback sockets in CI, including Host/Origin, size, flood, slow-client and idle cases. Never reached from a phone over Wi-Fi; the Windows firewall prompt not seen. Plain HTTP: trusted networks only. |
| Pairing, tokens, rotation, revocation | Unverified | [D251](DECISIONS.md), `strobe_remote::{pairing, clients}` and `tests/server.rs` | Six-digit one-use code plus desktop approval; token digests on disk; secrets never logged (a test captures the log). No real phone has paired. |
| Phone or tablet controller page | Unverified | [D253](DECISIONS.md), `strobe_remote::server::page` tests | One embedded file, no CDN. Checked statically and with Node's WebSocket client; no real browser has rendered it. No macros (protocol v2), no live preview. |
| HTTPS through the operator's own reverse proxy | Unverified | [D258](DECISIONS.md), `tests/server.rs` `an_https_proxy_on_this_computer_is_accepted_by_its_name_only` | `--remote-proxy NAME`. The proxy and its certificate are the operator's. Not yet tried on a phone. |
| Installable PWA over HTTPS | Unsupported | [L-07](IMPLEMENTATION_BACKLOG.md), [D258](DECISIONS.md) | No manifest or service worker until phones pass through the proxy. |

## Keeping this file true

- Change a row only with evidence: a test, a decision, or a hardware
  record with its machine, backend and commit.
- When STATUS_AUDIT.md gains or loses evidence, update the row here in
  the same change.
- "Works under llvmpipe" does not prove Windows driver behaviour, and a
  synthetic signal does not prove behaviour on real music.
