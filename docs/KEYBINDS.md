# Keybind Reference — Tempo

**Version:** 2.1 (2026-10-05)
**Source of truth:** `EX/DaVinci Resolve Keys.txt` — the default "DaVinci Resolve" keyboard
preset exported from DaVinci Resolve 20.

Every binding in §2–§12 was checked line by line against that file. The name in the
"Resolve command" column is the command's name in the file, so any entry can be looked up.

---

## 1. Rules

1. **Same action, same key.** If Resolve has a default key for an action Tempo has, Tempo
   uses that key.
2. **Never reuse a Resolve key for something else.** If Tempo does not have the feature, the
   key does nothing. §14 lists the keys kept free for this reason.
3. **Tempo-only actions go in §13** and use keys that are empty in the Resolve file.
4. **One action per key.**
5. Keys are not user-remappable in 1.0.

The file was exported on a system where it notes `Ctrl` may mean Command (macOS). On Linux,
`Ctrl` is Ctrl and `Alt` is Alt.

---

## 2. Playback

| Key | Action | Tempo action | Resolve command |
|---|---|---|---|
| `Space` | Play / stop | `win.play-toggle` | `controlPlayToggle` |
| `L` | Play forward; press again for 2×, 4×, 8× | `win.play-forward` | `controlPlayForward` |
| `J` | Play reverse; press again for 2×, 4×, 8× | `win.play-reverse` | `controlPlayReverse` |
| `K` | Stop | `win.stop` | `controlStop` |
| `Shift+L` | Fast forward | `win.fast-forward` | `controlFastForward` |
| `Shift+J` | Fast reverse | `win.fast-reverse` | `controlFastReverse` |
| `Shift+K` | Play slow | `win.play-slow` | `controlPlaySlow` |
| `Alt+L` | Play again | `win.play-again` | `controlPlayAgain` |
| `→` / `←` | One frame forward / back | `win.step-forward` / `win.step-reverse` | `controlStepForward` / `controlStepReverse` |
| `Shift+→` / `Shift+←` | One second forward / back | `win.large-step-forward` / `win.large-step-reverse` | `controlLargeStepForward` / `…Reverse` |
| `↓` / `↑` | Next / previous clip (edit point) | `win.clip-next` / `win.clip-prev` | `controlClipNext` / `controlClipPrev` |
| `Home` / `End` | Timeline start / end | `win.timeline-start` / `win.timeline-end` | `controlTimelineStart` / `controlTimelineEnd` |
| `;` / `'` | First / last frame of the clip under the playhead | `win.first-frame` / `win.last-frame` | `controlFirstFrame` / `controlLastFrame` |
| `Ctrl+/` | Loop on / off | `win.loop-toggle` | `controlLoop` |
| `/` | Play around current selection | `win.play-around` | `controlPlayAroundToPlayAroundCurrentSelection` |
| `Alt+/` | Play In to Out | `win.play-in-to-out` | `controlPlayAroundToPlayInToOut` |
| `Shift+S` | Audio scrubbing on / off | `win.scrub-audio-toggle` | `editScrubAudio` |

Holding `K` while tapping or holding `L` or `J` (frame step and slow shuttle) is built into
Resolve's J-K-L handling and is not in the key file. Tempo implements it the same way.

Reverse and fast playback may show video only; audio is muted above 2×.

---

## 3. In and Out points

| Key | Action | Tempo action | Resolve command |
|---|---|---|---|
| `I` | Mark In | `win.mark-in` | `markIn` |
| `O` | Mark Out | `win.mark-out` | `markOut` |
| `Alt+I` | Clear In | `win.clear-in` | `markResetIn` |
| `Alt+O` | Clear Out | `win.clear-out` | `markResetOut` |
| `Alt+X` | Clear In and Out | `win.clear-in-out` | `markResetInOut` |
| `X` | Mark clip | `win.mark-clip` | `markClip` |
| `Shift+A` | Mark selection | `win.mark-selection` | `markSelected` |
| `Shift+I` | Go to In | `win.goto-in` | `controlGotoIn` |
| `Shift+O` | Go to Out | `win.goto-out` | `controlGotoOut` |

---

## 4. Markers

| Key | Action | Tempo action | Resolve command |
|---|---|---|---|
| `M` | Add marker | `win.marker-add` | `markMarkerAdd` |
| `Ctrl+M` | Add marker and open its dialog | `win.marker-add-modify` | `markMarkerAddAndModify` |
| `Shift+M` | Modify marker | `win.marker-modify` | `markMarkerModify` |
| `Alt+M` | Delete marker | `win.marker-delete` | `markMarkerClear` |
| `Shift+↓` | Next marker | `win.marker-next` | `controlMarkersNext` |
| `Shift+↑` | Previous marker | `win.marker-prev` | `controlMarkersPrev` |

Markers with a name become chapter timestamps on export (`UI_SPEC.md §7.4`).

---

## 5. Edit modes

| Key | Action | Tempo action | Resolve command |
|---|---|---|---|
| `A` | Selection mode | `win.tool-select` | `editPointer` |
| `T` | Trim edit mode | `win.tool-trim` | `editTrim` |
| `B` | Blade edit mode | `win.tool-blade` | `editBlade` |
| `N` | Snapping on / off | `win.snap-toggle` | `editSnapping` |
| `Ctrl+Shift+L` | Linked selection on / off | `win.linked-selection-toggle` | `editLinkedSelection` |

---

## 6. Editing

| Key | Action | Tempo action | Resolve command |
|---|---|---|---|
| `F9` | Insert | `win.edit-insert` | `editInsertOverwriteActionInsert` |
| `F10` | Overwrite | `win.edit-overwrite` | `…ActionOverwrite` |
| `F11` | Replace | `win.edit-replace` | `…ActionReplace` |
| `F12` | Place on top | `win.edit-place-on-top` | `…ActionPlaceOnTop` |
| `Shift+F10` | Ripple overwrite | `win.edit-ripple-overwrite` | `…ActionRippleOverwrite` |
| `Shift+F12` | Append at end | `win.edit-append` | `…ActionAppendAtEnd` |
| `Ctrl+B` | Razor: cut at the playhead | `win.razor` | `editBladeRazor` |
| `Ctrl+\` | Split clip | `win.split-clip` | `editM2SplitClip` |
| `Alt+\` | Join clip | `win.join-clip` | `editM2JoinClip` |
| `Backspace` | Delete selected, leave a gap | `win.delete` | `editBackspace` |
| `Shift+Backspace` or `Delete` | Ripple delete | `win.ripple-delete` | `editDelete` |
| `Ctrl+X` | Cut | `win.cut` | `editCut` |
| `Ctrl+Shift+X` | Ripple cut | `win.ripple-cut` | `editRippleCut` |
| `Ctrl+C` | Copy | `win.copy` | `editCopy` |
| `Ctrl+V` | Paste | `win.paste` | `editPaste` |
| `Ctrl+Shift+V` | Paste insert | `win.paste-insert` | `editPasteInsert` |
| `Ctrl+Z` | Undo | `win.undo` | `editUndo` |
| `Ctrl+Shift+Z` | Redo | `win.redo` | `editRedo` |
| `Ctrl+A` | Select all | `win.select-all` | `editSelectAll` |
| `Ctrl+Shift+A` | Deselect all | `win.deselect-all` | `editDeselectAll` |
| `Shift+V` | Select clip at playhead | `win.select-at-playhead` | `editSelectItemAtPlayhead` |
| `Y` | Select clips forward on this track | `win.select-track-after` | `editSelectClipsSelectTrackAfter` |
| `Alt+Y` | Select clips forward on all tracks | `win.select-all-after` | `editSelectClipsSelectAllAfter` |
| `Ctrl+Y` | Select clips backward on this track | `win.select-track-before` | `editSelectClipsSelectTrackBefore` |
| `Ctrl+Alt+Y` | Select clips backward on all tracks | `win.select-all-before` | `editSelectClipsSelectAllBefore` |
| `D` | Enable / disable clip | `win.clip-enable-toggle` | `editClipEnabled` |
| `Ctrl+Alt+L` | Link / unlink clips | `win.link-toggle` | `editLink` |
| `Ctrl+T` | Add transition (video and audio) | `win.transition-add` | `editAddTransition` |
| `Alt+T` | Add video-only transition | `win.transition-add-video` | `editAddVideoTransition` |
| `Shift+T` | Add audio-only transition | `win.transition-add-audio` | `editAddAudioTransition` |

Note that **`Ctrl+Y` is a selection command in Resolve, not redo.**

Resolve also binds the backtick key `` ` `` to undo. Tempo leaves that out, because it is
easy to press by accident.

---

## 7. Trimming, moving, fades, volume

| Key | Action | Tempo action | Resolve command |
|---|---|---|---|
| `Shift+[` | Trim start to playhead | `win.trim-start` | `editNudgeTrimStepTrimStart` |
| `Shift+]` | Trim end to playhead | `win.trim-end` | `editNudgeTrimStepTrimEnd` |
| `Ctrl+Shift+[` | Ripple start to playhead | `win.ripple-start` | `trimRippleStartToPlayhead` |
| `Ctrl+Shift+]` | Ripple end to playhead | `win.ripple-end` | `trimRippleEndToPlayhead` |
| `,` / `.` | Nudge one frame left / right | `win.nudge-reverse` / `win.nudge-forward` | `editNudgeTrimStepNudgeReverse` / `…Forward` |
| `Shift+,` / `Shift+.` | Nudge several frames left / right | `win.nudge-multi-left` / `win.nudge-multi-right` | `…TrimMultiFrameLeft` / `…Right` |
| `Ctrl+Shift+,` / `Ctrl+Shift+.` | Swap clip with the one before / after | `win.swap-reverse` / `win.swap-forward` | `editNudgeSwapEditReverse` / `…Forward` |
| `Alt+↑` / `Alt+↓` | Move clip up / down one track | `win.clip-move-up` / `win.clip-move-down` | `editMoveClipsUp` / `editMoveClipsDown` |
| `V` | Select nearest edit point | `win.select-edit-point` | `editSelectEditPoint` |
| `Alt+Shift+D` | Fade in to playhead | `win.fade-in-to-playhead` | `trimFadeInToPlayhead` |
| `Alt+Shift+G` | Fade out to playhead | `win.fade-out-to-playhead` | `trimFadeOutToPlayhead` |
| `Ctrl+Alt+=` / `Ctrl+Alt+-` | Clip volume up / down 1 dB | `win.volume-up-1` / `win.volume-down-1` | `clipAudioIncreaseAudioLevel1dB` / `…Decrease…` |
| `Alt+Shift+=` / `Alt+Shift+-` | Clip volume up / down 3 dB | `win.volume-up-3` / `win.volume-down-3` | `clipAudioIncreaseAudioLevel3dB` / `…Decrease…` |
| `Ctrl+Shift+1` … `4` | Enable / disable video track 1 … 4 | `win.video-track-toggle-N` | `editEnableDisableToggleVideoTrackN` |

---

## 8. Timeline view

| Input | Action | Tempo action | Resolve command |
|---|---|---|---|
| `Ctrl+=` | Zoom in | `win.zoom-in` | `viewZoomSubZoomIn` |
| `Ctrl+-` | Zoom out | `win.zoom-out` | `viewZoomSubZoomOut` |
| `Shift+Z` | Zoom to fit | `win.zoom-fit` | `viewZoomSubZoomToFit` |
| `Alt+Scroll` | Zoom around the pointer | — | mouse default |
| `Ctrl+Scroll` | Scroll left / right | — | mouse default |
| `Shift+Scroll` | Change track height | — | mouse default |

The three mouse gestures are not in the key file. Confirm them once in Resolve by hand.

---

## 9. Viewer

| Key | Action | Tempo action | Resolve command |
|---|---|---|---|
| `Q` | Switch between source and timeline viewer | `win.viewer-toggle` | `viewViewerToggle` |
| `Z` | Fit image to viewer | `win.viewer-fit` | `viewZoomViewerToFit` |
| `Alt+Shift+Z` | Actual size | `win.viewer-actual-size` | `viewZoomActualSize` |
| `P` or `Ctrl+F` | Cinema viewer (full screen) | `win.viewer-cinema` | `workspaceViewerModeCinemaViewer` |
| `Shift+F` | Full-page viewer (hides the timeline) | `win.viewer-full-page` | `workspaceViewerModeFullViewer` |
| `Escape` | Leave full-screen viewer | — | — |

---

## 10. Panel focus

In Resolve these keys move keyboard focus to a panel. They do not open or close it.
Opening and closing panels has no default key; use the buttons in the top bar.

| Key | Focus | Tempo action | Resolve command |
|---|---|---|---|
| `Ctrl+2` | Media Pool clips | `win.focus-media-pool` | `viewActiveWindowSelectionMediaPoolClips` |
| `Ctrl+3` | Source viewer | `win.focus-source-viewer` | `viewActiveWindowSelectionSourceViewer` |
| `Ctrl+4` | Timeline | `win.focus-timeline` | `viewActiveWindowSelectionTimeline` |
| `Ctrl+5` | Timeline viewer | `win.focus-timeline-viewer` | `viewActiveWindowSelectionTimelineViewer` |
| `Ctrl+6` | Effects | `win.focus-effects` | `viewActiveWindowSelectionEffects` |
| `Ctrl+9` | Inspector | `win.focus-inspector` | `viewActiveWindowSelectionInspector` |

If the panel is closed, the key opens it first and then focuses it.

---

## 11. Pages and windows

| Key | Action | Tempo action | Resolve command |
|---|---|---|---|
| `Shift+1` | Project Manager | `app.project-manager` | `fileProjectManager` |
| `Shift+4` | Edit page | `win.page-edit` | `workspacePrimaryWorkspaceEdit` |
| `Shift+8` | Export page (Deliver in Resolve) | `win.page-export` | `workspacePrimaryWorkspaceDeliver` |
| `Shift+9` | Project settings | `win.project-settings` | `fileProjectSettings` |
| `Ctrl+,` | Preferences | `app.preferences` | `resolvePreferences` |

---

## 12. File

| Key | Action | Tempo action | Resolve command |
|---|---|---|---|
| `Ctrl+S` | Save project | `app.save` | `fileSaveProject` |
| `Ctrl+Shift+S` | Save project as | `app.save-as` | `fileSaveProjectAs` |
| `Ctrl+I` | Import media | `win.import-media` | `fileImportMedia` |
| `Ctrl+Shift+N` | New bin | `win.new-bin` | `fileNewFolder` |
| `Ctrl+Q` | Quit | `app.quit` | `resolveQuit` |

"New Project" has no default key in Resolve (`fileNewProject :=` is empty), so it has none
in Tempo. Use the Project Manager.

---

## 13. Tempo-only actions

None of these keys appears anywhere in the Resolve file. Each action is also on a button or
in a menu.

| Key | Action | Tempo action |
|---|---|---|
| `Ctrl+Shift+E` | Quick Export | `win.quick-export` |
| `Ctrl+Alt+M` | Copy chapter list (from named markers) | `win.copy-chapters` |
| `Ctrl+Return` | Export page: add to render queue | `win.queue-add` |
| `Ctrl+Shift+Return` | Export page: render all | `win.queue-render-all` |
| `F1` | Keyboard shortcuts window | `win.show-shortcuts` |

`Ctrl+Shift+C` was used for Copy chapters in version 2.0 of this document. It is Resolve's
"Show Keyframe Editor", so it was moved.

---

## 14. Keys kept free on purpose

These are Resolve defaults for features Tempo does not have in 1.0. They must stay unbound
so that the key is available, with the right meaning, when the feature arrives.

| Key | Resolve action | Tempo plan |
|---|---|---|
| `R` | Range selection / Change clip speed | After 1.0 |
| `Ctrl+R`, `Shift+R`, `Ctrl+Alt+R` | Retime controls, freeze frame, reset retime | After 1.0 |
| `Ctrl+D` | Change clip duration | After 1.0 |
| `W`, `Ctrl+K` | Dynamic trim mode, stop | After 1.0 |
| `S` | Slip / slide mode | After 1.0 |
| `E`, `U`, `Alt+U`, `Alt+E`, `Shift+E` | Extend edit, edit-point type and selection | After 1.0 |
| `F` | Match frame | After 1.0 |
| `G` | Add flag | Not planned |
| `Shift+C`, `Ctrl+Shift+C` | Curve editor, keyframe editor | With the keyframes plugin point |
| `[`, `]` | Previous / next keyframe | With the keyframes plugin point |
| `Ctrl+N` | New timeline | Tempo has one timeline per project |
| `Ctrl+E`, `Ctrl+Shift+O`, `Ctrl+Shift+I` | Export project, export / import XML | With the interchange plugin |
| `Shift+2`, `3`, `5`, `6`, `7` | Media, Cut, Fusion, Color, Fairlight pages | Never |
| `Alt+1`…`9`, `Ctrl+Alt+1`…`9` | Track destination | Not planned |
| `Alt+F1`…`F10`, `Alt+Shift+F1`…`F9` | Auto-select and track-lock toggles | Not planned |
| `Shift+Q`, `Alt+Q`, `Ctrl+PgUp`, `Ctrl+PgDown` | Source/timeline viewer modes | Not planned |
| `` Shift+` `` | Viewer overlay | Not planned |
| `Alt+F`, `Ctrl+Shift+F` | Enhanced viewer / find clip, lightbox | Not planned |
| `Alt+V`, `Alt+Shift+V` | Paste attributes, paste value | After 1.0 |
| `Alt+B` | Create subclip | Not planned |
| `Shift+F11` | Fit to fill | After 1.0 (needs speed change) |
| `1`…`9` | Multicam angle | Never |

---

## 15. Implementation

- Bindings live in `assets/keybinds/keybinds.json`, loaded once at start-up.
- Each maps to a `gio::SimpleAction`. One `gtk::ShortcutController` in capture scope on the
  window registers them.
- Single-letter and punctuation keys must not fire while a text entry has focus.
- J-K-L held-key behaviour needs press and release tracking, so it uses an
  `EventControllerKey`.
- The same file fills the shortcuts window and button tooltips.

```json
{
  "version": 2,
  "keybinds": [
    { "action": "win.play-toggle",   "keys": ["space"], "label": "Play / Stop",
      "group": "Playback", "resolve": "controlPlayToggle" },
    { "action": "win.ripple-delete", "keys": ["<Shift>BackSpace", "Delete"], "label": "Ripple Delete",
      "group": "Editing", "resolve": "editDelete" },
    { "action": "win.copy-chapters", "keys": ["<Primary><Alt>m"], "label": "Copy Chapters",
      "group": "Markers", "resolve": null }
  ]
}
```

Entries with `"resolve": null` are shown under a "Tempo only" heading in the shortcuts window.

### Tests (`crates/tempo-ui/tests/keybinds.rs`)

1. No key is assigned to two actions.
2. No key in §14 is assigned.
3. For every entry with a `resolve` name, the key equals the key for that name in
   `EX/DaVinci Resolve Keys.txt`. This makes the Resolve export the automatic check, so the
   two can never drift apart.
