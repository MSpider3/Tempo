# AGENTS.md — Tempo

## Stack

- **Rust 2021** (11-crate workspace)
- **UI:** GTK 4 + libadwaita
- **Rendering:** wgpu 22, Skia (skia-safe 0.75), Slang → WGSL
- **Media/Audio:** ffmpeg-next 9 (VAAPI), PipeWire 0.10, rubato 0.15
- **Plugins/DB:** Lua 5.4 (mlua 0.10), SQLite (rusqlite 0.32). wasmtime is being removed (see `docs/PLUGIN_SPEC.md`)
- **Concurrency:** rayon 1.10, tokio 1.40, crossbeam 0.8

## Build / Test / Run

```bash
cargo check --workspace
cargo test --workspace
cargo test -p tempo-timeline -- --nocapture
./scripts/run_all_tests.sh
cargo run --bin tempo
cargo run --bin tempo -- --smoke-test        # headless smoke
```

## Code Conventions

- **Crate isolation:** Follow layer hierarchy (`tempo-timeline` has no UI/async deps).
- **Errors:** `thiserror` in libraries; `anyhow` in `tempo-app`. No `.unwrap()` in production.
- **State:** Timeline mutations MUST go through `CommandLog` (undo/redo).
- **Thread rules:** No blocking I/O, FFmpeg, or DB calls on GTK thread; dispatch updates via `glib::MainContext::invoke`.
- **Audio real-time:** PipeWire callbacks must never allocate or lock mutexes (`AtomicI64` playhead).
- **Logging:** Use `tracing` macros (`info!`, `error!`), never `println!`.

## Structure

```
crates/
  tempo-timeline/   # Data model & undo/redo
  tempo-media/      # FFmpeg decode & VAAPI
  tempo-render/     # wgpu compositor & Skia
  tempo-audio/      # PipeWire & waveforms
  tempo-proxy/      # Proxy transcode
  tempo-export/     # Export & EDL
  tempo-plugin/     # Lua plugin host (wasmtime being removed)
  tempo-project/    # SQLite persistence
  tempo-compute/    # Python IPC client (deferred, not in 1.0)
  tempo-ui/         # GTK4 / Libadwaita
  tempo-app/        # App entry point
```

## Guardrails

- **DO NOT** block GTK thread with I/O, FFmpeg, or DB writes.
- **DO NOT** put Python in playback, render, or export paths.
- **DO NOT** change DB schema without migrations in `tempo-project`.
- **DO NOT** break DaVinci Resolve keybind parity. `docs/KEYBINDS.md` is the source; never reuse a Resolve key for another action.
- **DO NOT** add sample data or unconnected controls to the UI.
- **DO NOT** exceed 2 GB RAM or force eager transcoding.
- **DO NOT** commit directly to `main`.

## Full Context

Start with `docs/README.md` and `docs/AGENT_PROMPT.md`. Screens: Loading, Project Manager, Edit page, Export page (no Cut page). See also `PROJECT.md` at repo root or in vault at `/run/media/mehulgolecha/Extra Volume/Projects/Project Documents/Tempo (Video Editor)/PROJECT.md`.
