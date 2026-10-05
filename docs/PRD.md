# Product Requirements — Tempo

**Version:** 2.0 (2026-10-03)
**Replaces:** version 1.0 (Cut page + Edit page, WASM plugins).

---

## 1. Problem

DaVinci Resolve is one of the best free editors and a good place for a beginner to end up.
But it needs a lot of memory, and on Linux it will not run without a supported graphics card.
People with an ordinary laptop cannot start there.

The lighter Linux editors have a different problem: many convert or pre-process footage
before editing, which is slow and fills memory on a CPU-only machine, and what you learn in
them does not carry over to Resolve.

## 2. What Tempo is

> **The editor you use before DaVinci Resolve — and keep for quick videos.**

A light, fast video editor for Linux that looks and works like Resolve's Edit page, with only
the features a beginner needs.

- **Same keys, same layout.** A Tempo user who later opens Resolve already knows where things
  are and which keys to press.
- **Runs on a weak machine.** No graphics card needed. Files are never converted on import.
- **Simple.** Import, cut, add a title, export, post. Extra features come as plugins.

Tempo is not a Resolve clone. It copies Resolve's arrangement and shortcuts on purpose, and
leaves out everything else.

## 3. Users

**The beginner on an ordinary laptop.** Intel Core i3/i5 with integrated graphics, 8 GB RAM,
Linux. Has edited little or nothing. Wants to make YouTube videos, school projects, short
clips. Plans to move to Resolve when they have the hardware.

**The casual editor.** Wants to trim a video, add a title and music, and post it to social
media without learning a professional tool.

**The Resolve user on a second machine.** Knows Resolve; wants something light for quick cuts
that uses the same keys.

Not for: colourists, motion-graphics artists, audio engineers.

## 4. Targets

Measured on the reference machine: Intel Core i3-6100, 8 GB RAM, integrated graphics.

| Measure | Target |
|---|---|
| Start to Project Manager | under 2 seconds |
| Import to playable | under 1 second per file, no conversion |
| 1080p H.264 playback | 30 fps with no dropped frames |
| Scrubbing | picture follows the pointer within 100 ms |
| Memory, project open and idle | under 400 MB |
| Memory while editing | under 2 GB |
| Export, 10 minutes of 1080p | under 15 minutes |
| Resolve key parity | every shared action uses Resolve's default key |

## 5. Platform

| | |
|---|---|
| OS | Linux, Wayland first, X11 supported |
| CPU | x86_64, Intel 6th generation (Skylake) or newer, or AMD equivalent |
| RAM | 8 GB |
| Graphics | Integrated graphics with Vulkan or OpenGL ES 3. No separate graphics card needed. Hardware video decode (VA-API) is used when present |
| Delivery | Flatpak; also builds from source |

---

## 6. Screens

Tempo has four screens (details in `UI_SPEC.md` and `VISUAL_DESIGN.md`).

1. **Loading** — shown while the app starts.
2. **Project Manager** — create, open and manage projects.
3. **Edit page** — all editing. Arranged like Resolve's Edit page.
4. **Export page** — render one or several files; share them. Arranged like Resolve's
   Deliver page.

There is no separate Cut page.

---

## 7. Core features (1.0)

### 7.1 Projects
- Project Manager with thumbnails, search, rename, duplicate, move to trash.
- New project: name, shape (landscape, vertical, square), resolution, frame rate.
- "Match project to first clip" prompt.
- One project is one `.tempo` file. Source media is never copied into it.
- Autosave (default every 2 minutes) and recovery after a crash.
- Missing-media detection with relink.

### 7.2 Media
- Import by `Ctrl+I` or drag and drop. No conversion, no waiting.
- Video: H.264, H.265, VP9, AV1, ProRes, DNxHD, MPEG-2, MPEG-4 in MP4, MOV, MKV, WebM, AVI, TS.
- Audio: AAC, MP3, FLAC, WAV, Opus, Vorbis.
- Images: JPEG, PNG, WebP.
- Mixed resolutions and frame rates in one project. Variable-frame-rate phone footage plays
  correctly.
- Media Pool with thumbnails, search, grid and list views. Bins are optional.

### 7.3 Editing
- Tracks: `V1`, `V2`, `A1`, `A2` by default; up to four video and four audio.
- Edit modes: Selection, Trim, Blade.
- Insert, Overwrite, Replace, Place on Top, Append.
- Move, trim, ripple trim, roll, split, delete, ripple delete, copy, paste, duplicate.
- Snapping. Linked video and audio, with a linked-selection switch.
- Enable/disable clip. Lock track. Hide video track, mute audio track.
- Source clips can be previewed and given In/Out points before placing.
- Undo and redo for every change (100 steps).

### 7.4 Picture and sound
- Transform per clip: zoom, position, rotation. Opacity.
- Transitions: Cross Dissolve, Dip to Colour. Fade in and fade out handles on every clip.
- Titles: Text and Lower Third, with font, size, colour, background and position.
- Volume and pan per clip. Audio fades. Waveforms. Output level meter.

### 7.5 Playback
- `J` `K` `L`, frame step, loop, play In to Out.
- Full, Half and Quarter playback quality.
- Automatic small-size proxies for footage the machine cannot play smoothly, made in the
  background and only when needed. Export always uses the original files.
- Full-screen viewer.

### 7.6 Markers and chapters
- Add a marker at the playhead with `M`; name, colour and note.
- Jump between markers.
- **Named markers become chapters.** On export they can be written into the video file and
  copied as a timestamp list for a video description (`00:00 Intro`, `01:25 …`).

### 7.7 Export
- Export page with formats chosen by frame ratio, each labelled with what it suits:
  16:9 (YouTube), 9:16 (TikTok, Reels, Shorts), 1:1 and 4:5 (Instagram), Custom.
  No platform logos.
- Render queue: several outputs of the same project in different sizes and formats, rendered
  one after another while editing continues.
- Export the whole timeline or the In–Out range.
- Quick Export dialog from the Edit page.
- **Share:** open the platform's upload page with the file ready. Direct upload to an account
  is provided by upload plugins after 1.0 (`PLUGIN_SPEC.md §7.5`).

### 7.8 Plugins
- Lua-based plugin system (`PLUGIN_SPEC.md`).
- Plugins add transitions, filters, title styles, export presets, timeline commands and
  upload targets.
- Plugins dialog: install, enable, see permissions, remove.

### 7.9 Settings
- Font scale, autosave interval, default playback quality, hardware decoding, automatic
  proxies, cache size and location.
- Shortcuts are shown but not remappable in 1.0.

---

## 8. Not in the core

| Feature | Where it goes |
|---|---|
| Keyframe animation | Plugin point after 1.0 |
| Colour correction and filters | Filter plugins |
| More transitions and title styles | Plugins (a starter pack ships with the app) |
| Direct upload to YouTube, Instagram, TikTok | Upload plugins after 1.0 |
| Remove silence, other timeline automation | Script plugins |
| EDL / FCPXML export for moving a project to Resolve | Script plugin after 1.0 |
| Noise reduction, auto subtitles (AI) | Optional helper after 1.0 |
| Speed changes, slip and slide, dynamic trim | After 1.0; their Resolve keys are reserved |
| Multicam, colour grading, audio mixing, node compositing | Never; use Resolve |

---

## 9. User flows

**First video**

```
Open Tempo → Project Manager → New Project
  → drop clips into the Media Pool
  → drag clips to the timeline
  → cut (B, or Ctrl+B at the playhead), delete (Shift+Backspace), drag to reorder
  → drag a title from Effects
  → Shift+8 → pick "YouTube 1080p" → Add to Render Queue → Render All
  → Share… → Open YouTube upload page
```

**Video with chapters**

```
While editing: press M at each new section, type a name
  → Export with "Chapters from markers" on
  → Copy chapters → paste into the video description
```

**Slow footage**

```
Import 4K phone footage → it appears and plays at once, at reduced quality if needed
  → Tempo notices dropped frames → makes a small proxy in the background
  → playback becomes smooth; a PROXY badge shows in the viewer
  → export uses the original 4K file
```

**After a crash**

```
Open the project → "Restore unsaved changes from 14:32?" → Restore
```

---

## 10. Accessibility and language

- Every action reachable from the keyboard.
- Every control has an accessible name; custom-drawn widgets expose their contents.
- Text meets 4.5:1 contrast; status never relies on colour alone.
- The UI scales with the system text size.
- All text is marked for translation. English only at launch.
