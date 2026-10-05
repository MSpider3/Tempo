# Changelog

All notable changes to Tempo are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-05

The first release of the Rust rewrite. It replaces the earlier Python prototype.

### Added

- Project Manager: create, open and import projects; search; remove from the list.
- Edit page arranged like DaVinci Resolve's: Media Pool, viewer, Inspector, toolbar,
  timeline, and a page bar at the bottom.
- Timeline with video and audio tracks, markers, snapping, zoom, and a playhead.
- Editing: insert, overwrite, replace, append, place on top, move, trim, ripple trim, blade,
  razor, split, delete, ripple delete, nudge, move between tracks, copy, cut, paste, and
  turning a clip off and on.
- Waveforms on audio clips, worked out in the background and kept on disk.
- An output level meter beside the timeline.
- Cross dissolve between two clips (`Ctrl+T`), for picture and sound.
- Titles (Text and Lower Third) and fade in / fade out, in the viewer and in export, with
  an Effects panel to add them and Inspector sections to edit them.
- Selecting several clips (Ctrl+click, Select All); a clip and its sound are linked and
  move, trim and delete together; rolling a cut with Shift in Trim mode.
- Dual viewer, and a layout for vertical projects with the viewer in a tall column.
- Filters from plugins (Black and White, Saturation, Brightness, Contrast, Blur, Warm come
  built in): added from the Effects panel, adjusted in the Inspector, shown in the viewer
  and in export.
- Upload targets from plugins, listed under Share on a finished export, limited to the
  sites the plugin names.
- Inspector: drag left or right on a setting's name to change it live; the drag is one undo
  step.
- Project Manager: rename, duplicate, show in Files, move to Trash.
- Preferences: autosave interval, playback quality, automatic proxies, cache.
- Lua plugins: sandboxed scripts that add commands to the menu and change the timeline as
  one undo step. A Remove Silence plugin is built in.
- Optional hardware (VA-API) decoding and encoding, off by default, falling back to the
  processor when the graphics driver cannot be used.
- A dialog to find media files that have moved.
- A one-time hint for first-time users.
- Undo and redo for every edit, marker and track change.
- Viewer with a source mode for previewing Media Pool clips and marking In and Out.
- Preview sound through PipeWire. The picture follows the audio clock.
- Inspector: zoom, position, rotation, opacity, volume and pan.
- Background 540p proxies for footage above 720p or not in H.264. Export always uses the
  original files.
- Markers with name, colour and note. Named markers become chapters in the exported file
  and can be copied as a timestamp list, with YouTube's chapter rules checked.
- Export page: outputs chosen by frame shape (16:9, 16:9 4K, 9:16, 1:1, 4:5), a render
  queue, Fit or Fill when the shape differs, and Quick Export.
- Keyboard shortcuts matching DaVinci Resolve 20, checked by a test against a key file
  exported from Resolve.
- Autosave every two minutes, and an offer to restore unsaved work after a crash.
- Its own icon set, so the interface looks the same under any system icon theme.
- AppImage, Flatpak, `.rpm` and `.deb` packages with SHA-256 checksums. The release workflow
  installs each one and starts Tempo from it before publishing.

### Changed

- Decoding runs on a background thread; the interface never waits for it.
- Nothing is decoded while paused.
- Video is decoded with frame threading and scaled straight to the preview size.
- Scrubbing shows keyframes while dragging and the exact frame when the pointer rests.
- The viewer is drawn by GTK on the graphics chip, replacing a path that copied each frame
  to the graphics chip and back.
- Export is done by FFmpeg from the original files, replacing a path that produced blank
  frames.

### Removed

- The Cut page; editing happens on the Edit page.
- The wgpu renderer, the WebAssembly plugin host and the Python helper, none of which the
  editor used any more.

[Unreleased]: https://github.com/MSpider3/Tempo/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/MSpider3/Tempo/releases/tag/v0.1.0
