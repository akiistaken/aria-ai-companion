# Aria — Refactor Progress

Source of truth for what is done, what is next and what is deliberately deferred.
Companion to `ARIA-HANDOFF.md`.

---

## Environment (Phase 0)

| Item | Value |
|---|---|
| Repo root | `C:\Users\Aki\Documents\Backup\Aria` |
| Toolchain | `stable-x86_64-pc-windows-gnu` (Rust 1.99.1) — **GNU, not MSVC** |
| Linker | MinGW `gcc` from scoop. No MSVC Build Tools installed (no `vswhere`, no `link.exe`). `ld.lld` is not on PATH and the GNU toolchain ships no `rust-lld`, so no fast-linker override is set. |
| Backup | `C:\Users\Aki\Documents\Aria-backup-2026-11-11.zip` (verified: 19 `.py`, 25 `.rs`, 8 `.slint`, the `.vrm`) |
| Archive | `C:\Users\Aki\Documents\Aria-archive\` (Python sources, Live2D, puppet, plugins, old runtime data) |

### Temp-directory workaround

`.cargo/config.toml` at the repo root sets `TMP`/`TEMP` to a repo-local `.tmp` directory, so build
scripts (`ring`, `cc`) never touch the permission-restricted system temp. Verified with a clean
`cargo build` and no manually exported environment variables.

### Build timings

| Measurement | Time |
|---|---|
| Clean build, before workspace/profile changes | **19m 24s** |
| Incremental build (touch one line in `app.rs`) | **1.3s** |
| Clean build, whole workspace (aria + avatar + protocol) | **29m 10s** |
| Incremental, after first full build | **1–2s** |

The workspace clean build is slower than the original because dependencies now compile with
`opt-level = 2` (needed for Bevy) and there are three crates instead of one.

Windows Defender exclusions have **not** been added (they need an elevated shell). Recorded here as
requested.

---

## Phase 1 — Cleanup

Done.

- All Python (`*.py`, `*.bat`, `__pycache__`) moved to the archive. The repo is Rust only.
- Archived: `witch_live2d\`, `silver_witch.png/.puppet.json`, `checkmark.svg`, `voice_samples\`
  (grep confirmed no Rust reference), `cua-plugin\`, `tinyfish-mcp\`, `ARIA-TECHNICAL-REPORT.md`.
- Deleted: probe artefacts (`probe.c/.o`, `probe_bin`, `probe_crate`, `build_probe.rs`,
  `probe.slint`), all `build_*.txt` / `*_err.txt` / `*_out.txt` scratch logs, the empty `logs\`,
  and the zero-byte orphan in `assets\tts_cache\`.
- Kept as the behaviour spec: `docs\reference\vtuber_stage.html`, `docs\reference\stage_bridge.js`.
- Runtime data now lives in `%APPDATA%\Aria\` (`data_dir()` in `config.rs`): settings, memory,
  launch cache, PID file, logs, TTS cache. A one-time migration copies the old repo-root files
  across on startup and logs that it happened.
- Removed unused dependencies/features: `tray-icon`; the `windows` features
  `Win32_Graphics_Dwm` and `Win32_UI_Controls` (grep-verified unused).
- Removed debug instrumentation: `Stage::probe`, `Stage::enforce_transparency`, the `DIAG`
  page-load handler and its call site.
- Removed superseded `check_game_dodge` (a duplicate of `apply_game_state`).
- Unwired features kept and marked `#[allow(dead_code)] // TODO(wire-up):`:
  `brain::Attachment`, `brain::analyze_dropped`, `config::assets_dir`,
  `config::AVATAR_SCALE_PRESETS`, `offline::sample_tool_result`,
  `proactive::{focus_remaining_minutes, start_focus, stop_focus}`, `util::today_hour_minute`,
  `voice::set_speaking`, `win32::handle_is_null`.
- Workspace created: root `Cargo.toml` (`resolver = "2"`, members `aria` + `aria-avatar` +
  `aria-protocol`, `exclude = ["vendor/msedge-tts"]`, profiles), `Cargo.lock` at the root,
  profiles removed from `aria\Cargo.toml`.
- `.gitignore` and `settings.example.json` (API keys blanked) written.

---

## Phase 2 — Stability

Done.

1. **Slint binding loops — zero warnings.** The root cause was `ScrollView { width: parent.width;
   height: parent.height }` inside a `HorizontalLayout`, plus the sidebar's
   `y: sidebar.height - 76px` footer and its `x: parent.width - 1px` hairline. Replaced with
   `horizontal-stretch`/`vertical-stretch`, layout padding and a stretch spacer. No child sizes
   itself from its parent any more, and the build prints **zero** binding-loop warnings.
2. **TLS double provider — fixed.** `msedge-tts 0.4.1` hard-coded
   `rustls::crypto::aws_lc_rs` and pulled `rustls` with default features. It is now **vendored** at
   `vendor/msedge-tts` with `rustls` pinned to `["std", "ring", "tls12", "logging"]` and the
   provider switched to `ring`. `cargo tree -i aws-lc-sys` finds no package; only `ring` remains, so
   the `aws-lc-sys` C build is gone. The explicit `install_default()` of the ring provider is kept
   in `main.rs`, because both `ureq` and `tungstenite` still build rustls configs.
3. **RefCell re-entrancy — fixed.** `try_with_app_mut` no longer silently drops work: calls made
   while a mutation is in flight are queued in a thread-local `DEFERRED` list and drained as soon
   as the outer mutation ends. A hard cap (64) prevents an unbounded feedback loop and logs a
   warning. `post` (cross-thread) and `defer` (UI thread) both route through it.
4. **Timers — owned.** All nine leaked `slint::Timer`s now live in `App::timers: Vec<Timer>`, so
   they stop when the app is dropped. The `main.rs` single-shot styling timer is held in a local
   instead of being leaked.
5. **Headpat rate limit:** the detector moved into the avatar process in Phase 5; the JS-side 5 s
   limit is superseded there.

---

## Phase 3 — Avatar renderer spike

Done, with the **layered-window fallback** (the swap-chain path is impossible on this stack).

### Why the fallback was required

`wgpu-hal` reports `composite_alpha_modes: vec![Opaque]` for any plain Win32 HWND
(`wgpu-hal-30.0.1/src/dx12/adapter.rs`; the Vulkan backend matches). A Bevy swap chain therefore
**cannot** composite over the desktop, whatever `composite_alpha_mode`, `transparent: true` or
`ClearColor::NONE` say. Verified directly: Vulkan rejects `PreMultiplied`, and DX12 stays opaque
black over the desktop.

### What was built

- `aria-avatar` crate: **headless Bevy 0.20 + `bevy_vrm1` 0.11**, no `WinitPlugin`, no swap chain.
  The scene renders into an RGBA8 `Image` (`Image::new_target_texture`), is captured through
  `Screenshot::image`, and is presented with `UpdateLayeredWindow` as premultiplied BGRA.
- The window lives on its **own thread with a real message loop**. `UpdateLayeredWindow` sends
  synchronous messages to the window's owner thread; with nobody pumping that queue the call
  deadlocks. That is exactly what happened on the first attempts: the process sat blocked,
  `Not Responding`, with no frames presented.
- The camera receives `RenderTarget::Image` in the *same* system that spawns it. Attaching it from
  a separate `Startup` system silently matched nothing, because `commands.spawn` is invisible to
  other systems until deferred commands are applied.
- Model loading uses an absolute path with `UnapprovedPathMode::Allow`, needed for arbitrary
  `custom_vrm_path` values, and an asset root resolved exe-first then `aria-avatar/assets`.
- Window: 520x740 physical px, layered, frameless, `WS_EX_TOOLWINDOW` (no taskbar or Alt-Tab),
  always-on-top. The main app owns its geometry via `SetWindowPos` on the HWND it reports.

### Custom model check

`C:/Users/Aki/Downloads/vrms/freepurplecozytuberfixedshoes.vrm` (17.0 MB) parses as `"VRMC_vrm"`,
so it is **VRM 1.0**. No conversion needed, and no VRM 0.x support was added.

### Measured

| Metric | Value |
|---|---|
| Backend / adapter | Vulkan / NVIDIA GTX 1060 (Dx12 also works) |
| Alpha path | layered window, premultiplied BGRA |
| Working set after 60 s | ~529 MB |
| CPU over 60 s | 28.3 s (roughly half of one core at 30 FPS) |
| Frame pace | 30 FPS (explicit pacing; headless has no vsync) |
| Stability | 60 s continuous, no panic |

Screenshot: `docs\screenshots\phase3-final-proof.png`, the avatar over a live desktop with desktop
text visible through the gaps around the model.

---

## Phase 4 — Protocol, supervision, window split

Done. Verified end to end by running the real `aria.exe`.

### What was built

- **`aria-protocol`** crate: `serde`-only shared types, one JSON object per line, tagged enums
  (`{"kind":"set_state","data":{…}}`). Three unit tests cover the round-trip and prove a malformed
  or unknown line is ignorable rather than fatal.
- **`aria\src\avatar_link.rs`**: binds `127.0.0.1:0`, generates a token, spawns
  `aria-avatar.exe --port --token --model`, validates the renderer's `Hello` token, and restarts it
  with backoff (1 s, 2 s, 5 s, then gives up after 4 attempts). The child goes into a Windows job
  object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so it cannot outlive the app.
- Method names mirror the old `Stage` (`set_state`, `set_mouse_look`, `set_peeking`, …), so call
  sites in `app.rs` barely changed. `AvatarLink` uses interior mutability, which deleted every
  `stage.borrow_mut()` in the codebase.
- **Single body layout**: `set_window_pos` is the one funnel and now calls `layout_windows`, so
  walking, dragging, edge snap, game dodge, perch, boredom roam and initial placement all move the
  chrome and the avatar window together.
- Added `win32::move_and_size_window`, `win32::put_in_kill_on_close_job`,
  `win32::set_click_through`, and the `Win32_System_JobObjects` feature.

### Two bugs worth recording

1. **`Ready` was never delivered.** The renderer queued it in a `pending` buffer for a socket
   handle the app had not yet attached a writer to. The link now flips to `connected` and flushes
   that queue when the window handle becomes available; without this the app never learned the
   HWND and the avatar stayed at (0,0). Found by driving the renderer against a hand-written
   listener and seeing only `hello` arrive.
2. **Input needed protocol additions** (`SetPointer`, `SetClickThrough`, `Event::HoverChanged`),
   because a layered window receives no input of its own.

### Verified

| Check | Result |
|---|---|
| `aria.exe` spawns `aria-avatar.exe` | yes, both processes live |
| Token handshake | `aria` logs `avatar renderer ready, hwnd=…` |
| Layout follows the app | avatar at (1596, 443), app at (1596, 522) |
| Kill the renderer | restarted within ~8 s, layout re-applied |
| Kill `aria.exe` | renderer dies with it (job object) |

---

## Phase 5 — Avatar behaviour

**Partially done.** This is where the work stopped, so the split is recorded precisely.

| # | Item | State |
|---|---|---|
| 1 | Expressions from emotion | **done** — `SetExpressions` per emotion plus blink, smoothed |
| 2 | Mouth / lip-sync | **done** — `aa`/`oh` smoothed, procedural flap fallback when `talking` |
| 3 | Idle + per-emotion bone posing | **done** — breathing/sway, walk, dance, groove, sit, happy, annoyed, talking, idle |
| 4 | Hand posing | **not done** — fingers are not driven yet |
| 5 | Walking / facing | **done** — stride cycle, facings, yaw interpolation at `dt * 7` |
| 6 | Peeking + drag-inertia lean | **done** — lean ±0.14, body roll/pitch from inertia |
| 7 | Input + gesture detectors | **done** — drag, poke, middle-click, right-click, headpat (4 reversals / 1.25 s, 5 s cool-down, 8 px steps), shake (5 / 0.85 s) |
| 8 | Click-through on transparent areas | **partial** — `WS_EX_TRANSPARENT` toggling is wired, but "over the model" uses an **approximate** screen box (30% × 42% of the window), not a projected bone hull |
| 9 | Camera framing + lighting presets | **framing done**, lighting presets **not done** (the preset travels over the wire and is ignored) |
| 10 | Music vibe / groove body motion | **partial** — the `groove` pose branch exists; `SetMusicVibe` energy is not consumed |
| 11 | Wardrobe props | **not done** — `SetProps` is received and stored, nothing is attached to bones |
| 12 | Headpat hearts | **not done** (optional in the handoff) |
| 13 | Music spectrum removed | **done** (deleted in Phase 6) |

### Posing design decision

The reference applies its numbers to three.js's *normalized* humanoid node, which starts at
identity. `bevy_vrm1` writes the rig's real rest orientation onto the bone transforms, so blindly
overwriting produced a T-pose and composing onto the rest produced a T-pose too. Final rule, in
`posing.rs`: **spine/chest/neck/head/hips compose onto their captured rest rotation; limbs are set
absolutely.** That reproduces the reference pose — screenshot `phase5-arms.png` shows the arms down.

Bone entities are resolved **once per model** and cached in `AvatarState::bones`, together with the
rest rotations captured before the first pose is applied.

### Bevy ECS pitfalls hit (read before touching `behaviors.rs`)

- All `Transform` access in `animate` must live in one `ParamSet`. Four separate
  `Query<&mut Transform, …>` parameters trip `error[B0001]` at runtime even when their filters look
  disjoint. Adding `"debug"` to the Bevy features makes the error name the system instead of
  printing `<Enable the debug feature to see the name>`.
- `apply_pose` is generic over `QueryFilter` so it accepts a `ParamSet` member directly.

Screenshots: `docs\screenshots\phase5-arms.png` (idle pose, arms down, no T-pose),
`phase5-rest.png`, `phase5-happy.png`, `phase5-posed.png`.

---

## Phase 6 — WebView removal

Done.

- Deleted `aria\src\stage.rs`, `aria\src\assets.rs`, `aria\src\stage_bridge.js`,
  `assets\vtuber_stage.html`, and the `mod` lines for the first two.
- Dropped the `wry` and `tiny_http` dependencies; `cargo tree` now shows neither, nor any
  `webview2` crate.
- Removed the `avatar_mode`, `live2d_source`, `puppet_source` and `music_spectrum_enabled`
  settings, their Control Center controls (the Live2D / artwork buttons and the spectrum toggle),
  and the `on_load_live2d` / `on_load_puppet` / `on_toggle_spectrum` handlers. `switch_avatar` now
  just reloads the configured VRM.
- `build.rs` no longer watches `stage_bridge.js`.
- Verified: app runs, **0 `msedgewebview2.exe` processes** spawned.

`docs\reference\` is kept, as the handoff requires, until the last Phase 5 item is ported.

---

## Noticed, not fixed

- `aria\src\app.rs` contains mojibake (double-encoded UTF-8) inside several user-facing strings,
  e.g. the "Continuous screen vision is on …" line and "Thinking …". They render as garbage in the
  bubble. Root cause is a bad encoding round-trip in the original port; fixing them is a text-only
  change and is **not** part of any current phase. Candidate for Phase 7.
- The `window_size` / `STAGE_BASE_W` constants assume a single-monitor work area.
- Writing files through PowerShell `Set-Content` corrupted UTF-8 twice (an em dash became `â€”`).
  Use the file-editing tools for anything with non-ASCII content.
- `aria-avatar` currently builds with 12 warnings, mostly unused protocol variants that Phase 5
  items 4/9/11 will consume.

---

## Next

1. Finish Phase 5 items 4 (hands), 8 (true bone-bounds hit test), 9 (lighting presets), 10 (music
   energy), 11 (wardrobe props).
2. Phase 7 — polish: TTS RMS lip-sync envelope, bubble growth, `SetFps` in game mode, pause when
   hidden.
3. Phase 8 — split `app.rs` into modules.
4. Phase 9 — performance pass and measurements.

`git log` at this point: Phases 0-2 (`initial`), then 3, 4, 6 and the bulk of 5 are committed.
