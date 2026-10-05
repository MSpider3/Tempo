# AGENTS.md — Tempo

## Stack

- **Rust 2021** (9-crate workspace)
- **UI:** GTK 4 + libadwaita (dark only; style in `assets/style/tempo.css`)
- **Media:** ffmpeg-next 9 for decoding; the `ffmpeg` program for export and proxies
- **Audio:** PipeWire 0.10
- **Plugins:** Lua 5.4 (mlua 0.10), sandboxed
- **Project file:** SQLite (rusqlite 0.32)

## Build / Test / Run

```bash
cargo check --workspace
./scripts/make_test_media.sh                 # sample clips for the media tests
cargo test --workspace
cargo test -p tempo-timeline -- --nocapture
./scripts/run_all_tests.sh
cargo clippy --workspace --all-targets -- -D warnings
cargo run --bin tempo
cargo run --bin tempo -- --smoke-test        # start, then quit
# Scripted checks: see TEMPO_ACTIONS in crates/tempo-ui/src/app.rs
```

## Code Conventions

- **Crate isolation:** Follow layer hierarchy (`tempo-timeline` has no UI/async deps).
- **Errors:** `thiserror` in libraries; `anyhow` in `tempo-app`. No `.unwrap()` in production.
- **State:** Timeline mutations MUST go through `CommandLog` (undo/redo).
- **Thread rules:** No blocking I/O, FFmpeg, or DB calls on GTK thread; run them with `gio::spawn_blocking` from a `spawn_local` future.
- **Audio real-time:** PipeWire callbacks must never allocate or lock mutexes (`AtomicI64` playhead).
- **Logging:** Use `tracing` macros (`info!`, `error!`), never `println!`.

## Structure

```
crates/
  tempo-timeline/   # Data model & undoable commands
  tempo-media/      # Probe, video and audio decoding
  tempo-audio/      # PipeWire output
  tempo-proxy/      # Background proxy files
  tempo-export/     # Export through ffmpeg, chapters
  tempo-plugin/     # Lua plugin host
  tempo-project/    # SQLite persistence
  tempo-ui/         # GTK4 / libadwaita interface
  tempo-app/        # App entry point
```

## Guardrails

- **DO NOT** block GTK thread with I/O, FFmpeg, or DB writes.
- **DO NOT** change DB schema without migrations in `tempo-project`.
- **DO NOT** break DaVinci Resolve keybind parity. `docs/KEYBINDS.md` is the source; never reuse a Resolve key for another action.
- **DO NOT** add sample data or unconnected controls to the UI.
- **DO NOT** exceed 2 GB RAM or force eager transcoding.
- **DO NOT** commit directly to `main`. It is protected: changes go through a pull request and CI must pass.

## Full Context

Start with `docs/README.md` and `docs/AGENT_PROMPT.md`. Screens: Loading, Project Manager, Edit page, Export page (no Cut page). See also `PROJECT.md` at repo root or in vault at `/run/media/mehulgolecha/Extra Volume/Projects/Project Documents/Tempo (Video Editor)/PROJECT.md`.
