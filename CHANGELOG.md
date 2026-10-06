# Changelog

All notable changes to Tempo are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.1] - 2026-10-06

Alpha. Fixes for the problems found in the first hands-on test of 0.1.0, and for more of
the same kind found by reading the code afterwards.

### Added

- Right-click menus on timeline clips (cut, copy, paste, split, delete, unlink, turn off,
  fades), on gaps (Close Gap) and in the Media Pool (insert, overwrite, add to end, open,
  show in Files, remove, import).
- An empty stretch between clips can be clicked and deleted to close it.
- A **Preview** button above the viewer: Full (original file), Half or Quarter. The
  choice is remembered. Half is the default.
- A **Back to Timeline** button while a Media Pool clip is in the viewer.
- Keys from DaVinci Resolve that were documented but not bound: `Shift+L`, `Shift+J`
  (step the speed up and down), `Shift+K` (half speed), `Ctrl+Shift+X` (cut and close the
  gap), `Shift+V` (select the clip at the playhead), `Ctrl+Shift+[` and `Ctrl+Shift+]`
  (trim to the playhead and close the gap), `Alt+Shift+D` and `Alt+Shift+G` (fade to the
  playhead), `Ctrl+6` (Effects). `Ctrl+Shift+E` now starts a Quick Export.
- In and Out marks are drawn as flags in the timeline ruler and on the viewer's scrub bar.
- Clips whose file is missing are marked "Media offline" on the timeline, and
  **Relink Missing Media…** is in the menu.
- A small mark on clips that are linked to another clip.

### Changed

- Export renders a long timeline in sections and joins them. Memory use no longer grows
  with the number of cuts (measured: 574 MB instead of 990 MB for 20 cuts, and flat from
  there). Export also runs at a lower priority, so the computer stays usable.
- Making a proxy uses about a third of a four-thread processor instead of nearly all of
  it, and waits while the timeline is playing or an export is running.
- Quick Export opens the Export page, so the job and its progress are in view.
- Insert (`F9`) and Delete-and-close-gap move every unlocked track, not only the track
  the clip is on, so later clips keep their picture and sound together. When that is not
  possible the gap is kept and a message says why.
- A clip dropped on a track replaces only what is on that track; its other half goes to
  a track that is free there.
- A new project's shape is used as the starting export format.
- A linked clip can be dragged to another track.
- The timeline follows the playhead when a key moves it out of view.

### Fixed

- Blade, split, trim to playhead, cut, copy and paste now act on a clip together with its
  linked sound. After a cut, the two later halves are a linked pair of their own.
- A cut no longer leaves a fade in and a fade out on both halves.
- After double-clicking a Media Pool clip, edits happened at the source clip's position
  instead of at the timeline playhead.
- Clips drew over the track headers when the timeline was scrolled or zoomed.
- Linked clips could not be moved or trimmed by small amounts with snapping on, because
  they snapped to their own old position. A click with a slightly unsteady hand no longer
  moves a clip.
- Short clips could only be trimmed, never moved.
- A clip's end could be dragged past the end of its footage; dragging past a limit threw
  the whole trim away instead of stopping at the limit. Titles and pictures can now be
  extended at their start.
- The timecode read one frame low after stepping by frames.
- Export: picture and sound drifted apart when cuts were not on frame boundaries.
- Redo failed after undoing an edit to the second half of an overwritten clip.
- The player kept reading the original file after a proxy became ready.
- Keyboard shortcuts no longer act on the timeline while a dialog or menu is open, and
  arrow keys work in lists again.
- Closing the window during an export discarded unsaved changes. A project that could not
  be saved could not be closed.
- Opening a project whose file was gone created an empty file in its place.
- The Project Manager's right-click menu acted on the last clicked project, not the one
  under the pointer.
- A double click on Render All started and at once stopped the export.
- The Media Pool forgot the selected clip whenever another file finished importing.
- A cross dissolve kept playing after the clip before it was moved or deleted.
- Track locks were ignored by trims, fades and the Inspector.
- "Restore unsaved changes?" was asked on every opening after choosing Discard.

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
