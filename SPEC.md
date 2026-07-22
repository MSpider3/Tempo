# Tempo Video Editor - Product Specification

## 1. Overview

**Product Vision**: Tempo is a lightweight, CPU-only desktop video editor that closely clones the DaVinci Resolve Cut Page experience. It is designed to run efficiently on standard hardware without relying on powerful GPUs.

**Target Audience**: Beginners, hobbyists, content creators, and users with low-end to mid-range PCs who need a fast, intuitive, and no-fuss editing tool.

**Core Philosophy**:
- **CPU-Only**: Operates efficiently on the CPU using FFmpeg for media processing.
- **Proxy-Based**: Employs an aggressive automatic proxy workflow for seamless playback and scrubbing, while using original files for high-quality export.
- **Beginner-Friendly**: Offers a straightforward interface heavily inspired by DaVinci Resolve's Cut Page.
- **Fast**: Emphasizes speed in editing with a dual-timeline design and comprehensive keyboard shortcuts.
- **Visually Clean**: Features a modern, minimalist dark theme to reduce eye strain and focus attention on the media.

---

## 2. Supported Formats

Tempo relies on FFmpeg for format support. The application will accept the following media types:

### Video
- **MP4** (.mp4)
- **MOV** (.mov)
- **MKV** (.mkv)
- **AVI** (.avi)
- **WebM** (.webm)

### Audio
- **MP3** (.mp3)
- **WAV** (.wav)
- **AAC** (.aac)
- **FLAC** (.flac)
- **OGG** (.ogg)

### Image (Imported as static clips)
- **JPG** (.jpg, .jpeg)
- **PNG** (.png)
- **WebP** (.webp)

---

## 3. Media Import & Proxy System

To ensure smooth playback on CPU-only hardware, Tempo uses a robust background proxy generation system.

### Import Workflow Step-by-Step
1. User clicks the "Import Media" button in the Media Bin or drags/drops files from the file explorer into the Media Bin.
2. The application reads the file paths and queues them for metadata extraction via `ffprobe`.
3. Valid files appear in the Media Bin immediately with a "Processing" status and a placeholder thumbnail.
4. Proxy generation begins asynchronously in the background.

### Auto-Proxy Generation
- **Logic**: Proxies are generated automatically based on the source resolution.
  - 1080p and higher: Generates a 480p proxy (H.264, low bitrate).
  - 4K and higher: Generates a 360p proxy (H.264, very low bitrate).
  - Below 1080p: No proxy needed; the original file is used for playback.

### Ongoing Proxy Generation Behavior
- **UI Indicator**: A small circular progress indicator appears on the thumbnail in the Media Bin while the proxy is rendering.
- **Playback**: If the user tries to play the file before the proxy is ready, the application uses the original file for playback until the proxy crosses a usable threshold (e.g., 10 seconds rendered), after which it seamlessly switches to the proxy stream.
- **Background Threading**: Proxy generation uses dedicated background threads (e.g., FFmpeg subprocesses) with low priority to ensure the UI thread is never blocked.

### Missing Original Files
- **Behavior**: If the source file is moved or deleted, a red "Media Offline" error icon appears on the thumbnail in the Media Bin and on any timeline clips referencing it.
- **Re-linking**: Right-clicking the missing media provides a "Re-link Media..." option, opening a standard file dialog to locate the missing file. The app validates the new file by comparing length and basic metadata.

### Metadata & Thumbnails
- **Metadata Extraction**: `ffprobe` extracts duration, resolution, frame rate, audio channels, and codec info.
- **Thumbnail Generation**: FFmpeg captures a frame at the 1-second mark (or 50% for very short clips) and caches it as a lightweight JPEG for the Media Bin.

---

## 4. Media Bin (Left Panel)

The Media Bin serves as the central repository for all project media.

- **Views**: Toggle between Grid view (large thumbnails) and List view (small thumbnails with sortable columns).
- **Display**: Shows filename, duration, and file type icon. Video thumbnails show frame previews. Audio shows waveform icons.
- **Import Button**: Prominent "Import Media" button at the top or bottom of the panel.
- **Right-Click Context Menu**:
  - `Remove`: Removes the item from the project (prompts if used in the timeline).
  - `Reveal in Explorer/Finder`: Opens the OS file browser to the file's location.
  - `Re-link Media...`: Manually update the file path.
- **Drag & Drop**: Users can drag items directly from the bin onto the upper or lower timeline.
- **Search & Filter**: A search bar allows filtering by filename. Dropdown filters allow filtering by media type (Video, Audio, Image).

---

## 5. Preview Player (Center Panel)

The Preview Player is the primary visual interface for playback, utilizing an embedded instance of MPV.

- **Embedded MPV Player**: Provides highly optimized, low-overhead playback.
- **Playback Source**: The player strictly plays the proxy file (if generated) to guarantee smooth scrubbing on CPUs.
- **Transport Controls**: 
  - Go to Start (`|◄`)
  - Fast Rewind (`◄◄`)
  - Play/Pause (`►/||`)
  - Fast Forward (`►►`)
  - Go to End (`►|`)
- **Timecode Display**: Shows `Current Time / Total Project Duration` (e.g., `00:01:23:14 / 00:05:00:00`).
- **In/Out Point Markers**: Visual brackets appear on the player's seek bar when In (`I`) or Out (`O`) points are set on the source media before dragging to the timeline.
- **Scrubbing**: Clicking and dragging on the player seek bar scrubs the video. MPV handles rapid seeking efficiently.
- **Text Overlay Preview**: Text blocks placed on the timeline are rendered live over the proxy stream using FFmpeg's `drawtext` filter (or MPV's native OSD rendering capabilities) so the user sees text immediately during playback.

---

## 6. Inspector Panel (Right Panel)

The Inspector Panel is context-sensitive. Its contents change entirely based on what is currently selected in the timeline.

### Video/Audio Clip Inspector
When a video or audio clip is selected:
- **Clip Info**: Name, Total Duration, In Point timecode, Out Point timecode.
- **Speed Dropdown**: Allows changing clip playback speed (Options: `0.25x`, `0.5x`, `0.75x`, `1x`, `1.25x`, `1.5x`, `2x`, `4x`).
- **Pitch Correction Toggle**: Checkbox (default: ON). If on, audio pitch is maintained when speed changes.
- **Transition In Dropdown**: Determines how the clip starts.
  - Options: `Cut` (default), `Fade In`, `Cross Dissolve`, `Cut to Black`, `Cut to White`.
- **Transition Out Dropdown**: Determines how the clip ends.
  - Options: `Cut` (default), `Fade Out`, `Cross Dissolve`, `Cut to Black`, `Cut to White`.
- **Transition Duration**: Number input (in seconds/frames) for how long the transition lasts. (e.g., `1.0s`).

### Text Block Inspector
When a Text (TX) clip is selected:
- **Text Content**: A multiline text area where the user types the on-screen text.
- **Font Family Dropdown**: Options: `Inter`, `Arial`, `Roboto`, `Times New Roman`, `monospace`.
- **Font Size**: Number input/slider from `8px` to `120px`.
- **Font Color Picker**: Standard hex color picker.
- **Background Color + Opacity**: Hex color picker and an opacity slider (0-100%) for a solid background box behind the text.
- **Formatting Toggles**: Bold, Italic, Underline buttons.
- **Alignment**: Left, Center, Right align buttons.
- **Position**: 
  - `X%` Slider (0-100, default 50% for center).
  - `Y%` Slider (0-100, default 85% for lower thirds).
- **Rotation**: Dial/Slider from `-180°` to `+180°`.

---

## 7. Dual Timeline System

Modeled exactly after DaVinci Resolve's Cut Page, Tempo features two distinct timeline views stacked vertically.

- **Upper Timeline**: The entire project view. Fully zoomed out at all times. Shows the entire sequence from start to finish. A playhead indicator box highlights what portion of the project is currently visible in the lower timeline. Clicking anywhere jumps the playhead to that relative position.
- **Lower Timeline**: The detailed editing view. Zoomed in around the playhead. Scrolls automatically as the playhead moves during playback.
- **Tracks**: 
  - The track hierarchy from top to bottom: `TX` (Text), `V3`, `V2`, `V1` (Video), `A1`, `A2`, `A3` (Audio).
  - `V1` and `A1` are the primary tracks. Video with embedded audio dragged to `V1` will automatically put its audio in `A1`.
- **Clip Rendering**:
  - Video clips show start and end thumbnails within the clip block.
  - Audio clips display a simplified visual waveform.
  - Text clips on the `TX` track are rendered as solid purple blocks with a preview snippet of the text inside.
- **Speed Badges**: If a clip's speed is altered from `1x`, a small badge (e.g., `[1.5x]`) appears in the corner of the clip block. The physical width of the clip on the timeline scales automatically based on the speed (e.g., 2x speed makes the clip half as long).

---

## 8. Clip Operations

Interactions within the Lower Timeline:

- **Cut (Blade Tool)**: Pressing `B` cuts all selected clips exactly at the playhead position. If no clips are selected, it cuts all clips on all tracks at the playhead. The clip is split into two independent blocks.
- **Copy/Paste**: 
  - `Copy` (`Ctrl+C`): Copies the selected clip and all its inspector properties (speed, transitions).
  - `Paste` (`Ctrl+V`): Pastes the copied clip at the current playhead position on the originally targeted track. Existing clips at or after the paste point are pushed to the right (ripple insert) to make room. This avoids the complexity of partial clip splitting and is more intuitive for beginners.
- **Delete**: 
  - `Backspace`: Standard delete. Removes the clip and leaves a blank gap in the timeline.
  - `Delete` (or `Shift+Backspace`): Ripple delete. Removes the clip and shifts all subsequent clips on that track to the left to close the gap.
- **Trim**: Hovering over the left or right edge of a clip turns the cursor into a trim bracket. Clicking and dragging inward trims the In or Out point. The viewer updates to show the exact frame being trimmed.
- **Move/Reorder**: Clicking and dragging a clip left/right moves it in time. By default, it uses "Ripple Insert" behavior—dropping it between two clips pushes subsequent clips to the right. Dropping it directly over another clip overwrites the section.
- **Track Dragging**: Users can drag a clip vertically from V1 to V2 or V3 to overlay media. Audio can be moved similarly.

---

## 9. Transitions

Transitions are handled non-destructively and applied dynamically during playback and export.

- **Types**: 
  - `Cut` (Hard cut, default)
  - `Fade In` / `Fade Out` (Fades to/from transparent/black)
  - `Cross Dissolve` (Fades between two adjacent clips)
  - `Cut to Black` / `Cut to White` (Flashes to a solid color)
  - `Crossfade` (Audio only, fades audio levels between clips)
- **Application**: Applied entirely via the Inspector Panel dropdowns for "Transition In" and "Transition Out".
- **Visuals**: A small angled gradient overlays the beginning or end of the clip block on the timeline to indicate a transition is present.
- **Export**: Rendered at export time using FFmpeg filters (`xfade`, `fade`, `afade`).

---

## 10. Speed Control

Allows creating fast-motion or slow-motion effects.

- **Options**: `0.25x`, `0.5x`, `0.75x`, `1x`, `1.25x`, `1.5x`, `2x`, `4x`.
- **Controls**: Adjustable via the Inspector Dropdown, the `Ctrl+R` speed change dialog, or quick keys `Ctrl+Up` / `Ctrl+Down`.
- **Timeline Behavior**: When speed changes, the clip duration instantly adjusts. Subsequent clips on the track ripple forward or backward to accommodate the new duration.
- **Visuals**: A text-only speed badge (e.g., `[2×]` or `[0.5×]`) appears on the clip block. Emoji are avoided because Qt emoji rendering is platform-inconsistent — on some Linux setups they render as boxes or missing glyphs.
- **Audio Pitch**: If Pitch Correction is ON, FFmpeg uses the `asetrate` and `atempo` filters to maintain normal pitch. If OFF, pitch shifts up (chipmunk) or down (deep voice).
- **Export Handling**: Handled via `setpts` for video and `atempo` for audio during the FFmpeg export pipeline.

---

## 11. Text/Title Overlays

A simplified but powerful titling system.

- **TX Track**: Always the topmost track, positioned above `V3`.
- **Creation**: Double-click on an empty space in the `TX` track, or press `Ctrl+T` to spawn a new 5-second text block at the playhead.
- **Visuals**: Text blocks are bright purple.
- **Editing**: All text styling (font, size, color, background, alignment, position, rotation) is driven purely by the Inspector Panel.
- **Live Preview**: When the playhead is over a text block, the text is dynamically injected into the MPV player stream via the `drawtext` FFmpeg command (or MPV OSD) to provide instantaneous visual feedback as sliders are moved.
- **Export Rendering**: Fully baked into the final video file via complex FFmpeg `drawtext` filter graphs.

---

## 12. Export

The final step, utilizing original source files to ensure maximum quality.

- **Format**: MP4 container, H.264 Video Codec, AAC Audio Codec.
- **Resolution Options**:
  - `Original` (Matches the resolution of the first imported clip)
  - `1080p` (1920x1080)
  - `720p` (1280x720)
  - `480p` (854x480)
- **Mechanism**: The application generates a massive, complex FFmpeg command string mapping all cuts, transitions, text overlays, and speed changes, executing it as a subprocess. It uses the ORIGINAL files, completely ignoring the proxies used for playback.
- **UI**: A dedicated Export Dialog prompts for filename, destination, and resolution. A progress bar tracks FFmpeg output parsing. The UI remains responsive (though perhaps sluggish depending on CPU load), but the user cannot edit while exporting.

---

## 13. Project Management

- **Format**: Projects are saved as `.tempo` files, which are strictly JSON-formatted text files. They contain references to file paths, timeline layouts, inspector properties, and nothing else (no embedded media).
- **Operations**: Standard `Save` (`Ctrl+S`), `Load` (`Ctrl+O`), `Save As` (`Ctrl+Shift+S`).
- **Recent Projects**: The welcome screen displays a list of the 5 most recently opened `.tempo` files.
- **Auto-Save**: The application silently auto-saves every 3 minutes to a temporary `~/.tempo/autosave.tempo` file to prevent data loss on crash.

---

## 14. Keybindings Reference

These keybindings exactly match the core functionalities of the DaVinci Resolve Cut Page:

| Category | Action | Shortcut |
| :--- | :--- | :--- |
| **Playback** | Play / Pause | `Space` |
| | Play Reverse | `J` |
| | Pause | `K` |
| | Play Forward | `L` |
| | Fast Forward / Fast Reverse | `L` (tap twice) / `J` (tap twice) |
| **In/Out** | Mark In | `I` |
| | Mark Out | `O` |
| | Clear In / Out | `Alt+I` / `Alt+O` |
| **Editing** | Blade (Cut) | `B` |
| | Select Tool | `A` |
| | Delete (Leave Gap) | `Backspace` |
| | Ripple Delete | `Delete` or `Shift+Backspace` |
| | Copy | `Ctrl+C` |
| | Paste | `Ctrl+V` |
| | Undo | `Ctrl+Z` |
| | Redo | `Ctrl+Shift+Z` |
| **Speed** | Open Speed Dialog | `Ctrl+R` |
| | Speed Up / Slow Down | `Ctrl+Up` / `Ctrl+Down` |
| **Text** | Add Text Block | `Ctrl+T` |
| **Timeline** | Zoom In / Out | `Alt+Scroll` or `Ctrl++` / `Ctrl+-` |
| | Fit to View | `Shift+Z` |
| **Import/Export**| Save Project | `Ctrl+S` |
| | Open Project | `Ctrl+O` |
| | Import Media | `Ctrl+I` |
| | Quick Export | `Ctrl+E` |

---

## 15. Out of Scope

To maintain the lightweight, CPU-only, beginner-focused nature of Tempo, the following features are explicitly **out of scope** and will not be implemented:

- Color Grading / Color Correction (LUTs, scopes, curves)
- Visual Effects (VFX) Plugins (OpenFX, VSTs)
- Green Screen / Chroma Keying
- Motion Tracking
- Keyframing (for position, scale, or effects)
- GPU Acceleration / Hardware Encoding (NVENC, AMF, QuickSync)
- Cloud Sync / Collaboration tools
- Advanced Audio Mixing / EQ / Compression
- Multi-cam editing
- Subtitles/Captioning automation (SRT import/export)
