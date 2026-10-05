# Technical Architecture Document — Tempo 2

> **Status (2026-10-03): partly out of date.** This document was written for version 1.0.
> The product and interface documents were rewritten on 2026-10-03 and take priority.
>
> Superseded here:
> - **Plugins** (§1 diagram, §3 crate table, §9, §10): the runtime is now sandboxed Lua, not
>   wasmtime. There is no Python compute server in 1.0. See `PLUGIN_SPEC.md`.
> - **Pages**: there is one Edit page and one Export page; no Cut page. See `UI_SPEC.md`.
> - **Timeline drawing** (§7.4): the timeline is a custom GTK widget drawn in `snapshot()`.
>   Skia is not needed for it.
> - **Transitions** (§7.3): fragment shaders, not compute shaders, so they work on the
>   OpenGL ES fallback.
> - **Thread messaging** (§4.2): use `async-channel` with `spawn_local`;
>   `glib::MainContext::channel` no longer exists.
>
> Still to be redesigned (open problems, not yet rewritten):
> - How decoded frames reach the screen without a GPU → CPU → GPU copy.
> - Timeline state shared as `Arc<RwLock<Project>>`; workers should get snapshots instead.
> - The memory table in §10 leaves out decoder buffers and export.

**Version:** 1.0  
**Status:** Approved  

---

## 1. Architecture Overview

Tempo 2 uses a **process-per-concern** architecture with **strict thread ownership** within the
main process. The rendering pipeline is fully decoupled from the UI layer through a message-passing
system. No shared mutable state crosses thread boundaries without explicit synchronization.

```
┌─────────────────────────────────────────────────────────────────────┐
│  TEMPO PROCESS                                                       │
│                                                                      │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────────┐  │
│  │  UI Thread   │    │ Render Thread│    │  Decode Thread Pool  │  │
│  │  (GTK main)  │◄──►│  (wgpu)      │◄──►│  (rayon, N threads)  │  │
│  └──────┬───────┘    └──────┬───────┘    └──────────────────────┘  │
│         │                   │                                        │
│         │ Commands           │ Frame requests                        │
│         ▼                   ▼                                        │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────────┐  │
│  │  Timeline    │    │ Frame Cache  │    │  Proxy Thread Pool   │  │
│  │  State       │    │  (LRU, Rust) │    │  (background, low    │  │
│  │  (Arc<Mutex>)│    │              │    │   priority threads)  │  │
│  └──────────────┘    └──────────────┘    └──────────────────────┘  │
│         │                                                            │
│  ┌──────▼───────┐    ┌──────────────┐    ┌──────────────────────┐  │
│  │  Command Log │    │ Audio Thread │    │  Plugin Sandbox      │  │
│  │  (undo/redo) │    │  (PipeWire)  │    │  (wasmtime, isolated) │  │
│  └──────────────┘    └──────────────┘    └──────────────────────┘  │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
         │                          │
         │ IPC (Unix socket)         │ Flatpak portal
         ▼                          ▼
┌─────────────────┐     ┌──────────────────────┐
│  Python Compute │     │  System Services      │
│  Server         │     │  (PipeWire, VAAPI,    │
│  (optional,     │     │   XDG portals)        │
│  lazy-launched) │     └──────────────────────┘
└─────────────────┘
```

---

## 2. Crate Dependency Graph

Crates are listed from lowest-level (no app deps) to highest-level (depends on most):

```
tempo-timeline   ←── Pure data, no UI, no media, no async
tempo-media      ←── FFmpeg bindings, format detection
tempo-render     ←── wgpu + Skia, depends on tempo-timeline
tempo-audio      ←── PipeWire, depends on tempo-timeline
tempo-proxy      ←── Depends on tempo-media
tempo-export     ←── Depends on tempo-media, tempo-timeline
tempo-plugin     ←── wasmtime sandbox, depends on tempo-timeline
tempo-project    ←── SQLite, depends on tempo-timeline
tempo-compute    ←── Unix socket client, no major deps
tempo-ui         ←── GTK4 widgets, depends on all above
tempo-app        ←── Entry point, wires everything together
```

**Rule:** No crate lower in the graph may depend on a crate higher in the graph. `tempo-timeline`
has zero app-level dependencies — it is a pure Rust library that could be published as a standalone
crate.

---

## 3. Tech Stack — Crate List

### Core Rust Crates

| Crate | Version (approx) | Purpose |
|---|---|---|
| `gtk4` | ≥ 0.9 | GTK4 bindings |
| `libadwaita` | ≥ 0.7 | libadwaita bindings |
| `glib` | ≥ 0.20 | GLib mainloop, signals |
| `wgpu` | ≥ 22 | GPU-agnostic rendering (Vulkan/OpenGL/Metal) |
| `skia-safe` | ≥ 0.75 | 2D canvas (timeline, waveforms, thumbnails) |
| `ffmpeg-next` | ≥ 7 | FFmpeg libav* bindings |
| `pipewire` | ≥ 0.8 | PipeWire audio |
| `wasmtime` | ≥ 23 | WASM plugin sandbox |
| `rusqlite` | ≥ 0.32 | SQLite project files |
| `mlua` | ≥ 0.10 | Lua 5.4 embedded (Code Mode plugin) |
| `rayon` | ≥ 1.10 | Thread pool for decode, thumbnail generation |
| `tokio` | ≥ 1.40 | Async runtime (compute server IPC, export) |
| `serde` + `serde_json` | ≥ 1 | Serialization |
| `toml` | ≥ 0.8 | Plugin manifest parsing |
| `image` | ≥ 0.25 | Image format support (stills import) |
| `rubato` | ≥ 0.15 | Audio resampling (SRC) |
| `rubberband` | ≥ 0.7 | Audio time-stretching for speed ramps |
| `crossbeam` | ≥ 0.8 | Lock-free queues between threads |
| `parking_lot` | ≥ 0.12 | Fast Mutex/RwLock |
| `tracing` + `tracing-subscriber` | ≥ 0.3 | Structured logging |
| `thiserror` | ≥ 2 | Error type derivation |
| `anyhow` | ≥ 1 | Error propagation in non-library code |
| `once_cell` | ≥ 1 | Lazy global initialization |
| `tempfile` | ≥ 3 | Temp files for proxy staging |
| `uuid` | ≥ 1 | Clip/track/marker IDs |
| `chrono` | ≥ 0.4 | Timestamps for autosave, project metadata |

### Build Dependencies

| Tool | Purpose |
|---|---|
| Rust toolchain ≥ 1.82 (stable) | Compiler |
| `cargo` | Build and dependency management |
| `bindgen` | FFmpeg header binding generation |
| `slang` compiler | Compile `.slang` shaders → WGSL |
| `glib-compile-resources` | Bundle GTK resource files |
| `flatpak-builder` | Flatpak packaging |
| `cargo-deny` | License and dependency audit |

---

## 4. Threading Model

### 4.1 Thread Inventory

| Thread | Owner | Responsibilities | Blocking? |
|---|---|---|---|
| **UI Thread** | GTK4 main loop | All widget rendering, user input, menu, dialogs | Never blocks (all I/O offloaded) |
| **Render Thread** | `tempo-render` | wgpu frame composition, presenting to viewer widget | Blocks on wgpu frame present |
| **Decode Pool** | `rayon` global pool | FFmpeg frame decode, thumbnail generation | Yes (CPU-bound) |
| **Proxy Pool** | Separate `rayon` pool | Background proxy encode, lower priority | Yes (CPU-bound, nice'd) |
| **Audio Thread** | PipeWire real-time | PCM frame delivery to PipeWire | Hard real-time, never allocates |
| **Plugin Thread** | `wasmtime` | Plugin execution (sandboxed) | Yes (WASM is blocking) |
| **Export Thread** | `tempo-export` (`thread::spawn`) | FFmpeg/VAAPI encode pipeline, RenderQueue jobs | Yes (CPU/GPU-bound) |
| **Compute IPC** | `tokio` task | Unix socket calls to Python server | Yes (I/O-bound) |

### 4.2 Thread Communication Patterns

```
UI Thread → Render Thread:    crossbeam::channel (MPSC, bounded 4 messages)
Render Thread → Decode Pool:  rayon::spawn (fire-and-forget frame requests)
Decode Pool → Frame Cache:    parking_lot::RwLock (write decoded frame, brief lock)
Render Thread → Frame Cache:  parking_lot::RwLock (read cached frame, brief lock)
UI Thread → Audio Thread:     atomic seek position (AtomicI64 microseconds)
UI Thread → Proxy Pool:       crossbeam::channel (MPSC, unbounded — proxy jobs queue up)
Proxy Pool → UI Thread:       glib::MainContext::invoke (progress updates to UI)
UI Thread → RenderQueue:      mpsc::channel / atomic cancel/pause tokens (job control)
Export Thread → UI Thread:    std::sync::mpsc::channel<ExportQueueEvent> (polled 100ms by GTK)
```

### 4.3 What Runs on the UI Thread

Strict rule: **the UI thread does zero blocking I/O and zero heavy computation.**

The following is allowed on the UI thread:
- GTK widget creation, property setting, signal connection
- Reading (not mutating) timeline state for display
- Dispatching commands to the command log
- Updating playhead position (reading `AtomicI64`)
- Sending messages to other threads via channels

The following is forbidden on the UI thread:
- File I/O of any kind
- FFmpeg calls of any kind
- SQLite writes (reads: only via WAL mode, brief)
- Network/IPC calls
- Any operation that could take > 1 ms

---

## 5. State Management

### 5.1 Application State

```rust
// Lives in tempo-app, Arc'd across the UI and render threads
pub struct AppState {
    pub project: Arc<RwLock<Project>>,          // current project (timeline + metadata)
    pub command_log: Arc<Mutex<CommandLog>>,     // undo/redo history
    pub media_pool: Arc<RwLock<MediaPool>>,      // imported clips
    pub playback: Arc<PlaybackState>,            // atomic playback state
    pub render_queue: Arc<Mutex<RenderQueue>>,   // in-memory export job queue
    pub plugin_manager: Arc<Mutex<PluginManager>>,
    pub settings: Arc<RwLock<Settings>>,
}

pub struct PlaybackState {
    pub position_us: AtomicI64,       // current playhead in microseconds
    pub playing: AtomicBool,
    pub speed: AtomicF32,             // 1.0 = normal, 2.0 = 2× forward, -1.0 = reverse
    pub loop_start_us: AtomicI64,     // -1 if no loop
    pub loop_end_us: AtomicI64,       // -1 if no loop
}
```

### 5.2 State Mutation Rules

- **Timeline mutations:** Only via Commands dispatched through `CommandLog`. No direct mutation.
- **Media Pool mutations:** Protected by `RwLock`. Write lock only held during import/delete.
- **Playback state:** Atomic ops only. No locks. Written by UI thread (seek, play/pause) and
  read by render + audio threads.
- **Plugin state:** Isolated in `wasmtime` sandbox. Plugins interact with app state only through
  the defined host function API.

---

## 6. Message Bus

The app uses a lightweight message bus for decoupled communication between components:

```rust
pub enum AppMessage {
    // Playback
    Play,
    Pause,
    Seek(i64),           // microseconds
    SetSpeed(f32),

    // Timeline
    CommandExecuted(CommandId),
    CommandUndone(CommandId),

    // Media
    ClipImported(ClipId),
    ClipThumbnailReady(ClipId, usize),  // usize = thumbnail index
    WaveformReady(ClipId),
    ProxyStarted(ClipId),
    ProxyProgress(ClipId, f32),         // 0.0–1.0
    ProxyComplete(ClipId, PathBuf),

    // Export
    ExportStarted,
    ExportProgress(f32, Duration),      // progress, ETA
    ExportComplete(PathBuf),
    ExportFailed(String),

    // Plugin
    PluginInstalled(PluginId),
    PluginEnabled(PluginId),
    PluginDisabled(PluginId),
}
```

Published via `glib::MainContext` for delivery to the UI thread. For cross-thread delivery to
non-UI threads, `crossbeam::channel` is used directly.

---

## 7. Render Pipeline

### 7.1 Overview

```
Playhead position (AtomicI64, microseconds)
        │
        ▼
Timeline::clips_at_time(position) → Vec<ClipAtTime>
        │
        ▼  (for each clip in z-order, bottom-up)
Frame Cache lookup → hit: GPU texture │ miss: request decode
        │
        ▼
wgpu Compositor:
  [V1 clip frame texture]
       ↓ (composited over)
  [V2 clip frame texture]  ← alpha blend with opacity
       ↓
  [V3 clip frame texture]
       ↓
  [V4 clip frame texture]
       ↓
  [Title texture] (rendered by Skia, uploaded as texture)
       ↓
  [Transition shader] (if edit point in current frame range)
       ↓
Final composited frame → display in viewer GTK widget
```

### 7.2 Frame Timing

The render thread runs at the timeline's target FPS. For a 30fps timeline:
- Frame budget: 33.3 ms per frame
- Target composition time: < 8 ms (leaves headroom for decode latency)
- If composition exceeds 33 ms: drop frame and display previous (warn in status bar)

### 7.3 Transition Rendering

Transitions are implemented as wgpu compute shaders (`.slang` source):

```slang
// cross_dissolve.slang
[shader("compute")]
void main(uint3 id: SV_DispatchThreadID,
          Texture2D clipA,
          Texture2D clipB,
          float progress,   // 0.0 = full A, 1.0 = full B
          RWTexture2D output) {
    float4 a = clipA[id.xy];
    float4 b = clipB[id.xy];
    output[id.xy] = lerp(a, b, progress);
}
```

Each transition type is one shader. Plugins can register additional shaders via host functions.

### 7.4 Skia Canvas Layers

Skia renders into a CPU-side surface that is uploaded to a wgpu texture each frame for:
- Timeline track backgrounds and clip bodies
- Waveform data (rendered once per clip, cached as texture)
- Thumbnail filmstrip on clips
- Timeline ruler (timecode labels, frame markers)
- Playhead indicator
- Selection highlight overlay

The Skia canvas for the timeline is re-rendered only when:
- The timeline view scrolls/zooms
- A clip is added, moved, or deleted
- A thumbnail or waveform finishes generating

This avoids re-rendering the Skia canvas every frame — only the viewer composites at full FPS.

---

## 8. Audio Pipeline

```
Playhead position (AtomicI64)
        │
        ▼
AudioEngine::get_audio_at(position, frame_size) → &[f32]
        │
        ▼  (for each audio clip at current position)
FFmpeg audio decode → PCM f32 interleaved
        │
        ▼
Resample to PipeWire target rate (if needed, via rubato)
        │
        ▼
Per-clip: apply volume, pan
        │
        ▼
Mix all clips (sum, clamp to [-1.0, 1.0])
        │
        ▼
Apply master volume
        │
        ▼
PipeWire stream write
```

The audio thread is driven by PipeWire's process callback (pull model). It must deliver audio
within PipeWire's quantum (typically 256 or 1024 samples). The callback may not allocate memory
or lock mutexes — all state is pre-fetched from atomics and lock-free structures.

Audio clip state (volume, pan) is read via a lock-free snapshot approach: the UI thread prepares
a compact `AudioMixPlan` struct and atomically swaps a pointer. The audio thread reads the pointer
at callback time — always sees a consistent, up-to-date plan with no lock contention.

---

## 9. Plugin Architecture

### 9.1 Plugin Isolation

Each plugin runs in its own `wasmtime::Store`. Plugins are loaded and executed on the Plugin
Thread. Plugin execution is synchronous from the plugin's perspective but non-blocking from the
app's perspective (the Plugin Thread owns the store and is separate from the UI thread).

Host functions are implemented in Rust and called by the WASM plugin via imports:

```rust
// Host functions exposed to plugins (wasmtime Linker)
linker.func_wrap("tempo", "timeline_get_clip", |clip_id: i64| -> i32 { ... })?;
linker.func_wrap("tempo", "timeline_add_clip", |track_id: i32, source_id: i64, ...| { ... })?;
linker.func_wrap("tempo", "render_register_effect", |id_ptr: i32, wgsl_ptr: i32, ...| { ... })?;
linker.func_wrap("tempo", "ui_add_panel", |panel_id: i32, title_ptr: i32, ...| { ... })?;
linker.func_wrap("tempo", "log", |level: i32, msg_ptr: i32, msg_len: i32| { ... })?;
```

Plugins communicate with the app's timeline state through these host functions. Plugins cannot
directly access Rust memory outside their WASM linear memory.

### 9.2 Plugin Lifecycle

```
PluginManager::install(path) → validate manifest → copy to plugin dir → emit PluginInstalled
PluginManager::enable(id)    → load WASM → init wasmtime Store → call plugin_init() → register
PluginManager::disable(id)   → call plugin_shutdown() → drop Store → unregister effects/panels
```

Plugins declare in their manifest whether they need the Python compute server. If Python is
unavailable, such plugins are marked unavailable in the Plugin Manager.

---

## 10. Memory Budget

For the minimum target machine (i3, 8 GB RAM, no discrete GPU):

| Component | Budget |
|---|---|
| GTK4 + libadwaita UI | ~60 MB |
| wgpu (Vulkan context, frame buffers) | ~80 MB |
| Skia canvas surfaces | ~40 MB |
| Timeline data model | ~10 MB (large project) |
| Media Pool thumbnails | ~30 MB (100 clips × 3 thumbnails) |
| Frame Cache (default) | 512 MB (user-configurable) |
| Proxy decode buffers | ~50 MB per active proxy stream |
| Audio decode buffers | ~20 MB |
| Plugin sandboxes | ~30 MB per loaded plugin |
| OS + other processes | ~2 GB headroom |
| **Total (2 active proxies, 1 plugin)** | **~870 MB** |

The frame cache is the primary memory lever. Users with more RAM should increase it for smoother
scrubbing. The Settings UI shows current cache usage and a slider to adjust.

---

## 11. Build System

### 11.1 Cargo Workspace

```toml
# Cargo.toml (workspace root)
[workspace]
members = [
    "crates/tempo-app",
    "crates/tempo-ui",
    "crates/tempo-timeline",
    "crates/tempo-media",
    "crates/tempo-render",
    "crates/tempo-audio",
    "crates/tempo-proxy",
    "crates/tempo-export",
    "crates/tempo-plugin",
    "crates/tempo-project",
    "crates/tempo-compute",
]
resolver = "2"

[workspace.dependencies]
# Pin versions here to keep all crates aligned
gtk4 = { version = "0.9", features = ["v4_14"] }
libadwaita = { version = "0.7", features = ["v1_6"] }
wgpu = "22"
ffmpeg-next = "7"
rusqlite = { version = "0.32", features = ["bundled"] }
# ... etc
```

### 11.2 Feature Flags

```toml
# In crates/tempo-render/Cargo.toml
[features]
default = ["vulkan", "vaapi"]
vulkan = ["wgpu/vulkan"]      # Primary backend
opengl = ["wgpu/gles"]        # Fallback for older hardware
vaapi = ["ffmpeg-next/vaapi"] # Intel hardware decode
cuda = ["ffmpeg-next/cuda"]   # NVIDIA hardware decode (optional)
```

`vaapi` is enabled by default but gracefully disabled at runtime if VAAPI is unavailable.

### 11.3 Shader Build

Slang shaders are compiled to WGSL at build time via a `build.rs` script in `tempo-render`:

```rust
// crates/tempo-render/build.rs
fn main() {
    let shader_dir = Path::new("shaders");
    for entry in fs::read_dir(shader_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension() == Some("slang") {
            compile_slang_to_wgsl(&path);
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
```

Output `.wgsl` files are included in the binary via `include_str!()`.

---

## 12. Error Handling Strategy

- **Library crates** (`tempo-timeline`, `tempo-media`, etc.): use `thiserror` with typed error enums. Never panic on bad input — return `Result`.
- **Application crates** (`tempo-app`, `tempo-ui`): use `anyhow` for error propagation. Display user-friendly error dialogs via `AdwAlertDialog` for recoverable errors.
- **Unrecoverable errors** (e.g., cannot initialize wgpu, cannot open display): log via `tracing`, show dialog explaining the issue, then exit cleanly.
- **Plugin errors**: caught at the WASM boundary. A crashing plugin logs the error and is disabled; the host app continues running.
- **Audio thread**: no panics ever. If decode fails, output silence. Log error.
- **Render thread**: if composition fails, display previous frame. Log error. Do not crash.

All errors are logged to `~/.local/share/tempo/tempo.log` (ring buffer, max 10 MB).

---

## 13. Testing Strategy

| Layer | Approach |
|---|---|
| `tempo-timeline` | Unit tests (100% coverage target) — pure Rust, no mocks needed |
| `tempo-media` | Tests with small sample clips made by `scripts/make_test_media.sh` in `target/test-media/` |
| `tempo-render` | Visual regression tests — render known timeline, compare PNG output |
| `tempo-audio` | Unit tests for mix math; integration test with PipeWire mock |
| `tempo-project` | Round-trip tests — write project, read back, compare |
| `tempo-plugin` | Plugin API contract tests — load test plugin, verify host calls |
| Full app | Manual test suite for each UI flow (documented in `docs/MANUAL_TESTS.md`) |

CI runs all unit and integration tests on every PR. Visual regression and manual tests run
on the nightly build.
