# Working Rules for Contributors and AI Agents — Tempo

**Version:** 3.0 (2026-10-03)
**Replaces:** version 2.1 (Cut/Edit/Export switcher, web-prototype phase, WASM plugins).

Read this before writing code. Then read the document for the area you are working on.

---

## 1. What you are building

Tempo is a light video editor for Linux that looks and works like DaVinci Resolve's Edit page,
with far fewer features. It must run smoothly on an Intel Core i3 with 8 GB of RAM and no
separate graphics card.

Four screens: **Loading**, **Project Manager**, **Edit page**, **Export page**.

| Question | Document |
|---|---|
| What is in scope? | `PRD.md` |
| How should it look? | `VISUAL_DESIGN.md` |
| Which widgets, what behaviour? | `UI_SPEC.md` |
| Which key? | `KEYBINDS.md` |
| How do plugins work? | `PLUGIN_SPEC.md` |
| What do I build next? | `ROADMAP.md` |

The engine documents (`ARCHITECTURE.md`, `SYSTEM_DESIGN.md`, `MEDIA_ENGINE.md`,
`DATABASE_SCHEMA.md`) were written earlier. Each has a note at the top listing what is out of
date. If they disagree with the documents above, the documents above are right.

---

## 2. Rules

### 2.1 Threads

1. **The GTK thread does no slow work.** No file reading, FFmpeg calls, database writes or
   network calls. If it could take more than a millisecond, it goes on a worker.
2. Workers report back through an `async-channel` read by a future started with
   `glib::MainContext::spawn_local`. Do not use `glib::MainContext::channel`; it was removed.
3. **The audio callback never allocates, locks or does I/O.** It reads atomics and lock-free
   buffers only.
4. Drive the viewer and timecode from `add_tick_callback`, not from a timer. Redraw only when
   something changed.

### 2.2 State

5. **Every timeline change is a command** sent through `tempo_timeline::CommandLog`. This
   includes markers, track settings and plugin actions. A drag is one command, committed on
   release.
6. Workers get a snapshot of the timeline, not a lock on the live one.

### 2.3 Layers

7. `tempo-timeline` depends on nothing else in the workspace and has no GTK, async or FFmpeg
   dependencies.
8. `tempo-ui` depends on the engine crates; never the other way round. `tempo-export` and
   `tempo-media` have no GTK dependency.
9. If the UI needs something the engine lacks, add it to the engine crate. Do not fake it in
   the UI.

### 2.4 Honest interface

10. **No sample data.** No hard-coded clips, levels, names or progress values. An empty
    project looks empty.
11. **No dead controls.** Do not draw a button, slider or meter until it is connected to
    something that works.
12. **No colour values in Rust.** Colours come from `assets/style/tempo.css`.

### 2.5 Keys

13. Bindings come only from `assets/keybinds/keybinds.json`, which must match `KEYBINDS.md`.
14. Never assign a DaVinci Resolve default key to a different action.

### 2.6 Code

15. Library crates return typed errors (`thiserror`). The app crate uses `anyhow`.
    No `.unwrap()` or `.expect()` outside tests.
16. Log with `tracing`. No `println!`.
17. A change to the project file needs a migration in `tempo-project`.
18. Do not commit to `main`.

---

## 3. Before you start

```bash
cargo check --workspace
cargo test --workspace
```

Both must pass before and after your change. If they fail before you start, say so and stop.

---

## 4. While you work

- Build the smallest thing that satisfies the roadmap item. Do not add options, panels or
  settings that the documents do not ask for.
- When a document is unclear or two documents disagree, ask. Do not guess and do not pick the
  larger feature.
- When the documents are wrong about the code (they sometimes are), fix the document in the
  same change.

---

## 5. Checking your work

**Performance** — measure on the reference machine, or say clearly that you could not.

```bash
# CPU while paused should be near zero; while playing 1080p, under about 35 %
pidstat -p $(pgrep -x tempo) 1

# Memory must stay under 2 GB
heaptrack target/release/tempo

# Hardware decode available?
vainfo

# Audio stream connected, no underruns
pw-top
```

**Interface** — run the app and look at it. Compare against `VISUAL_DESIGN.md` at
1366 × 768 and at a larger size. For a headless check, use a Wayland compositor in headless
mode rather than Xvfb, because Tempo is Wayland-first:

```bash
weston --backend=headless --width=1366 --height=768 &
WAYLAND_DISPLAY=wayland-1 cargo run --bin tempo
```

**Keys** — `cargo test -p tempo-ui keybinds` checks for duplicates and for keys that must
stay unbound.

**Undo** — every new command has a test in `tempo-timeline` that applies it, undoes it and
compares the timeline with the original.

---

## 6. Reporting

When you finish, state plainly:

- what you built;
- what you tested, and how;
- what you did **not** test (for example "not run on the reference machine");
- anything in the documents you found to be wrong.

Do not describe work as complete if a check failed or was skipped.
