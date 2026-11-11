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

`.cargo/config.toml` at the repo root sets `TMP`/`TEMP` to a repo-local `.tmp` directory,
so build scripts (`ring`, `cc`) never touch the permission-restricted system temp. Verified with
a clean `cargo build` and no manually exported environment variables.

### Build timings

| Measurement | Time |
|---|---|
| Clean build, before workspace/profile changes | **19m 24s** |
| Incremental build (touch one line in `app.rs`) | **1.3s** |
| Clean build, after workspace + `opt-level`/`debug` profile changes | see below |

Windows Defender exclusions have **not** been added (would need an elevated shell). Recorded
here as requested.

---

## Phase 1 — Cleanup

Done.

- All Python (`*.py`, `*.bat`, `__pycache__`) moved to the archive. Repo is Rust only.
- Archived: `witch_live2d\`, `silver_witch.png/.puppet.json`, `checkmark.svg`,
  `voice_samples\` (grep confirmed no Rust reference), `cua-plugin\`, `tinyfish-mcp\`,
  `ARIA-TECHNICAL-REPORT.md`.
- Deleted: probe artefacts (`probe.c/.o`, `probe_bin`, `probe_crate`, `build_probe.rs`,
  `probe.slint`), all `build_*.txt` / `*_err.txt` / `*_out.txt` scratch logs, empty `logs\`,
  `assets\tts_cache\` zero-byte orphan.
- Kept as behaviour spec: `docs\reference\vtuber_stage.html`, `docs\reference\stage_bridge.js`.
- Runtime data now lives in `%APPDATA%\Aria\` (`data_dir()` in `config.rs`): settings, memory,
  launch cache, PID file, logs, TTS cache. One-time migration copies the old repo-root files
  across on startup and logs it.
- Removed unused dependencies/features: `tray-icon`; `windows` features
  `Win32_Graphics_Dwm` and `Win32_UI_Controls` (grep-verified unused).
- Removed debug instrumentation: `Stage::probe`, `Stage::enforce_transparency`, the `DIAG`
  page-load handler and its call site.
- Removed superseded `check_game_dodge` (duplicate of `apply_game_state`).
- Unwired features kept and marked `#[allow(dead_code)] // TODO(wire-up):`:
  `brain::Attachment`, `brain::analyze_dropped`, `config::assets_dir`,
  `config::AVATAR_SCALE_PRESETS`, `offline::sample_tool_result`,
  `proactive::{focus_remaining_minutes, start_focus, stop_focus}`,
  `stage::{play_audio, set_render_paused}`, `util::today_hour_minute`,
  `voice::set_speaking`, `win32::handle_is_null`.
- Workspace created: root `Cargo.toml` (`resolver = "2"`, members `["aria"]`, profiles),
  `Cargo.lock` at the root, profiles removed from `aria\Cargo.toml`.
- `.gitignore` and `settings.example.json` (API keys blanked) written.

**Deviation:** `git init` is deferred to the end of the phase per handoff rule 3 in Phase 0
("do not `git init` yet"), and is done once the tree is clean.

---

## Phase 2 — Stability

Done.

1. **Slint binding loops — zero warnings.** Root cause was `ScrollView { width: parent.width;
   height: parent.height }` inside a `HorizontalLayout`, plus the sidebar's
   `y: sidebar.height - 76px` footer and `x: parent.width - 1px` hairline. Replaced with
   `horizontal-stretch`/`vertical-stretch`, layout padding, and a stretch spacer; no child sizes
   itself from its parent any more.
2. **TLS double provider — fixed.** `msedge-tts 0.4.1` hard-coded
   `rustls::crypto::aws_lc_rs` and pulled `rustls` with default features. It is now **vendored**
   at `vendor/msedge-tts` with `rustls` pinned to `["std", "ring", "tls12", "logging"]` and the
   provider switched to `ring`. `cargo tree -i aws-lc-sys` finds no package; only `ring` remains,
   so the `aws-lc-sys` C build is gone. The explicit `install_default()` of the ring provider is
   kept in `main.rs` (still needed because both `ureq` and `tungstenite` build rustls configs).
3. **RefCell re-entrancy — fixed.** `try_with_app_mut` no longer silently drops work: calls made
   while a mutation is in flight are queued in a thread-local `DEFERRED` list and drained as soon
   as the outer mutation ends. A hard cap (64) prevents an unbounded feedback loop and logs a
   warning. `post` (cross-thread) and `defer` (UI thread) both route through it.
4. **Timers — owned.** All nine leaked `slint::Timer`s now live in `App::timers: Vec<Timer>`,
   so they stop when the app is dropped. The `main.rs` single-shot styling timer is held in a
   local instead of leaked.
5. **Headpat rate limit:** the detector moves into the avatar process in Phase 5; the JS-side
   5 s limit is superseded there. Tracked in Phase 5.

---

## Noticed, not fixed

- `aria\src\app.rs` contains mojibake (double-encoded UTF-8) inside several user-facing strings,
  e.g. the "Continuous screen vision is on …" line and "Thinking …". They render as garbage in
  the bubble. Root cause is a bad encoding round-trip in the original port; fixing them is a
  text-only change and is **not** part of any current phase. Candidate for Phase 7.
- `Stage::set_music_spectrum_enabled` is pushed to JS but the spectrum is fake data; it is
  removed with the WebView in Phase 6.
- The `window_size` / `STAGE_BASE_W` constants assume a single-monitor work area.

---

## Next

Phase 3 — Bevy + `bevy_vrm1` standalone transparent avatar window.
