# Tempo — Keybinding Reference

> **Compatibility:** All keybindings in Tempo match **DaVinci Resolve's Cut Page (Page 2)** exactly.
> If you're coming from DaVinci Resolve, your muscle memory transfers directly.

---

## Playback

| Key | Action | Context |
|-----|--------|---------|
| `Space` | Play / Pause | Global |
| `J` | Rewind (press multiple times to increase speed) | Global |
| `K` | Stop / Pause | Global |
| `L` | Fast Forward (press multiple times to increase speed) | Global |
| `←` Left Arrow | Step back 1 frame | Global |
| `→` Right Arrow | Step forward 1 frame | Global |
| `Shift+←` | Step back 1 second | Global |
| `Shift+→` | Step forward 1 second | Global |
| `Home` | Go to start of timeline | Global |
| `End` | Go to end of timeline | Global |

### J/K/L Shuttle Control

The J, K, and L keys implement a **shuttle control** system, identical to DaVinci Resolve:

| State | Behavior |
|-------|----------|
| `L` × 1 | Play forward at 1× speed |
| `L` × 2 | Play forward at 2× speed |
| `L` × 3 | Play forward at 4× speed |
| `L` × 4 | Play forward at 8× speed |
| `J` × 1 | Play reverse at 1× speed |
| `J` × 2 | Play reverse at 2× speed |
| `J` × 3 | Play reverse at 4× speed |
| `J` × 4 | Play reverse at 8× speed |
| `K` | Stop / Pause (resets shuttle speed) |
| `K` + `L` (held) | Play forward at slow speed (1/4×) |
| `K` + `J` (held) | Play reverse at slow speed (1/4×) |

- Pressing `J` while playing forward decelerates (e.g., `L L J` → 1× forward).
- Pressing `L` while playing reverse decelerates (e.g., `J J L` → 1× reverse).
- `K` always stops playback and resets the shuttle state.

---

## In / Out Points

| Key | Action | Context |
|-----|--------|---------|
| `I` | Mark In point at current playhead position | Global |
| `O` | Mark Out point at current playhead position | Global |
| `Alt+I` | Clear In point | Global |
| `Alt+O` | Clear Out point | Global |

---

## Editing

| Key | Action | Context |
|-----|--------|---------|
| `Ctrl+Z` | Undo last action | Global |
| `Ctrl+Shift+Z` | Redo last undone action | Global |
| `Ctrl+C` | Copy selected clip(s) | Timeline |
| `Ctrl+X` | Cut selected clip(s) (copy + delete) | Timeline |
| `Ctrl+V` | Paste clip(s) at playhead position | Timeline |
| `Delete` | Ripple delete selected clip(s) — closes the gap | Timeline |
| `Backspace` | Delete selected clip(s) — leaves gap on timeline | Timeline |
| `B` | Blade / Razor tool — split clip at playhead | Timeline |
| `A` | Select / Arrow tool (default pointer mode) | Timeline |
| `T` | Trim tool — enable trim mode for clip edges | Timeline |

---

## Speed Control

| Key | Action | Context |
|-----|--------|---------|
| `Ctrl+R` | Open speed change dialog for selected clip | Clip selected |
| `Ctrl+↑` | Increase speed one step (e.g., 1× → 1.25× → 1.5× → 2× → 4×) | Clip selected |
| `Ctrl+↓` | Decrease speed one step (e.g., 1× → 0.75× → 0.5× → 0.25×) | Clip selected |

**Speed steps:** 0.25× → 0.5× → 0.75× → 1× → 1.25× → 1.5× → 2× → 4×

---

## Text / Title

| Key | Action | Context |
|-----|--------|---------|
| `Ctrl+T` | Add new text block at playhead position on TX track | Global |

Text blocks can also be created by **double-clicking** directly on the TX track at any position.

---

## Timeline Navigation

| Key | Action | Context |
|-----|--------|---------|
| `Ctrl+=` / `Ctrl++` | Zoom in timeline | Timeline |
| `Ctrl+-` | Zoom out timeline | Timeline |
| `Ctrl+Shift+F` | Fit timeline to window (zoom to fit all clips) | Timeline |
| `Shift+Z` | Zoom timeline to fit (alias for Ctrl+Shift+F) | Timeline |

---

## Import / Export / Project

| Key | Action | Context |
|-----|--------|---------|
| `Ctrl+I` | Import media files | Global |
| `Ctrl+E` | Export / Render project | Global |
| `Ctrl+S` | Save project | Global |
| `Ctrl+Shift+S` | Save project as (new file) | Global |
| `Ctrl+O` | Open existing project | Global |

---

## Media Bin

| Key | Action | Context |
|-----|--------|---------|
| `F5` | Refresh media bin (re-scan imported files) | Global |

---

## Quick Reference Card

```
PLAY/STOP    Space  J  K  L          NAVIGATE    ← → Home End
IN/OUT       I  O  Alt+I  Alt+O      ZOOM        Ctrl++  Ctrl+-  Shift+Z
EDIT         B (blade)  A (select)   CLIPBOARD   Ctrl+C  Ctrl+X  Ctrl+V
             T (trim)                DELETE      Delete (ripple)  Backspace (gap)
SPEED        Ctrl+R  Ctrl+↑  Ctrl+↓  TEXT        Ctrl+T
PROJECT      Ctrl+S  Ctrl+O  Ctrl+E  UNDO        Ctrl+Z  Ctrl+Shift+Z
IMPORT       Ctrl+I                  REFRESH     F5
```

---

## Notes

- All keybindings are registered centrally in `ui/keybindings.py` — they are never hardcoded in individual widget files.
- Context-dependent shortcuts (e.g., Timeline-only shortcuts) are automatically enabled/disabled based on focus.
- Modifier keys follow platform conventions: `Ctrl` on Linux/Windows, `Cmd` on macOS.
- The `?` key will open a keybinding cheat sheet overlay (planned for Phase 4).
