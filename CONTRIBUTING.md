# Contributing to Tempo

Thank you for helping. This page says what Tempo is for, what it will and won't accept, and how
to get a change merged. Please read the first section even if you read nothing else.

---

## ⚠️ The two rules that come before everything else

> [!IMPORTANT]
> **1. Tempo is made for weaker computers.**
> Every feature must run well on a cheap laptop: a dual-core processor, 4–8 GB of memory,
> integrated graphics, maybe a slow hard disk. That includes everything you add. If a feature
> only works well on a strong machine, it does not belong in Tempo.
>
> **2. Tempo is made for beginners.**
> It is a first step into video editing, and a path into DaVinci Resolve. Tempo copies
> Resolve's Edit page layout and keyboard shortcuts **on purpose**, so that what people learn
> here still works when they move on. A beginner must never feel lost.

**What follows from that:**

| Rule | In practice |
|---|---|
| **No bloat** | No feature "because Resolve has it". Things beginners don't need for cutting a video are out: colour wheels, node graphs, scopes, a Fairlight-style mixer, Fusion-style compositing, multicam, and so on. When unsure, leave it out. |
| **Don't change the interface or keys** | The layout and every shortcut follow DaVinci Resolve. Don't move panels, rename things Resolve names differently, or give a Resolve key a different job. Small polish is welcome; redesigns are not. |
| **Effects and titles are plugins** | New filters, transitions, title styles and timeline tools are added as **plugins**, not in the core (see [Plugins](#plugins)). The core stays small. |
| **Measure, don't guess** | A change that may cost speed or memory comes with numbers from `scripts/bench_playback.sh` (see [Performance](#performance-rules)). |

If a pull request breaks one of these rules, it will be asked to change, however good the code
is. Please open an issue to discuss a feature **before** you build it.

---

## What belongs where

| Kind of change | Where it goes | Examples |
|---|---|---|
| Making basic editing work, faster or more reliable | **Core** | bug fixes, smoother playback, lower memory, safer saving |
| Something a beginner needs to cut and finish a video, as Resolve's Edit page does it | **Core**, after an issue | trimming, fades, markers, export |
| A look, effect, transition, title style or extra tool | **Plugin** | Glow, Vignette, Wipe, lower-third titles, Remove Silence |
| Uploading somewhere | **Plugin** (upload target) | YouTube, a company server |
| Pro features beginners don't need | **Not accepted** | colour wheels, curves, scopes, node graphs, mixers, multicam, 3D |
| Features needing a strong GPU or lots of memory | **Not accepted** | real-time 4K effects without proxies, AI features in the core |

---

## Performance rules

Tempo is developed and measured on an Intel Core i3-1005G1 (2 cores) with integrated
graphics. These limits are checked in review:

- **Memory:** peak use stays under **1.5 GB**, with an absolute ceiling of 2 GB. No unbounded
  caches: anything kept in memory has a size limit.
- **Never block the interface thread.** File access, FFmpeg and database work run through
  `gio::spawn_blocking`, started from a `glib::MainContext::spawn_local` future.
- **Never block the sound.** The PipeWire callback must not allocate memory, lock a mutex or
  wait. It only reads from a lock-free queue and updates atomics.
- **No forced conversions.** Files must be usable as soon as they are imported. Proxies are
  made in the background, only for footage on the timeline, and pause while playing or
  exporting.
- **Don't redraw or decode more than needed.** Timeline redraws, frame copies and decodes
  are where weak machines struggle.
- **Graphics chip optional.** Hardware decoding and encoding (VA-API) are welcome as an
  option, but everything must still work on the processor alone.

If your change touches playback, decoding, drawing, proxies or export, run the benchmark
before and after, back to back on the same machine, and put both tables in the pull request.
Add a line to the experiment log in [docs/PERF.md](docs/PERF.md):

```bash
./scripts/bench_playback.sh                      # software decoding
HW=true ./scripts/bench_playback.sh              # with hardware decoding, if your machine has it
```

A change that doesn't measurably help is reverted, not kept "just in case".

---

## Interface and keyboard rules

- **Layout:** four screens (Loading, Project Manager, Edit, Export). There is no Cut page.
  [docs/UI_SPEC.md](docs/UI_SPEC.md) and [docs/VISUAL_DESIGN.md](docs/VISUAL_DESIGN.md)
  describe them.
- **Keys:** [docs/KEYBINDS.md](docs/KEYBINDS.md) is the source of truth, and it follows
  DaVinci Resolve's default keyboard file (`EX/DaVinci Resolve Keys.txt`). A test checks
  `assets/keybinds/keybinds.json` against that file.
  - A Resolve key may only do what it does in Resolve.
  - A key Resolve uses for a feature Tempo doesn't have yet stays **unbound**
    (KEYBINDS.md §14).
  - Tempo-only actions use keys Resolve leaves free (KEYBINDS.md §13).
- **Plain words.** Labels, tooltips and messages are for beginners: short, no jargon, no
  error codes.
- **No dead controls.** Every button does something, and no sample or placeholder data is shown.
- **Dark theme only.** Colours come from the tokens in `assets/style/tempo.css`.

---

## Plugins

Plugins keep the core small. Each one is a folder with a `plugin.toml`, and optionally a
`main.lua` script. Install it from **Plugins…** in the menu; plugins start switched off.
[docs/PLUGIN_SPEC.md](docs/PLUGIN_SPEC.md) is the full design, and
[assets/plugins/](assets/plugins/) has working examples.

**What a plugin can add today:**

| Section in `plugin.toml` | Adds | Code needed |
|---|---|---|
| `[[filter]]` | A filter in the Effects panel, with sliders in the Inspector | None. A list of `ops`: `saturation`, `brightness`, `contrast`, `blur`, `matrix`. A value can be a fixed number or `"$param"` |
| `[[command]]` | A timeline tool (like Remove Silence), one undo step | A Lua function in `main.lua` |
| `[[uploader]]` | A Share target on the Export page | A Lua function in `main.lua` |

A filter, for example:

```toml
[plugin]
id = "com.example.warm"            # reverse-DNS, unique
name = "Warm Look"
version = "1.0.0"
description = "A slightly warmer, brighter picture."

[[filter]]
id = "warm"
name = "Warm"
ops = [{ brightness = "$lift" }, { saturation = 1.1 }]

  [[filter.param]]
  id = "lift"
  name = "Brightness"
  min = 0.0
  max = 0.3
  default = 0.05
```

**Rules for plugins:**

- **Ask for as little as possible.** `[permissions]` defaults to nothing:
  - `timeline = "read" | "write"` for the timeline;
  - `network = ["host.name"]` for internet access;
  - `files` and `secrets` only if really needed.

  The user sees this list before switching the plugin on.
- **Stay inside the sandbox.** Scripts get Lua's `string`, `table`, `math` and `utf8` only:
  no file, OS or module access. Each call gets 64 MB of memory and 5 seconds of work. A script
  that fails changes nothing.
- **Be cheap.** Filters are drawn by the graphics card from the `ops` list, never pixel by
  pixel in a script. Keep the number of ops small.
- **Look native.** Plugins declare their settings; Tempo draws them. Plugins don't bring
  their own windows or widgets.
- **Ship the built-ins as examples.** A plugin that comes with Tempo lives in `assets/plugins/`,
  uses the same format, and has a test in `crates/tempo-plugin`.

**A kind of plugin that doesn't exist yet** (transitions, title templates and export presets
are planned in the spec): add the smallest *plugin point* to the core that lets a plugin
provide it, plus one built-in plugin that uses it. Don't build the feature itself into the
core. Open an issue first.

---

## Code rules

The full list is in [AGENTS.md](AGENTS.md) and [docs/AGENT_PROMPT.md](docs/AGENT_PROMPT.md).
The short version:

- **Crates keep their layers.**
  - `tempo-timeline` is the data model only: no interface, no async, no files.
  - `tempo-media`, `tempo-audio`, `tempo-proxy`, `tempo-export` and `tempo-project` know
    nothing about GTK.
  - Only `tempo-ui` builds the interface.
- **Every edit is undoable.** Timeline changes go through a `Command` and the `CommandLog`. A
  command either succeeds or leaves the timeline exactly as it was.
- **Never lose the user's work.**
  - Saving writes a temporary file and renames it.
  - Data that can't be read is an error, never silently replaced with defaults.
  - Database schema changes need a migration in `tempo-project`.
- **Errors:**
  - Libraries use `thiserror`.
  - No `.unwrap()` or `.expect()` outside tests.
  - Logging is `tracing`, never `println!`.
- **Small and plain.** The simplest code that does the whole job. No abstraction, option or
  dependency that nothing needs yet. A new crate dependency needs a reason in the pull request.
- **Comments say why**, not what.

---

## Before you open a pull request

Run all of these. CI runs the same checks, and `main` only accepts a pull request when they pass.

```bash
cargo clippy --workspace --all-targets -- -D warnings
./scripts/make_test_media.sh && cargo test --workspace
cargo run --bin tempo -- --smoke-test
# or everything at once:
./scripts/run_all_tests.sh
```

- **Write the test first** for new behaviour and for every bug fix: see it fail, then make it
  pass. Don't weaken, skip or delete a test to get green.
- **Check the interface yourself.** The scripted hooks help (see `TEMPO_ACTIONS` in
  `crates/tempo-ui/src/app.rs`):

  ```bash
  TEMPO_ACTIONS="dump;win.play-toggle" TEMPO_SCREENSHOT=shot.png cargo run --bin tempo -- my.tempo
  ```

- **One change per pull request,** on its own branch. Never push to `main`.
- **Commit messages** say what changed and why, in plain words. Link the issue.
- **Update the docs** you affect: `docs/KEYBINDS.md` for keys, `CHANGELOG.md` under
  *Unreleased*, `docs/PERF.md` for measured changes.

---

## Reporting a problem

Open an [issue](https://github.com/MSpider3/Tempo/issues) with:

- what you did, what you expected, and what happened;
- your computer (processor, memory, graphics) and Linux distribution;
- the Tempo version, and the log if there is one (run `tempo` from a terminal).

---

Tempo is free software under the GNU GPL version 3. By contributing, you agree that your
contribution is released under the same licence.
