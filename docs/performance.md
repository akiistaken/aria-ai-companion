# Resource usage

The renderer uses a 30 FPS ceiling, drops to 15 FPS at rest, and keeps the existing 10 FPS game
mode. Hiding Aria or locking the session stops rendering and the full Bevy update schedule;
socket commands are still drained so showing her resumes normally. Desktop movement remains
at 60 Hz. Activity no longer automatically raises rendering to 60 FPS.

Frames use a persistent GPU texture readback with pooled staging buffers, instead of creating
a screenshot entity, texture and blit pass for every capture. The two presentation handoffs
retain only the latest frame. The renderer releases CPU texture pixels after upload, uses
wgpu's memory-saving allocation hint, and uses a single lighting cluster without GPU clustering.
Vulkan remains the default; the tested DirectX path did not render MToon parts correctly.

The music worker blocks while inactive and resolves session owners once per enumeration rather
than at 50 Hz. Clipboard checks run once per second. The UI does not copy hidden bubble text
on every typing tick. Periodic working-set trimming was removed because it pages memory out
rather than reducing allocations.

## Measured on this machine

Windows 10, i7-7700HQ (8 logical processors), GTX 1060, bundled model at 260 by 370 pixels.
Both samples use development builds; the updated avatar crate is optimized at level 2.
These are idle measurements, not limits or guarantees for other GPUs, models or active tasks.

| Avatar renderer | Before | After |
|---|---:|---:|
| CPU, normalized across logical processors | 4.16% | 1.54% |
| Physical working set | 500 MiB | 366 MiB |
| Private committed memory | 948 MiB | 517 MiB |
| GPU engine utilization, sum across engines | 8.23% | 1.55% |

The complete app's idle sample used **1.64% CPU, 423 MiB working set and 1.45% on its busiest
GPU engine**. That sample disabled voice, AI idle events, music and Cua to keep the state idle,
and used a separate settings directory. Normal user settings were not changed. It followed a
20-second warm-up and sampled for approximately 25 seconds including counter-query overhead.
The paused renderer measured 0.00% CPU and GPU. A separate dancing sample measured 3.34% CPU,
367 MiB working set and 1.00% GPU. GPU clocks and other desktop activity affect these values.

Run the app in the state you want to measure, then use:

```powershell
.\scripts\measure-resources.ps1
.\scripts\measure-resources.ps1 -Seconds 60 -JsonOut .tmp\resources.json
```

The script measures already-running processes. `GpuPercent` is the busiest engine for each
process, averaged over the sample; `GpuEngineTotalPercent` sums engines for comparison with
the earlier renderer sample. GPU values are blank when Windows exposes no matching counters.

Visual previews checked idle, happiness, dancing, being held and sitting. Socket checks covered
handshake, pause, resume, viewport resize, model reload and shutdown. All 191 workspace tests pass;
six live tests are ignored by default.

Cleanup removed the identical model under `aria-avatar/assets` (the canonical model remains
under `assets/models`), obsolete Cua window-list code, and old root scratch logs. Reference
documents, screenshots and dependency licensing are retained because they still have a purpose.
