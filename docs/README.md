# Tempo — Documentation Index

**Tempo** is a light, beginner-friendly video editor for Linux. It looks and works like
DaVinci Resolve's Edit page, so the keys and layout a beginner learns carry straight over to
Resolve later. It runs on an ordinary laptop without a graphics card and never converts
footage on import.

---

## At a glance

| | |
|---|---|
| Version | 0.2.0 alpha (2026-10-21) |
| Language | Rust |
| UI | GTK 4 + libadwaita, dark only |
| Screens | Loading, Project Manager, Edit page, Export page |
| Media | FFmpeg; VA-API hardware decode optional (off by default); proxies made with VA-API when it works |
| Audio | PipeWire |
| Plugins | Lua 5.4 (`mlua`); filters as lists of `ops` (WGSL shader effects are planned) |
| Project file | SQLite, one `.tempo` file |
| Reference machine | Intel Core i3-6100, 8 GB RAM, integrated graphics |
| Reference for layout and keys | DaVinci Resolve 20 — screenshots in `EX/DVR_UI/`, key export in `EX/DaVinci Resolve Keys.txt` |
| Licence | GPL-3.0 |

---

## Documents

**Product and interface — current (rewritten 2026-10-03)**

| File | What it covers |
|---|---|
| `PRD.md` | What Tempo is, who it is for, what is in and out of the core |
| `VISUAL_DESIGN.md` | How every screen looks, with colours sampled from DaVinci Resolve 20 |
| `UI_SPEC.md` | Which widgets build each screen and how they behave |
| `KEYBINDS.md` | Every shortcut, checked against the key file exported from DaVinci Resolve 20 |
| `PLUGIN_SPEC.md` | The Lua plugin system; the note at its top says what is built today |
| `ROADMAP.md` | Order of work, and what is built and still open |
| `PERF.md` | Playback benchmark results (`scripts/bench_playback.sh`) and the experiment log |
| `AGENT_PROMPT.md` | Working rules for anyone (or any AI agent) writing code |

**Engine — written for version 1.0, partly out of date**

| File | What it covers | Status |
|---|---|---|
| `ARCHITECTURE.md` | Crates, threads, memory | Plugin and UI parts are superseded; see the note at its top |
| `SYSTEM_DESIGN.md` | Data types, commands, interfaces | Plugin API superseded; see note |
| `MEDIA_ENGINE.md` | Decoding, cache, proxies | Known design problems listed in its top note |
| `DATABASE_SCHEMA.md` | Project file tables, loading rules, backups | Known design problems listed in its top note; updated for 0.2.0 |

Where an engine document disagrees with a product or interface document, the product or
interface document is right.

**Outside `docs/`:** [`CONTRIBUTING.md`](../CONTRIBUTING.md) says what Tempo accepts and how
to get a change merged; [`CHANGELOG.md`](../CHANGELOG.md) lists what each version changed.

The previous versions of all documents are kept in `EX/docs_backup_2026-10-03/`.

---

## Rules that do not bend

1. **Nothing slow on the UI thread.** No file reading, decoding, saving or network calls.
2. **No conversion on import.** Files are usable at once. Proxies are optional and made in
   the background.
3. **Resolve's keys.** Any action Tempo shares with Resolve uses Resolve's default key.
   A Resolve key is never reused for something else.
4. **Under 2 GB of memory** while editing on the reference machine.
5. **Every edit can be undone.** Timeline changes go through the command log.
6. **Small core.** New features are plugins unless a beginner needs them on day one.
7. **No fake UI.** No sample clips, no controls that are not connected to a working feature.
