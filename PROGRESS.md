# Aria — Refactor Progress

Source of truth for what is done and what is deliberately deferred. (`ARIA-HANDOFF.md`, which
earlier revisions referred to, is not in the repository.)

The Python/WebView implementation is a **reference, not a spec**: its tuning numbers are kept
where they work, and behaviour is changed where something better is possible. Deviations are
called out below.

---

## Status

### Music, walking and the wardrobe — 2026-11-12 (second pass)

Follow-up to the report that vibing/dancing was still broken, the walk was stiff, and the props
were unwanted.

**Music.** Three separate acquisition bugs made her behaviour random between launches, which is
why the same song could get a dance one session and a slow sway the next:

1. The beat tracker reset its whole onset history on **any** quiet sample
   (`if tracker.energy < 0.025 { rhythm = RhythmLock::default() }`). A breakdown, a quiet bar or
   a spoken intro threw away the locked tempo, and re-acquiring it takes seconds. Silence now
   ends a rhythm only after `HOLD` (8 s) of nothing.
2. `listen()` treated an **empty track label** as a new song and cleared the estimate. Media
   metadata is queried with a 750 ms/1.5 s timeout and does come back blank while the music plays
   on, so the tempo was wiped mid-song at random. A blank label now means "same song, metadata
   unavailable"; the reset is an explicit `RESET` flag instead of a tuple comparison.
3. Before the first lock the worker published a **made-up 78 BPM**. That is under the 94 BPM
   groove threshold, so a fast song she had not locked onto yet got a sway, not a dance. It now
   publishes `bpm = 0` ("keep your own tempo") until a real measurement lands.

Loudness was also judged on absolute dBFS, which made quietly mastered tracks sit under the dance
threshold permanently (`DANCE_ENERGY = 0.5`). The level is now `absolute.sqrt() * (0.35 + 0.65 ×
loudness relative to the song's own recent peak)`: absolute level still decides whether anything
is audible at all (real silence is exactly 0), while the song's own loudness sets the scale, so a
good mix reaches dancing at any mastering level. Tests cover all four cases, including a quiet
master and a metadata dropout mid-song.

**Walk.** Two separate things made it read as a stiff glide, and the first one was the real
culprit:

1. **The arms never moved.** The upper body was driven by `lead`, computed from the difference in
   the feet's *view-axis* position. Walking across the screen puts both feet at the same depth, so
   `lead` was always ≈ 0: the arm swing, the shoulder ride and the head nod were all multiplied by
   nothing. Measured over a cycle, the hands travelled 7 cm — effectively pinned to her sides.
2. Knee flexion already reached 7°→69°, so "locked knees" was **not** the cause (the first
   hypothesis, disproved by dumping the joints). The body still lacked the vertical travel and
   weight shift that make a step read as weight, and the upper-body values were one-directional
   sines rather than an authored cycle.

Replaced with a **keyframed cycle** (`locomotion.rs`): contact, down, passing, up, contact — hips
rising and falling twice per cycle, pelvis sway toward the carrying foot, pelvis twist and roll,
arms swinging opposite their own leg, elbows closing on the forward swing, trunk lean swelling
into each push-off. Every key is sampled by `Gait::phase`, which advances with distance travelled,
so the cycle stays locked to the planted foot at any speed. The knee bend then comes from the hips
through IK (an extra "loading sink" term was tried and measured to add nothing, so it was not
kept — noted in the code so nobody adds it back).

The regression test measures the two things that were actually wrong: the hand's travel relative
to its shoulder across a cycle (a walking arm now moves >15 cm) and the knee's peak flexion. It
was verified to fail on the old arm code and pass on the new.

**Wardrobe.** All 13 built-in props are gone (`cat_ears`, `halo`, `witch_hat`, `wings`, …), along
with their per-frame animation code and the fake physics-object hand posing (`hold_props`). The
wardrobe is now only what the user loads: pick any glTF/GLB/VRM, choose a bone from 13, and set
offset/rotation/scale. Files are spawned as a child scene of the bone's normalized attachment
frame — the same one the rig uses — so they follow that bone exactly. `SetProps` carries the whole
`Vec<PropSpec>`; the renderer builds, moves and despawns scenes to match. Verified live: a whole
VRM scaled to 0.12 sits on her head and tilts with every dance pose.

Regression suite: **165 tests pass** (85 app, 72 avatar, 8 protocol); 4 opt-in live tests are
excluded from the default run.

### Chat input and music motion — 2026-11-12

Follow-up from live Spotify playback: the old 0.3 confidence cutoff rejected real songs whose
beat confidence was around 0.16–0.26. Three agreeing estimates now acquire tempo at 0.12,
isolated tempo outliers are ignored, and short quiet breaks retain the beat. Metadata queries
are bounded, paused sessions are skipped, and an audible Spotify session can keep reactions
running even if Windows never returns a metadata request. Only one metadata query can remain
outstanding. `ARIA_MUSIC_TRACE=1` logs delivered level/BPM every three seconds; the ignored
`live_music_probe` test checks the live meters and production beat worker. The full app was
observed receiving changing Spotify levels/tempo, including silence and subsequent tracks.
Current regression suite: 156 tests pass; 4 opt-in live tests are excluded from the default run.

- Chat now reserves at least 420 logical pixels for its controls and prompt at every avatar
  scale. Opening it transfers focus from the non-activating avatar to the text field. Drafts
  remain editable during a reply, and focused typing keeps the bar open. Empty stage margins
  still pass clicks through to the desktop.
- The greeting uses a wrist wave followed by a peace sign; speech gestures and dance forearm
  rolls are slower and fluid. Finger accents and claps now approach the beat continuously.
- Tempo detection covers 45–180 BPM. Levels use smoothed session-meter dBFS estimates mapped
  from -48 to -6 dB, sent to the renderer at 20 Hz. Song/source changes reset beat history.
  Slow or uncertain rhythms groove; loud ballads cannot trigger fast choreography. Expressions
  blend relaxation, smiles, eyelids and small beat accents from both tempo and level.
- Validation: 154 workspace tests pass (3 live AI tests ignored), plus the native
  `cargo run --example chat_input_check` smoke check at scales 0.5, 1 and 2.5. Scripted avatar
  previews checked the greeting, speech, 65 BPM and 140 BPM at multiple levels. Preview steps
  accept optional `@persona@bpm@dbfs`, for example `idle@ground@0,0@12@8@@65@-18`.
- Beat accuracy was checked with synthetic rhythms; real-song tempo remains an estimate from
  session peaks, and dBFS is a digital level estimate rather than calibrated acoustic dB SPL.


| Phase | State |
|---|---|
| 0 Environment | done |
| 1 Cleanup | done |
| 2 Stability | done |
| 3 Avatar renderer spike | done (layered-window path) |
| 4 Protocol, supervision, window split | done |
| 5 Avatar behaviour | **done** |
| 6 WebView removal | done |
| 7 Polish | **done** |
| 8 Split `app.rs` | **done** |
| 9 Performance pass | **done** (measurements below) |
| Build speed | **done** (measurements below) |
| 10 Full-body animation and desktop physics | **done** (see below) |
| 11 Cleanup, AI SDK, voice input, music, dances, idle moments, Cua Driver | **done** (see below) |

---

## Environment (Phase 0)

| Item | Value |
|---|---|
| Repo root | `C:\Users\Aki\Documents\Backup\Aria` |
| Toolchain | `stable-x86_64-pc-windows-gnu` (Rust 1.99.1) — **GNU, not MSVC** |
| Linker | MinGW `gcc` driving the toolchain's own **rust-lld** (see Build speed). No MSVC Build Tools. |
| Backup | `C:\Users\Aki\Documents\Aria-backup-2026-11-11.zip` (19 `.py`, 25 `.rs`, 8 `.slint`, the `.vrm`) |
| Archive | `C:\Users\Aki\Documents\Aria-archive\` (Python sources, Live2D, puppet, plugins, old runtime data) |

`.cargo/config.toml` sets `TMP`/`TEMP` to a repo-local `.tmp`, so build scripts (`ring`, `cc`)
never touch the permission-restricted system temp.

---

## Build speed

### What was slow, and what changed

1. **Linking.** Phase 0 recorded that the GNU toolchain ships no `rust-lld`. It does:
   `lib/rustlib/x86_64-pc-windows-gnu/bin/gcc-ld/ld.lld.exe`. `aria/build.rs` and
   `aria-avatar/build.rs` now pass `-fuse-ld=lld -B<that dir>` as **per-binary link args**.
   Doing it with `rustflags` would have changed every crate's fingerprint and rebuilt all of
   Bevy and Slint; a link arg only relinks. `ARIA_GNU_LD=1` switches back to MinGW `ld`.
2. **`.slint` edits.** The `aria` crate is mostly Slint-generated code. At `opt-level = 1` every UI
   edit spent ~2 minutes in codegen. `[profile.dev.package.aria] opt-level = 0`; the app does
   nothing CPU-heavy on its own thread, and every dependency stays at `opt-level = 2`.
3. **Hidden full rebuilds.** A plain `cargo build` with nothing changed once took **10m 39s**,
   recompiling `bevy_render` and everything above it. Shared dependencies resolve to different
   feature sets for `-p aria-avatar` and for the whole workspace (e.g. `image` is `png` alone vs
   `bmp,jpeg,png`), so alternating `cargo build -p …` and `cargo build` invalidates large parts
   of the tree. Cargo's workspace feature unification is still unstable on this toolchain
   (`-Zfeature-unification`), so the rule is: **always build the whole workspace**
   (`cargo build` or `scripts\run.ps1`). This is noted in `.cargo/config.toml`.
4. **Windows Defender** scans every rlib and object written. Excluding the build folders needs
   an elevated shell, so it is packaged as `scripts\dev-setup.ps1` (run once, as Administrator).
   Not applied by the agent.

### Measured (dev profile, i7-7700HQ, 4C/8T)

| Edit | Before | After |
|---|---|---|
| Touch one avatar `.rs` file | 13.1 s | **6.3 s** |
| Touch one app `.rs` file | 10.4 s | **7.8 s** |
| Change a `.slint` file | ~126 s | **8.5 s** |
| No-op build | 1.7 s (10m 39s after a `-p` build) | **1.7 s** |

Clean builds are still dominated by compiling Bevy, wgpu and Slint at `opt-level = 2`
(29 min recorded in Phase 0). They are rare: toolchain updates or `cargo clean`. The Defender
exclusion is the remaining lever for them.

---

## Phase 1 — Cleanup

Done. Python, Live2D and puppet assets archived; runtime data in `%APPDATA%\Aria\`; dead code
removed or marked `TODO(wire-up)`; workspace created with `aria`, `aria-avatar`,
`aria-protocol`; `msedge-tts` vendored (see Phase 2).

## Phase 2 — Stability

Done. Zero Slint binding-loop warnings; a single TLS provider (`ring`, `aws-lc-sys` gone);
re-entrant UI mutations are queued instead of dropped; all timers owned by `App`.

## Phase 3 — Avatar renderer spike

Done, with the layered-window path. `wgpu-hal` only reports `Opaque` composite alpha for a plain
Win32 HWND, so the scene renders headless into an `Image`, is read back, and is shown with
`UpdateLayeredWindow` (premultiplied BGRA). The window lives on its own thread with a message
loop, since `UpdateLayeredWindow` deadlocks without one.

## Phase 4 — Protocol, supervision, window split

Done. `aria-protocol` carries line-delimited JSON; `avatar_link.rs` spawns `aria-avatar.exe`,
validates its token, restarts it with backoff and puts it in a kill-on-close job object.

## Phase 5 — Avatar behaviour

**Done.** All items from the original split:

| # | Item | How |
|---|---|---|
| 1 | Expressions from emotion | One `SetExpressions` per frame (see bug list) with the reference weights for blink, aa, oh, angry, happy, relaxed, surprised; head-pat and sleepy overrides |
| 2 | Mouth / lip-sync | Real RMS envelope from the playing TTS (Phase 7); procedural flap when no envelope arrives |
| 3 | Idle + emotion posing | `posing.rs`, ported branch by branch from `animate()` |
| 4 | Hand posing | `applyHandPosing` ported: wrist plus 3 joints × 5 fingers per hand, per emotion |
| 5 | Walking / facing | Stride, arm swing, yaw toward walk direction; body yaw also follows the cursor |
| 6 | Peeking + drag inertia | Lean, head tilt, yaw; inertia smoothed at the reference rate |
| 7 | Input + gestures | Drag, shake, head-pat, double-click poke, right-click menu, middle-click dial |
| 8 | Click-through | **Pixel-accurate**: the hit mask is the alpha of the last presented frame |
| 9 | Framing + lighting | 28° lens like the reference; day/dawn/sunset/night presets; `auto` follows the clock |
| 10 | Music vibe | Groove branch scaled by the loudness the app measures from media apps |
| 11 | Wardrobe props | **Replaced in the 2026-11-12 second pass**: 13 built-ins removed; props are now user-loaded models attached to a bone |
| 12 | Head-pat hearts | Painted into the presented frame in screen space |
| 13 | Spectrum removed | done in Phase 6 |

### The normalized rig (`aria-avatar/src/rig.rs`)

The old `posing.rs` wrote reference numbers straight onto raw bones, with spine/head composed
onto rest and limbs set absolutely. On the bundled model that produced a **T-pose** (the arms
stuck straight out; see the first screenshot of this session), and the numbers had been hand-
tweaked away from the reference to compensate.

The reference drives three-vrm's *normalized* bones, so `rig.rs` now does what
`VRMHumanoidRig.update()` does:

```
raw_local = P⁻¹ · N · P · rest_local      P = parent's rest rotation in model space
```

`bevy_vrm1` already snapshots `RestTransform` / `RestGlobalTransform` per bone, so this is exact
for any rig, and the reference numbers are used **verbatim**. Bones a branch does not mention
return to rest each frame. Props hang off a per-bone "attachment frame" child whose rotation is
the inverse rest rotation, which makes child space equal normalized space, so the reference prop
offsets apply unchanged.

Rig resolution is an **exclusive system**: in Bevy 0.20 resources are entities, so a
`Query<EntityRef>` conflicts with `ResMut<AvatarState>` (`B0001` at startup).

### Deliberate deviations from the reference

- **"half" framing is waist-up** (`1.22 × height`, target `+0.22 × height`). The reference's
  1.55 with a 28° lens actually showed knees-up, wasting half of the small stage on legs.
  "full" and "face" keep the reference numbers.
- **The avatar window is the stage slot**, rendered 1:1 at its size (260×370 at scale 1). It was
  a fixed 520×740 surface anchored at the mascot window's top-left, so `avatar_scale` (including
  the game-mode shrink to 0.52) had no effect and the avatar ran off the right screen edge.
- **Gestures need a press on the model.** The app forwards the *global* mouse state, and the old
  detector started a drag on any left press, so dragging anywhere on the desktop could drag Aria.
- Head-pat counts only while the cursor is over the model's head band, not anywhere in the band.

---

## Phase 6 — WebView removal

Done. `wry`, `tiny_http`, the stage HTML/JS and the Live2D/puppet/spectrum settings are gone.
`docs\reference\` is kept as the behaviour reference.

---

## Phase 7 — Polish

**Done.**

| Item | How |
|---|---|
| TTS RMS lip-sync | `voice.rs` wraps the rodio decoder in `LipSync`: ~20 ms windows give RMS (mouth open) and zero-crossing rate (rounded `oh` vs open `aa`). The app forwards it at 30 Hz. |
| Real end of speech | `SPEAKING` drops when the audio actually ends, not after a duration guessed from MP3 size; `stop_speaking` now really stops playback. |
| Bubble growth | The window takes the bubble's laid-out height instead of a fixed 74 px, grows as the text types out, and grows **upward** so the avatar does not jump. |
| `SetFps` in game mode | 10 FPS while a game is detected, 30 otherwise; the renderer boosts to 45 while talking, walking or dancing, like the reference. |
| Pause when hidden | 0 FPS (camera off, window hidden) while the session is locked or the mascot is not visible. |
| Click-through that works | The stage slot is cut out of the Slint window's region (`SetWindowRgn`); before, the transparent Slint window swallowed every click over the avatar's empty space. |
| No focus stealing | Avatar window is `WS_EX_NOACTIVATE` and answers `MA_NOACTIVATE`. |
| Mojibake | Fixed (see below). |
| HiDPI | Mascot window sizes are logical; the avatar slot is computed with the scale factor. |

### Mojibake

The "noticed" garbage strings were text double-encoded (UTF-8 read as Windows-1252, re-encoded)
**many times over**: each `—` had grown to ~10 KB. `app.rs` went from 125k to 58k characters.
Every run of non-ASCII characters in `aria/src`, `aria/ui` and the avatar crates was decoded
until stable; real characters (em dash, ellipsis, curly quotes, `·`, emoji) fail the round trip
and were left alone. The same old tooling had also put a blank line before every `use`, which
split two strings mid-word (`"I'll\n\nuse text"`, `"play pa\n\nuse music"`); both are fixed.

---

## Phase 8 — Split `app.rs`

**Done.** `app.rs` (1,700 lines, previously a single file with mangled formatting) is now:

| Module | Contents |
|---|---|
| `app.rs` | `App` state, UI-thread plumbing (`with_app`, `post`, `defer`), `init`, `cleanup` |
| `app/chat.rs` | `say`, bubble, persona accent, microphone, prompts |
| `app/body.rs` | geometry, walking, dragging, edge snap, peeking, the avatar slot and region |
| `app/ticks.rs` | every timer, render policy, game dodge, music vibe, boredom |
| `app/menus.rs` | mascot buttons, global hotkeys, context menu, quick dial |
| `app/avatar.rs` | renderer start-up, its events and settings, poke / shake / head-pat |
| `app/studio.rs` | Control Center (unchanged) |

The split was mechanical (rustfmt, then each top-level function moved by name), so behaviour is
unchanged. Functions are `pub(crate)` and re-exported from `app`, so callers still use
`crate::app::say` and friends. Both crates build with **zero warnings**.

---

## Phase 9 — Performance

| Change | Effect |
|---|---|
| Avatar renders at the slot size (260×370) instead of 520×740 | 4× fewer pixels to render, read back and premultiply |
| Bevy task pools capped at 3 threads | renderer CPU 24.5 → 17.7 s per minute (dev build), 31 → 26 threads |
| Premultiply fast paths for alpha 0 / 255 and a division-free multiply | most of the frame is fully transparent |
| One frame copy fewer (the frame `Vec` moves to the window thread) | |
| Window thread blocks on its queue (`recv_timeout`) instead of polling every 2 ms | |
| App → renderer messages deduplicated (look, pointer, mouth, fps, rect) | the 30 Hz tick no longer writes 3 lines every frame |
| TTS text regexes compiled once | was recompiled for every spoken line |

### Measured (60 s at idle, after a 20 s warm-up)

| Build | Process | CPU per minute | Working set | Threads |
|---|---|---|---|---|
| Phase 3 spike (520×740, dev) | aria-avatar | 28.3 s | ~529 MB | — |
| This session, dev | aria-avatar | 17.7 s | 574 MB | 26 |
| This session, dev | aria | (not readable from the sandboxed shell) | 76 MB | 14 |

Release numbers are **not measured yet**: the release build (thin LTO over Bevy) was stopped by
the agent host because the machine ran low on memory. Run `.\scripts\run.ps1 -Release` and repeat
the 60 s sample. Expect the renderer to drop further, since dev builds run Bevy with debug
assertions.

Most of the renderer's memory is the Vulkan driver and wgpu; the model itself is small.

---

## Phase 10 — Full-body animation and desktop physics

**Done.** Replaces the Phase 5 port of the reference's `animate()` (`posing.rs` is gone): that
code posed the upper body only, moved the legs only while walking, and hand-tuned Euler angles
that bent fingers backward on the bundled rig.

### Bugs this fixed

- **Mouth open with no voice.** The renderer ran a procedural mouth flap whenever `talking` was
  set, and the app set `talking` when a line *started* and never cleared it. Now the mouth moves
  only from a fresh lip-sync envelope, and the app sends `talking = voice::is_speaking()` every
  tick.
- **Walking on the spot, or walking forever.** Walking was a flag; the window moved at a
  constant 4.5 px per tick in any direction. Now the renderer *measures* its window's movement
  and animates from that, and the app moves her with real acceleration and braking.
- **Legs only animated while walking.** Both legs are solved with IK every frame now.

### Renderer (`aria-avatar`)

| Module | What |
|---|---|
| `rig.rs` | Typed `Bone` enum for all 55 VRM humanoid bones (fixed arrays instead of a string map). Forward kinematics in normalized space. **Missing optional bones** (chest, neck, shoulders, toes, fingers, eyes) fold their rotation into their children, and body `Metrics` (leg and arm lengths, hip width, eye/mouth/chin) are measured off the rest pose, so poses fit any proportions. |
| `pose.rs` | Poses written in anatomical terms (arm hang, swing, elbow bend; finger curl and spread; leg swing, knee bend), authored once for the left side and mirrored. `reach()` places a hand with IK on a body landmark. |
| `ik.rs` | Analytic two-bone IK that tolerates bent or A-pose rests. |
| `emotes.rs` | 17 full-body emotions: idle, talking, happy, laugh, angry (foot stomps), annoyed (arms folded, toe tap), sad, cry (fists at the eyes, sobbing), thinking (hand to chin, finger tapping), surprised, shy, dizzy, sleepy (nodding off, yawns), dance (three figures over side taps), groove, sit (on the floor, hugging knees) and wave. Seven persona styles change the idle habit (hand on hip, hands behind, fists up and bouncing, …) and energy. |
| `face.rs` | Expression blend per emotion, blinks (with the odd double blink), lip-sync; channels the model does not declare are never sent. Expression-driven gaze for models without eye bones. `ModelLoaded` now lists the model's expressions. |
| `locomotion.rs` | `Motion` reads the window's travel. `Gait` plants each foot on a desktop pixel and advances the step cycle with **distance**, with stride growing with √speed, so cadence and stride scale together and a planted foot never slides. Falling and held poses. |
| `animator.rs` | Layers emotion → locomotion (leg IK, hips riding the feet, arm swing) → cursor attention → follow-through springs (lean into acceleration, landing crouch, pendulum swing when carried). A hard landing (> 6.5 m/s) leaves her dizzy. The VRM root travels with the window's movement (scaled down), so hair and clothes spring bones react to walking, throwing and falling. |
| `demo.rs` | Scripted preview runs that save contact sheets, no app needed: `.\scripts\avatar-shots.ps1` (see the script's header). |

### App (`aria`)

| What | How |
|---|---|
| Gravity, taskbar floor | `app/physics.rs` (pure, unit-tested). Feet-space simulation; gravity in m/s² scaled by the renderer's reported pixels per metre. The floor is the work-area bottom of whichever monitor she is on (`win32::work_area_at`). |
| Throwing | Releasing a drag keeps the cursor's velocity (capped at 9 m/s); walls bounce, the top of the screen stops her. |
| Walking | Accelerates, cruises at the persona's pace, jogs for long trips, brakes to stop exactly on the spot. |
| Perching | Jumps up onto the active window (aimed ballistic jump), rides it when it moves, falls when it closes or minimizes, walks off its edge to get down. |
| Thinking | "Thinking…" uses the thinking pose; without subtitles she still strikes it. |
| Emotions fade | Reaction emotions return to idle when the bubble hides; sit, sleepy, dance and groove persist. |
| LLM tags | The prompt and tag parser know all 17 emotions; a test keeps them in sync with `aria_protocol::EMOTIONS`. |
| Chat bar | Moved **above her head** (bubble, bar, stage): with her feet on the taskbar, anything below the stage would be off screen. The empty padding under the stage is now cut out of the click region. |

### Protocol

`SetBody { mode: ground | air | held }`, `SetMood { persona }`, and the `Stage` event (feet offset
in the window, pixels per metre). `SetState` lost `facing_right`/`moving`; `SetDragInertia` and
`DragInertia` are gone (the renderer measures motion itself).

### Verified

- `cargo test --workspace`: 54 app, 52 avatar, 5 protocol tests. Highlights: a planted foot does
  not slide while walking; feet alternate; every emotion × persona stays finite on a full rig and
  on a 15-mandatory-bone rig; hands reach chin, eyes and hips; legs read correctly from the front
  in the air and when held; a fall lands at √(2gh); a walk stops exactly on its target; a jump
  lands on a window.
- Contact sheets of every emotion, walking both ways, stopping, falling, landing and being held,
  on the bundled model.
- Live, on the user's own custom VRM: her soles are exactly on the taskbar (0 px off), the
  mouth stays closed when silent, and the bar slot sits on screen above her head.

### Notes

- New installs default to `full` framing (head to toe). With `half` or `face` the legs are
  cropped and the bottom of the avatar slot rests on the taskbar instead.
- The renderer's logs go nowhere when the app spawns it; run it standalone to read them (the rig
  line lists any optional bones a custom model lacks).

---

## Phase 11 — Cleanup, AI SDK, voice input, music, dances, idle moments, Cua Driver

### Workspace and build speed

| Crate | What |
|---|---|
| `aria` | the app: logic and UI glue |
| `aria-ui` | **new**: only the generated Slint code (`ui/*.slint` moved here) |
| `aria-avatar` | the renderer |
| `aria-protocol` | the socket messages |
| `aria-build` | **new**: the rust-lld build-script helper both binaries used to copy |

Shared dependency versions live in `[workspace.dependencies]`. Rust type-checks a whole crate on
every edit, and the generated UI was most of `aria`; with it in its own crate an app edit no longer
re-checks it.

| Edit (dev, steady state) | Before | After |
|---|---|---|
| Touch one app `.rs` file | 9.2 s | **5.7 s** |
| Touch one avatar `.rs` file | 6.5 s | 6.2 s |
| Change a `.slint` file | ~11 s | 12.8 s |
| No-op build | 1.8 s | 1.6 s |

(The recorded 7.8 s in "Build speed" above was before Phase 10 grew the crate.)

### App layout

| Folder | Modules |
|---|---|
| `ai/` | `client` (async-openai), `prompt`, `brain` (conversation + tool loop), `tools`, `tasks` (one-shot jobs), `facts` |
| `persona/` | the catalogue and flavour (`mod`), `mood`, `reactions` |
| `voice/` | `tts`, `lipsync`, `mic`, `stt`, `listener` |
| `desktop/` | `win32`, `input`, `cua`, `presence`, `media`, `beat`, `spotify`, `audio`, `system`, `launcher`, `com` |
| `app/` | unchanged split (chat, body, physics, ticks, menus, avatar, studio) |

Duplicates removed: three copies of `capitalize`/`title_case`, two of the `persona_line!` macro,
two game lists (one each in `mood` and `proactive`, with different contents), two persona-key
lists, two mic-capture paths, two copies of the lld helper, a hand-rolled base64 encoder (now the
`base64` crate, already in the tree). Dead code removed: `offline.rs` (813 lines), the Ollama
fallback, `analyze_dropped` and `Attachment` (never reachable), `config::set_api_key` (wrote
process environment variables with `unsafe`), `handle_is_null`, `fact_texts`, and others. Zero
compiler warnings, zero clippy warnings (`cargo clippy --workspace --all-targets`).

### AI endpoint (`ai/`)

- **async-openai** (0.43, `chat-completion` + `byot` + `rustls-no-provider`): typed requests,
  lenient responses (DeepSeek-style finish reasons and `reasoning_content` would fail the strict
  types). No second TLS backend: the client uses the `ring` provider `main` installs.
- **One entry point** (`client::complete`) for every model call: endpoint, timeout (90 s),
  classified errors, and a retry without any optional parameter the endpoint refuses
  (`reasoning_effort`, `prompt_cache_key`, `response_format`), remembered for the session.
- **Reasoning** setting in the Control Center (Engine → Reasoning): Auto / Off / Low / Medium /
  High → `reasoning_effort`. Background jobs use Low unless the setting is Auto.
- **Caching**: requests are laid out stable-first (persona prompt, tools, history) with the
  per-turn context (mood, memory, time) last, and carry `prompt_cache_key`. Measured on the Sail
  endpoint: 5,801 prompt tokens, **5,775 cached** on a repeat. Token and cache use is logged per call.
- **Offline mode removed.** Every reply comes from the endpoint; without a key she says so. Touch
  reactions stay scripted (they answer a gesture instantly). Proactive events now carry a
  `situation`, and the AI writes her line for it; the scripted line is only the fallback.
- Persona switch requests ("be genki") now switch the mood *and* let the model answer in it.
- Empty replies (a reasoning model thinking through its budget) are retried once.

### Voice input (the bug: mic mode never took effect)

The hands-free listener was only started by flipping the wake-word toggle, so "Always-Open" did
nothing, wake-word phrases were logged and dropped, push-to-talk had no key, and a new input
device was ignored until restart. Now `voice::listener::apply` runs at startup and on every change
of mic mode, wake word or input device; heard phrases are submitted; **push-to-talk** records while
Ctrl+Shift+Space is held. The API-key field no longer saves (and wiped the conversation) on every
keystroke.

### Music (`desktop/media.rs`, `desktop/beat.rs`)

- What is playing comes from **Windows system media sessions** (SMTC): Spotify, Wynk, and browser
  tabs (YouTube, YouTube Music, SoundCloud, Wynk web) all publish one. Window titles name the site
  for browser sessions and cover players without a session.
- A **beat tracker** samples the music app's own session meter at 50 Hz (so her voice is never the
  beat), estimates tempo by autocorrelation with a 120 BPM prior, and locks the phase with a comb
  filter. Tested on synthetic kicks at 92–140 BPM.
- `SetMusicVibe` carries `bpm` and `beat`; the renderer runs a **beat clock** that eases onto the
  music's phase (never skipping). Loud music gets a dance, soft music a groove (with hysteresis).

### Avatar

- **Single click** on the model (after the double-click window passes) shows her last line again,
  without re-speaking it. Double-click still pokes; a long press is neither.
- **Smoother**: the app's motion tick is 60 Hz (was 30, so a walk visibly stepped), the renderer
  boosts to 60 FPS while anything moves (was 45), an upper-body follow-through filter eases every
  bone above the hips (legs untouched so feet never slide), and emotion blends use smootherstep
  (overshoot only for big moves).
- **Walk**: soft knees, a head nod and shoulder lift each step, wider arm swing, trailing wrists.
- **Dances** (`emotes/dance.rs`): eight routines on the beat (side taps, idol heart, disco point,
  shimmy, concert wave, bounce-and-clap, step kicks, spin and pose), one per 16-beat phrase, dealt
  in shuffled decks so none repeats back to back; calm personas use the subtler four. Three groove
  loops (nod, sway and snap, step-touch).
- **Idle moments** (`emotes/moments.rs`, new emotions): stretch, look around, hum, twirl, fix hair,
  clap. The app asks the AI (`ai::tasks::idle_event`) every 1–4 minutes of quiet, paced by persona,
  what she does next given her mood and what the user is doing; usually silent. Toggle: Presence →
  Idle moments.
- `emotes.rs` (1,038 lines) became `emotes/{mod, moments, dance}.rs`.

### Cua Driver (`desktop/cua.rs`)

`cua-driver mcp` is started as a child (kill-on-close job) and spoken to over MCP JSON-RPC on stdio.
- **Tools**: eight driver tools (`desktop_list_windows`, `desktop_get_window_state`,
  `desktop_click`, `desktop_type_text`, `desktop_press_key`, `desktop_hotkey`, `desktop_scroll`,
  `desktop_bring_to_front`) with compacted schemas. They replace the three legacy tools that guessed
  pixels and stole focus; without the driver the legacy tools come back.
- **Presence and games** (`desktop/presence.rs`): the front-most on-screen window comes from the
  driver's z-ordered window list (Win32 fallback). Games are detected by install location (Steam,
  Epic, Riot, GOG, Xbox, Battle.net, …), then by title/exe, then by covering the monitor (browsers
  and players excluded). Fullscreen video with media playing is not "away".
- Toggle: Presence → Desktop control → Cua Driver.

### Verified

- `cargo test --workspace`: **80 app, 62 avatar, 7 protocol** tests, plus three live tests
  (`--ignored`) that passed against the Sail endpoint: a tool-calling turn (`get_now_playing_media`
  read the real Spotify track through SMTC, reply in persona with tags parsed), an AI idle event, an
  AI proactive line, and the cache numbers above.
- Contact sheets of all eight dances over 32 s, every idle moment, and the new walk.
- A 50 s live run: the always-open listener started on the saved setting, and the playing Spotify
  track was detected.

### Not done / open

- Not exercised live: push-to-talk and the wake word with a real voice; the beat tracker on real
  songs (only synthetic beats in tests); a driver tool call made by the model in a real turn.
- `launcher.rs` (922 lines) and `win32.rs` were cleaned only where touched.
- Focus mode still has no UI entry point (`TODO(wire-up)`).

---

## Bugs found and fixed along the way

- **UI-thread panic on any reply over 160 characters**: `format_speech` split sentences with a
  look-behind regex, which the `regex` crate rejects at runtime. It ran inside `speak_async` on
  the UI thread.
- **Only one expression ever applied**: `SetExpressions` replaces every override, and it was
  triggered once per expression per frame, so blink (sent last) erased the emotion.
- **Loading a model spawned a second avatar**: the previous VRM root was never despawned.
- `ModelLoaded` was never sent; load failures now fall back to the bundled model and report it.
- Poke, right-click menu and middle-click dial were listed as done but were never detected.
- The avatar window could snap back to (0, 0): two processes moved it (the app with
  `SetWindowPos`, the renderer with `UpdateLayeredWindow`'s destination point). The renderer's
  window thread is now the only mover; the app sends `SetWindowRect`.
- Resizing the layered DIB deselected the new bitmap (wrong release order), so every later
  `UpdateLayeredWindow` failed with error 31.
- Registry key names in the app launcher carried 256 characters of trailing NULs.
- While speaking, the 30 Hz tick reset `facing_right`/`moving` to false every frame.
- A release renderer would have opened its own console window (now GUI subsystem and
  `CREATE_NO_WINDOW`).
- A date test used a 2025 timestamp but expected 2026.

---

## Verifying

- `cargo test --workspace`: 80 app, 62 avatar, 7 protocol tests pass (see Phases 10 and 11).
- Live endpoint checks (need a key): `cargo test -p aria live_ -- --ignored --nocapture`.
- Avatar contact sheets without the app: `.\scripts\avatar-shots.ps1` (pictures in
  `target\shots`).
- Standalone preview of the renderer, no app needed:
  `ARIA_AVATAR_EMOTION=happy ARIA_AVATAR_LIGHTING=sunset
  ARIA_AVATAR_FRAMING=full ARIA_AVATAR_SIZE=390x555 target\debug\aria-avatar.exe`
- A prop can be previewed standalone with `ARIA_AVATAR_PROPS="C:\path\hat.glb@head@0,0.12,0@1.0"`
  (path@bone@offset x,y,z@scale); several props are separated by `;`.
- In debug builds the app's log is mirrored to stderr, so a dev run is readable even when
  `%APPDATA%\Aria` is not writable (it was not, from the agent's sandboxed shell).

## Not done / open

Presence/gaming update (2026-11-13): native activity sampling now runs independently every two
seconds, with executable aliases for Valorant/Apex (including DX12), Steam/Epic manifest discovery
on custom drives, engine evidence and launcher/editor exclusions. Protected process names fall
back to Toolhelp enumeration. Controller input and fullscreen media viewing count toward presence.
Game sessions survive Alt-Tab; local transitions are queued and revalidated before speech, with
scripted fallback lines when AI is unavailable. Smart Dodge uses a temporary size independently
of Game Copilot. The Control Center shows current detection evidence, and chat can query it through
`get_pc_activity`. Copilot screenshots use the game monitor and short background requests.
Both avatar/chat windows recover actual topmost order without taking focus. Validation:
`cargo build --workspace`; 176 default tests plus the native `live_topmost` test passed. Actual
Valorant/Apex sessions were not launched; their executable/focus/session cases are regression-tested.
Exclusive fullscreen games can bypass desktop overlays; use borderless/windowed mode.

Valorant data update (2026-11-13): added opt-in automatic signed-in account discovery from the
local Riot Client, with an optional test override instead of a fixed Riot ID. The real-client
probe identified `Akii#myr` automatically; the current launcher supports local RSO userinfo before
the game starts, while `/chat/v1/session` returns 404 at that stage. Region comes from the running
Valorant deployment. Local credentials stay inside a dedicated fixed-loopback HTTPS client with
proxies/redirects disabled; no Tracker.gg key is stored or used.

Structured game-window frames capture only visible own-player statistics, with fresh native focus
checks, account revalidation before/after analysis, confidence thresholds, null missing fields,
and timestamps/staleness. The default interval is 60 seconds. Valorant bypasses generic tactical
game commentary. Readable results screens produce a once-per-visit recap; completed official
VAL-MATCH-V1 JSON imports select the detected PUUID and calculate K/D, ACS, ADR and hit-based
headshot percentage only from complete underlying data. Each account keeps a bounded 20-record
cache, with serialised/merged writes for concurrent imports and captures. The Control Center has
account status, current observations, recap, refresh/import/disconnect and optional test setup.
Chat can query `get_valorant_stats` for sourced observations and reflective analysis.

Validation: 191 default workspace tests pass, including account login/logout, ownership, metric
denominators, missing/duplicate rounds, ambiguous player names, stale/Alt-Tab observations,
account-switch invalidation and bounded/deduplicated history. The ignored `live_account_discovery`
probe passed against the actual Riot Client. `cargo build --workspace` passed. Actual in-game
vision extraction has not been exercised; it needs a visible scoreboard/results screen and a
vision-capable configured model. Automatic remote Riot match fetching/RSO is not connected; the
official match parser is the adapter boundary for future approved backend access. Screen records
have locally generated IDs, so they cannot reliably be reconciled with official imported match IDs.

- Clean builds are still ~30 min until the Defender exclusion is applied.
- Dropping her onto a window does not catch her; only aimed jumps land on windows.
- Physics follows the primary-monitor rules per monitor, but walking never crosses to another
  monitor on its own (each monitor's edges are walls); dragging does.
- Not exercised live: throwing and lip-sync during speech (injected input did not reach the app
  from the agent's shell). Both are covered by unit tests.
