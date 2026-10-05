# Roadmap — Tempo

**Version:** 2.0 (2026-10-03)
**Replaces:** version 1.0, which was organised around a Cut page, an Edit page and WASM plugins.

---

## How to read this

- Phases are in order. A phase is finished only when every check in its "Done when" list
  passes **on the reference machine** (Intel Core i3-6100, 8 GB RAM, integrated graphics).
- Smooth playback comes before features. If a later phase makes playback worse, stop and fix.
- No time estimates. Each phase is done when its checks pass.

### Where the code is today (updated 2026-10-05)

Boxes ticked below are built and were checked by tests or by running the app. A box is left
open if any part of its line is missing.

**Measured** on the development machine, an Intel Core i3-1005G1 (2 cores, 4 threads,
integrated graphics), release build, with a 2400 × 1080 HEVC variable-frame-rate phone
recording:

| Measure | Result |
|---|---|
| Start to window | 0.7 s |
| Probe a file on import | about 55 ms |
| Paused, project open | 0.6 % of one core, 193 MB |
| Playing the original HEVC file at Half quality | 124 % of one core (about a third of the machine), 220 MB, 6 dropped frames in 25 s |
| Playing its 540p proxy | 25 % of one core, 186 MB, no dropped frames |
| Making the proxy | 51 s for a 135 s clip, in the background |
| Program size | 4.4 MB |

This is a newer i3 than the i3-6100 reference machine, and it has more memory, so the
reference machine still needs its own run.

**Checked without a person at the keyboard.** The editing gestures were run through the
real mouse handlers by scripted steps (click, Ctrl+click, drag, trim, ripple, roll, blade,
insert, overwrite, append, place on top, copy and paste, delete, undo) and the resulting
timeline compared with what was expected. Three bugs were found and fixed that way. A person
has still not sat down and used it.

**Main gaps**

| Gap | Note |
|---|---|
| Hardware decode and encode | Built with automatic fallback, but off by default: the development machine's VA-API driver does not start, so only the fallback could be tested |
| Cross dissolve | Not built. Fade in and fade out are |
| Plugins | Lua commands work (timeline, markers, loudness). Not built: shader effects, title and export-preset packs, file and network access, upload targets, the `.tempo-plugin` archive (plugins install from a folder) |
| Inspector | Values change in steps, one undo step each; no live drag |
| Project Manager | No rename, duplicate or move to trash |
| Project Settings window, duplicate clip, track add/remove | Not built |
| Export | Uses the `ffmpeg` program, not the libraries. This is deliberate: it is robust and keeps Tempo small |
| Proxies | Made for all heavy footage on import, not only when playback drops frames |
| Flatpak | A manifest exists in `packaging/` but has not been built |
| A/V sync | Picture follows the audio clock; the 40 ms target has not been measured |
| Roll trim | Uses Shift in Trim mode, not Resolve's click-on-the-cut |

---

## Phase 1 — One clip plays properly

**Goal:** a single clip plays with sound, smoothly, with the UI thread doing no media work.

- [x] Playback worker thread owns decoding. The UI only receives finished frames.
- [x] No decoding when the playhead has not moved.
- [x] Frame-threaded software decode. Separate demuxers for audio and video.
- [x] Real VA-API decode with automatic fallback to software.
- [ ] Viewer shows frames without a round trip through CPU memory: `GdkDmabufTexture` in
      `GtkGraphicsOffload` when hardware decode is active, YUV upload otherwise.
- [x] Real PipeWire output. Audio is the clock; video follows it.
- [ ] Look-ahead that fills a small frame cache (frames kept in YUV).
- [x] Scrubbing: keyframes while dragging, exact frame on pause or release.
- [x] `Space`, `J` `K` `L`, arrow keys, `Home`, `End`.
- [ ] Variable-frame-rate files play in sync.

**Done when**
1. 1080p30 H.264 plays for 10 minutes with zero dropped frames and under 35 % CPU.
2. Audio and video stay within 40 ms of each other over 30 minutes.
3. Paused with a project open, CPU use is under 2 %.
4. Scrubbing shows a picture within 100 ms of the pointer.
5. A 4K HEVC phone clip opens and plays at Quarter quality without freezing the UI.

---

## Phase 2 — The Edit page

**Goal:** a real editor. Someone can make a rough cut and save it.

**Shell**
- [x] Root stack: Loading, Project Manager, project view. Page bar at the bottom.
- [x] Design tokens in `tempo.css`; forced dark scheme.
- [x] Loading screen driven by real start-up steps.
- [ ] Project Manager: grid, search, new, open, import, rename, duplicate, trash.

**Timeline**
- [x] `TimelineWidget`: one custom-drawn, scrollable widget. Ruler, tracks, clips, playhead.
- [x] Remove the button-based timeline and all sample clips.
- [x] Selection, Trim and Blade modes; move, trim, ripple, roll, split.
- [x] Insert, Overwrite, Replace, Place on Top, Append.
- [x] Snapping; linked clips and linked selection.
- [ ] Copy, cut, paste, duplicate, enable/disable clip, lock track.
- [x] **Every change goes through `CommandLog`.** Undo and redo.
- [ ] Zoom and scroll with Resolve's mouse gestures.

**Media and viewer**
- [ ] Media Pool on `GtkGridView`; background probing; thumbnails cached on disk.
- [x] Source mode in the viewer with In/Out; `Q` to switch.
- [x] Multi-track compositing for `V1`–`V4`; audio mixing for `A1`–`A4`.
- [x] Waveforms, generated in the background and cached on disk.
- [x] Audio meter.

**Markers**
- [ ] Add, name, colour, move, delete (all undoable). Jump to next and previous.

**Project**
- [x] Save and load on a worker thread. Autosave. Crash recovery prompt.
- [x] Missing-media dialog with relink.

**Keys**
- [x] `keybinds.json` matches `KEYBINDS.md`. Tests pass: no duplicate keys, no reserved keys, and every Resolve-named binding equals the key in `EX/DaVinci Resolve Keys.txt`.

**Done when**
1. Ten mixed clips (H.264 and HEVC, 720p and 1080p, 24 and 30 fps) are cut into a
   three-minute video without touching the mouse for transport.
2. Playing a two-video, two-audio timeline drops no frames.
3. Thirty edits followed by thirty undos return an identical timeline.
4. Killing the app mid-edit and reopening recovers the work.
5. Memory stays under 1.5 GB during that session.
6. The whole Edit page is usable at 1366 × 768.

---

## Phase 3 — Inspector, titles, transitions, export

**Goal:** a finished video can be made and delivered.

- [ ] Inspector: Transform, Composite, Audio, Title, Transition, Marker sections.
- [ ] Live preview while dragging a value; one undo step per drag.
- [ ] Cross Dissolve and Dip to Colour; fade handles on clips; audio fades.
- [x] Title clips: Text and Lower Third.
- [x] Vertical-project layout.
- [ ] Automatic small proxies, made only when playback drops frames, paused during playback.
- [x] Export page: presets, settings, viewer with range, render queue.
- [ ] Export engine in-process (no `ffmpeg` program): timeline video and mixed audio,
      correct colour tagging, hardware encode when available.
- [x] Quick Export dialog. Activity area in the page bar.
- [x] Chapters from markers: written into the file, and Copy chapters with the YouTube checks.
- [x] Share: open the platform's upload page and reveal the file.
- [x] Marker dialog (name, colour, note).
- [ ] Preferences and Project Settings dialogs. First-use hints.

**Done when**
1. A three-minute video with two titles, three transitions and music exports and plays
   correctly in another player, with sound in sync.
2. Three queued jobs (1080p, Vertical, Small File) render one after another while a clip is
   being trimmed on the Edit page, with no UI freeze.
3. The exported MP4 shows chapters in a player that supports them, and the copied chapter
   list is accepted by YouTube.
4. Cancelling an export stops it within one second and removes the partial file.
5. Ten minutes of 1080p exports in under 15 minutes.
6. Memory stays under 2 GB throughout.

---

## Phase 4 — Plugins

**Goal:** features can be added without touching the core.

**Tier 1 — data plugins**
- [ ] Bundle format, manifest parsing, install checks.
- [ ] Shader effects with declared parameters; shader validation at install.
- [ ] Inspector sections generated from parameters.
- [ ] Title templates and export presets from plugins.
- [ ] Plugins dialog.
- [ ] Built-in packs: transitions, filters, titles.

**Tier 2 — Lua**
- [x] Sandboxed Lua states with memory and run-time limits.
- [ ] `tempo` API: project, timeline read and write, markers, media, ui.
- [x] Commands in context menus; one undo step per run.
- [x] Built-in example: Remove Silence.
- [x] Remove `wasmtime` and `tempo-compute` from the build.

**Done when**
1. A transition plugin made of one manifest and one shader installs, appears in Effects,
   shows its controls in the Inspector, and renders in both viewer and export.
2. A script with an endless loop is stopped, the user is told, and the editor keeps working.
3. A script command that makes 50 changes is undone with one `Ctrl+Z`.
4. A project that uses a disabled plugin still opens.
5. Three enabled plugins add less than 30 MB of memory.

---

## Phase 5 — Release

- [ ] Eight-hour editing session on the reference machine with no crash.
- [ ] Profile and fix the three largest CPU and memory costs.
- [ ] Keyboard-only and screen-reader pass over every screen.
- [ ] Side-by-side check of every screen against the Resolve 20 screenshots in `EX/DVR_UI/`.
- [ ] Confirm by hand the three mouse gestures in `KEYBINDS.md §8` (not in the key file).
- [ ] Flatpak, desktop file, AppStream data, icon.
- [ ] User guide and plugin guide.

**Done when**
1. Three beginners, given no instructions, import clips, make a cut, add a title and export.
2. A Resolve user can operate Tempo without looking up a key.
3. Start to Project Manager takes under 2 seconds.

---

## After 1.0

| Feature | Form |
|---|---|
| Direct upload — YouTube first | Upload plugin (`network`, `secrets`, OAuth) |
| Keyframes | New plugin point; separate bottom panel as in Resolve 20+ |
| EDL / FCPXML export to Resolve | Script plugin |
| Speed changes, slip/slide, dynamic trim | Core, on their Resolve keys |
| Subtitles | Core track type plus importer plugins |
| Noise reduction, auto subtitles | Optional AI helper |
| Remappable keys | Preferences |
| Translations | Community |

---

## Definition of done, for any feature

1. Works on the reference machine.
2. Does no slow work on the UI thread.
3. Can be undone, if it changes the project.
4. Has a test.
5. Has a tooltip, an accessible name and, if it has a key, an entry in `keybinds.json`.
6. Matches `VISUAL_DESIGN.md`.
7. Fails with a message the user can understand.
