# Aria

An autonomous desktop companion for Windows: a VRM avatar who lives on your desktop, talks, walks,
falls off windows, reacts to touch, dances to whatever is playing, and occasionally does something
on her own.

Written in Rust. The avatar is a native transparent-window renderer (Bevy + VRM); everything else is
Slint. The original Python/WebView version is archived and kept only as a behaviour reference — see
[PROGRESS.md](PROGRESS.md).

> **Windows only, x86_64.** The app talks to Win32 (layered windows, SMTC media sessions, WASAPI,
> global hotkeys, DWM, job objects). There is no cross-platform path.

## What she does

| | |
|---|---|
| **Full-body animation** | 17 emotions plus six idle moments, IK legs and arms, procedural gait that plants feet on the taskbar, follow-through springs, hair/cloth physics driven by her own movement |
| **Desktop physics** | Gravity and a walkable floor on every monitor, drag to throw, bounce off screen edges, perch on windows and ride them, fall when they close |
| **Personality** | Personas (moods) that change her idle habits, gestures and speaking style; the AI writes both conversation and idle moments |
| **Voice** | TTS playback with real RMS lip-sync, wake word, push-to-talk, speech-to-text |
| **Music** | Reads Windows system media sessions (Spotify, browsers, players), estimates tempo and levels, and runs dances on the beat |
| **Desktop awareness** | Knows the front-most window, notices games and steps aside, optional desktop control through the Cua Driver |
| **Chat UI** | Speech bubble, chat bar above her head, Control Center, context menu, quick dial, system tray |

## Repository layout

| Crate | What |
|---|---|
| `aria` | the app: AI, voice, personality, desktop integration, UI glue |
| `aria-avatar` | the renderer: a VRM 1.0 stage in its own transparent, click-through window |
| `aria-ui` | the Slint windows, compiled once into their own crate (keeps rebuilds fast) |
| `aria-protocol` | the line-delimited JSON messages between the app and the renderer |
| `aria-build` | build-script helper shared by both binaries (rust-lld linking) |
| `assets/models/` | the bundled VRM model |
| `docs/` | the behaviour reference and development screenshots |
| `scripts/` | `run.ps1`, `avatar-shots.ps1`, `dev-setup.ps1` |
| `vendor/msedge-tts` | patched Edge TTS client, vendored |

The app and the renderer are separate processes: `aria` owns the Slint windows and all logic,
`aria-avatar` owns a layered Win32 window. They speak over a local socket, and the app supervises
the renderer (token check, restart with backoff, kill-on-close job object).

## Getting started

### Prerequisites

- **Windows 10/11, x86_64**. Nothing needs WebView2.
- **Rust stable**, `x86_64-pc-windows-gnu` (GNU, **not** MSVC). Verified with 1.99.1; the crates
  declare `rust-version = "1.85"`.
- **MinGW gcc** on `PATH` (e.g. `scoop install gcc`). No MSVC Build Tools are needed or used.
- A **Vulkan-capable GPU driver** (wgpu renders the avatar headless and reads the frame back).

The `.cargo/config.toml` redirects `TMP`/`TEMP` into a repo-local `.tmp/`, because build scripts
(`ring`, `cc`) otherwise fail on the permission-restricted system temp directory.

### Build and run

```powershell
# debug build, then launch
.\scripts\run.ps1

# release build
.\scripts\run.ps1 -Release

# build only, no launch
.\scripts\run.ps1 -BuildOnly
```

Or use cargo directly:

```powershell
cargo build
.\target\debug\aria.exe
```

> **Always build the whole workspace** (`cargo build`, never `cargo build -p <crate>` for routine
> work). A per-crate build resolves different feature sets on shared dependencies and silently
> invalidates large parts of the tree — a no-op build can turn into a 10-minute rebuild.

**Optional, once, from an elevated shell** — excludes build output and the toolchain from Defender
real-time scanning, which dominates clean build times:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\dev-setup.ps1
```

### Configuration

No keys are committed. Runtime state lives in `%APPDATA%\Aria\settings.json`, created on first run.
[settings.example.json](settings.example.json) shows the shape; the important fields:

| Field | Meaning |
|---|---|
| `base_url` | an OpenAI-compatible endpoint (any provider: OpenAI, DeepSeek, Sail, local, …) |
| `deepseek_api_key` | the API key for that endpoint |
| `ai_model` | the model id |
| `tts_voice`, `tts_enabled` | voice selection and whether she speaks at all |
| `mic_mode` | `push_to_talk`, `wake_word` or `always_open` |
| `avatar_scale`, `body_framing` | size on screen and how much of her is shown |
| `avatar_props` | the 13 wardrobe props (cat ears, halo, glasses, …) |

Most of it is editable from the tray → **Control Center**. Never commit `settings.json`; it is
already ignored, along with logs, the TTS cache and the memory vault.

### Keyboard

| Keys | Action |
|---|---|
| `Alt+Space` | summon her |
| `Ctrl+Shift+M` | toggle the microphone |
| `Ctrl+Shift+Space` | push-to-talk (hold) |
| `Ctrl+Shift+C` | open the Control Center |

Double-click pokes her; a long press is ignored; right-click opens the context menu; middle-click
opens the quick dial.

## Testing

```powershell
cargo test --workspace
```

156 tests pass by default (app, renderer and protocol). A few live tests need a real API key and are
excluded from the default run:

```powershell
cargo test -p aria live_ -- --ignored --nocapture
```

Other checks that need no app:

```powershell
# contact sheets of every emotion and motion, written to target\shots
.\scripts\avatar-shots.ps1

# the renderer alone, with a pose and props forced
$env:ARIA_AVATAR_EMOTION='happy'; $env:ARIA_AVATAR_PROPS='halo,cat_ears'
$env:ARIA_AVATAR_LIGHTING='sunset'; $env:ARIA_AVATAR_FRAMING='full'
.\target\debug\aria-avatar.exe
```

`ARIA_MUSIC_TRACE=1` logs the delivered levels and BPM every three seconds, which is the easiest way
to see whether music detection works on your machine.

## Status and known limits

Phase 0–11 are done; [PROGRESS.md](PROGRESS.md) is the source of truth for what is finished, every
deliberate deviation, measured build/performance numbers, and the bug list found along the way.

Known limits, briefly:

- Clean builds are slow (~30 min) until the Defender exclusion above is applied.
- Beat detection is an estimate from session peaks; dBFS is a digital level, not calibrated SPL.
- Physics treats each monitor's edges as walls, so she never walks to another monitor on her own
  (dragging works).
- Focus mode has no UI entry point yet.

## Licence

No licence file is included, which means all rights are reserved by default. Add one before
publishing or accepting contributions.
