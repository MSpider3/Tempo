# Plugin System Specification — Tempo

**Version:** 2.0 (2026-10-03)
**Crate:** `tempo-plugin`
**Runtime:** Lua 5.4 (via `mlua`)
**Replaces:** version 1.0 (WASM plugins on wasmtime, HTML panels, Python compute server).

> **What is built (2026-10-05).** Script plugins (§7) work: a folder with `plugin.toml`
> and `main.lua`, commands in the main menu, the sandbox and limits of §7.2, and these parts
> of the API: `tempo.log`, `tempo.notify`, `tempo.playhead`, `tempo.selection`,
> `tempo.timeline.tracks/clips/clip/split/delete/move/set`, `tempo.markers.list/add/remove`,
> `tempo.media.loudness`. A script runs against a copy of the timeline and its changes are
> applied as one undo step.
>
> **Filters** work as data: a `[[filter]]` entry lists `ops` (`saturation`, `brightness`,
> `contrast`, `blur`, or a 4 × 5 colour `matrix`), each a number or `"$param"` naming one of
> its `[[filter.param]]` entries (`id`, `name`, `min`, `max`, `default`). Tempo draws them on
> the graphics card in the viewer and with FFmpeg filters in export. A plugin with only
> filters needs no `main.lua`. See `assets/plugins/filters/plugin.toml`.
>
> **Upload targets** work: an `[[uploader]]` entry (`id`, `name`, `function`) appears in
> **Share…** on a finished export. Its function gets
> `{ title, description, token, file_name, size }` and may return the video's address. It can
> call `tempo.http.request{ url, method, headers, body }`,
> `tempo.http.upload{ url, method, headers, field, form }` (sends the exported file, as the
> body or as form field `field`), `tempo.json.encode/decode` and `tempo.notify`. Requests go
> only over HTTPS and only to the hosts in `[permissions] network = [...]`; redirects are
> not followed. The token the user types is kept in the system keyring. No uploader for a
> real site ships with Tempo.
>
> Not built yet: shader effects (§5), title and preset packs (§6), `tempo.ui`,
> `tempo.files`, `tempo.secrets` for scripts, and the `.tempo-plugin` archive (plugins are
> installed from a folder). `tempo.timeline.move` takes `(clip, start)`; `tempo.media.loudness`
> returns decibel values from the cached waveform.

---

## 1. Why this design

The core editor stays small. Everything beyond basic cutting is added by plugins: more
transitions, filters, title styles, export presets, timeline tools, upload targets.

Version 1.0 of this spec used WebAssembly. That is replaced by **Lua** because:

- Lua is small, light on memory and easy for hobbyists to write.
- One runtime is enough. Version 1.0 had two (wasmtime and Lua).
- Dropping wasmtime shortens build time and shrinks the app.

### 1.1 Goals

- **Most plugins need no code.** A transition is a shader file and a list of settings.
- **Plugin controls look native.** A plugin declares its settings; Tempo draws them in the
  Inspector. Plugins never draw their own widgets.
- **A plugin cannot freeze or crash the editor.**
- **A plugin can only do what its manifest asks for**, and the user sees that list.

### 1.2 Removed from version 1.0

| Removed | Reason |
|---|---|
| WASM runtime (wasmtime) | Replaced by Lua |
| HTML panels in a web view | Heavy on memory, opened a hole in the sandbox, did not look native |
| Per-pixel `effect_render` in plugin code | Too slow on the target hardware; effects are shaders |
| Python compute server | Does not work inside the Flatpak; deferred until after 1.0 |
| "Code Mode" as a built-in plugin | Lua scripting is now the plugin system itself |

---

## 2. Two tiers

| Tier | Contains | Can add |
|---|---|---|
| **1. Data plugin** | `plugin.toml` plus shader, template or preset files. No code | Transitions, filters, title templates, export presets |
| **2. Script plugin** | The above plus `main.lua` | Timeline commands, importers and exporters for text formats, upload targets, automatic actions |

Tier 1 is built first (see `ROADMAP.md`). Tier 2 uses the same bundle and manifest.

---

## 3. Bundle

A plugin is a ZIP file with the extension `.tempo-plugin`.

```
glow.tempo-plugin
├── plugin.toml          required
├── main.lua             only for script plugins
├── shaders/glow.wgsl
├── titles/bold.toml
├── presets/discord.toml
├── icon.svg             optional, symbolic
└── README.md            optional
```

Installed to `~/.local/share/tempo/plugins/{id}/`. Built-in plugins ship in the app's data
directory and use the same format.

Install checks: the manifest parses, the `id` is unique, every referenced file exists, shaders
compile, and the unpacked size is under 20 MB. Paths that leave the plugin folder are rejected.

---

## 4. Manifest

```toml
[plugin]
id          = "com.example.glow"        # reverse-DNS, unique
name        = "Glow Pack"
version     = "1.0.0"
description = "A glow transition and a soft-glow filter"
author      = "Example"
min_tempo   = "1.0.0"
api         = 1                         # plugin API version

[permissions]                           # all default to off / empty
timeline = "none"                       # "none" | "read" | "write"
network  = []                           # host names, e.g. ["www.googleapis.com"]
files    = false                        # may ask the user to pick a file or folder
secrets  = false                        # may store login tokens in the system keyring

# ---- Tier 1: declared content -------------------------------------------

[[transition]]
id     = "glow"
name   = "Glow"
shader = "shaders/glow.wgsl"
default_duration = 1.0                  # seconds

  [[transition.param]]
  id = "strength"; name = "Strength"; type = "float"
  min = 0.0; max = 1.0; default = 0.5

  [[transition.param]]
  id = "tint"; name = "Tint"; type = "color"; default = "#ffffff"

[[filter]]
id     = "soft-glow"
name   = "Soft Glow"
shader = "shaders/soft_glow.wgsl"

  [[filter.param]]
  id = "amount"; name = "Amount"; type = "float"; min = 0.0; max = 1.0; default = 0.3

[[title]]
id = "bold"; name = "Bold Title"; template = "titles/bold.toml"

[[export_preset]]
id = "discord"; name = "Discord (under 10 MB)"; preset = "presets/discord.toml"

# ---- Tier 2: declared entry points --------------------------------------

[[command]]
id       = "remove-silence"
name     = "Remove Silence"
menu     = "clip"                       # "clip" | "timeline" | "marker" | "main"
function = "remove_silence"             # a global function in main.lua

[[uploader]]
id       = "youtube"
name     = "YouTube"
function = "upload_youtube"
```

### 4.1 Parameter types

| `type` | Inspector control | Value in shader | Value in Lua |
|---|---|---|---|
| `float` | Number field, plus a slider when `min` and `max` are set | `f32` | number |
| `int` | Number field | `i32` | integer |
| `bool` | Switch | `u32` (0 or 1) | boolean |
| `color` | Colour button | `vec4<f32>` | `{r,g,b,a}` |
| `choice` | Drop-down (`options = [...]`) | `u32` index | string |
| `point` | Two number fields, draggable in the viewer | `vec2<f32>` (0–1) | `{x,y}` |
| `text` | Text entry | not available | string |

Each parameter may set `name`, `default`, `min`, `max`, `step`, `unit`. Tempo builds one
Inspector section per effect, saves the values with the clip, and makes every change undoable.

---

## 5. Shaders

Effects are WGSL **fragment** shaders. Fragment shaders work on every graphics backend Tempo
supports, including the OpenGL ES fallback on older integrated graphics.

Tempo supplies the vertex stage and these bindings:

```wgsl
struct Frame {
    resolution: vec2<f32>,   // output size in pixels
    time:       f32,         // seconds from the start of the clip or transition
    progress:   f32,         // transitions: 0.0 → 1.0. Filters: always 0.0
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var tex_a: texture_2d<f32>;   // the clip (filter) or outgoing clip (transition)
@group(0) @binding(3) var tex_b: texture_2d<f32>;   // incoming clip (transitions only)
@group(1) @binding(0) var<uniform> params: Params;  // generated from the manifest, in order
```

The plugin writes the `Params` struct to match its manifest and one entry point:

```wgsl
struct Params { strength: f32, tint: vec4<f32> };

@fragment
fn effect(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let a = textureSample(tex_a, samp, uv);
    let b = textureSample(tex_b, samp, uv);
    let glow = params.tint * params.strength * sin(frame.progress * 3.14159);
    return mix(a, b, frame.progress) + glow;
}
```

Rules, checked at install time:

- The shader must compile, and its `Params` struct must match the manifest.
- No loops with a bound that is not a constant. At most 16 texture samples per pixel.
- Colours are linear, premultiplied alpha, in and out.
- If a shader fails at run time, the effect is skipped (a transition becomes a cut) and the
  user sees one toast naming the plugin.

---

## 6. Title templates and export presets

**Title template** (`titles/bold.toml`) — the same fields as a built-in title, with defaults:

```toml
text = "Title"
font = "Sans Bold"
size = 96
color = "#ffffff"
background = "#000000aa"
background_padding = 24
position = [0.5, 0.5]        # fraction of the frame
fade_in = 0.3
fade_out = 0.3
```

**Export preset** (`presets/discord.toml`):

```toml
container = "mp4"
video_codec = "h264"
max_width = 1280
max_height = 720
fps = "timeline"
target_size_mb = 10          # Tempo works out the bitrate from the duration
audio_codec = "aac"
audio_bitrate_kbps = 96
```

---

## 7. Script plugins (Lua)

### 7.1 How scripts run

- Each plugin gets its own Lua state. States are created the first time the plugin is needed,
  not at start-up.
- Scripts run on a dedicated plugin thread, never on the UI thread or the audio thread.
- A script runs only when Tempo calls one of the functions named in the manifest.
- All timeline changes made during one call are grouped into **one undo step**, labelled
  with the command's name. If the script fails, none of its changes are kept.

### 7.2 Sandbox

| Limit | Value |
|---|---|
| Libraries loaded | `string`, `table`, `math`, `utf8`, and safe parts of the base library |
| Libraries not loaded | `io`, `os`, `package`, `debug`; also `load`, `loadfile`, `dofile`, `require` of anything outside the plugin's own folder |
| Memory | 64 MB per plugin (`Lua::set_memory_limit`) |
| Run time | An instruction-count hook stops a call after 5 seconds of work. Waiting on `tempo.http` or a dialog does not count |
| Bytecode | Not accepted; scripts are loaded as text only |

A script that hits a limit or raises an error is stopped. The user sees a toast with the
plugin name and the first line of the error; the full error goes to the log. After three
failures in a session the plugin is switched off.

### 7.3 API

Everything is under the global table `tempo`. A function whose permission was not granted is
`nil`, so a script can test for it.

**Always available**

```lua
tempo.log(level, message)            -- "debug" | "info" | "warn" | "error"
tempo.notify(message)                -- a toast
tempo.project.info()                 -- { name, width, height, fps, duration }
tempo.playhead()                     -- seconds
tempo.selection()                    -- list of clip ids
tempo.ui.ask(fields)                 -- small form built from parameter declarations;
                                     -- returns a table of values, or nil if cancelled
tempo.ui.progress(fraction, text)    -- shows in the activity area
```

**`timeline = "read"`**

```lua
tempo.timeline.tracks()              -- { {id, kind, name, locked}, ... }
tempo.timeline.clips(track_id)       -- { {id, name, start, duration, source_start, kind}, ... }
tempo.timeline.clip(clip_id)
tempo.markers.list()                 -- { {id, time, name, color, note}, ... }
tempo.media.info(clip_id)            -- { path, width, height, fps, duration, has_audio }
tempo.media.loudness(clip_id, window)-- list of dB values, one per window (seconds);
                                     -- read from the cached waveform, no decoding
```

**`timeline = "write"`** (includes read)

```lua
tempo.timeline.split(clip_id, time)
tempo.timeline.delete(clip_id, { ripple = true })
tempo.timeline.move(clip_id, track_id, start)
tempo.timeline.trim(clip_id, start, duration)
tempo.timeline.set(clip_id, property, value)   -- "volume", "opacity", "zoom", ...
tempo.timeline.add_title(track_id, start, duration, template_id, { text = "..." })
tempo.timeline.add_transition(clip_id, "in" | "out", transition_id, params)
tempo.markers.add(time, name, color)
tempo.markers.remove(marker_id)
```

All times are in seconds as numbers. Tempo converts to its exact internal time.

**`files = true`**

```lua
tempo.files.pick_open(filters)       -- shows the system file dialog; returns a handle or nil
tempo.files.pick_save(name, filters)
handle:read()                        -- whole file as a string (limit 16 MB)
handle:write(string)
```

A script can only touch files the user picked in the dialog. It never sees a path it can
open by itself.

**`network = [hosts]`**

```lua
tempo.http.request{ method, url, headers, body }           -- returns { status, headers, body }
tempo.http.upload{ method, url, headers, file = export }   -- streams an exported file
tempo.oauth.authorize{ auth_url, token_url, client_id, scopes }
                                     -- opens the browser, waits for the user, returns tokens
```

Requests are only allowed to the hosts listed in the manifest, over HTTPS.

**`secrets = true`**

```lua
tempo.secrets.get(key)               -- stored in the system keyring, per plugin
tempo.secrets.set(key, value)
```

### 7.4 Example: a timeline command

```lua
-- Remove quiet stretches longer than `min_gap` from the selected clips.
function remove_silence()
    local opts = tempo.ui.ask{
        { id = "threshold", name = "Quieter than", type = "float", unit = "dB",
          min = -60, max = -20, default = -40 },
        { id = "min_gap", name = "Longer than", type = "float", unit = "s",
          min = 0.2, max = 5, default = 0.6 },
    }
    if not opts then return end

    local removed = 0
    for _, id in ipairs(tempo.selection()) do
        local clip = tempo.timeline.clip(id)
        local window = 0.05
        local levels = tempo.media.loudness(id, window)
        local quiet_from = nil

        -- Walk backwards so earlier times stay valid after a ripple delete.
        for i = #levels, 1, -1 do
            local t = clip.start + (i - 1) * window
            if levels[i] < opts.threshold then
                quiet_from = quiet_from or (t + window)
            elseif quiet_from then
                if quiet_from - (t + window) >= opts.min_gap then
                    local right = tempo.timeline.split(id, quiet_from)
                    local gap   = tempo.timeline.split(id, t + window)
                    tempo.timeline.delete(gap, { ripple = true })
                    removed = removed + 1
                end
                quiet_from = nil
            end
        end
    end
    tempo.notify("Removed " .. removed .. " silent parts")
end
```

The whole run is one undo step named "Remove Silence".

### 7.5 Upload targets

An uploader is a script plugin with `network`, `secrets` and an `[[uploader]]` entry. Tempo
calls its function with the finished export and the form the user filled in:

```lua
function upload_youtube(job)
    -- job.file         the exported file (for tempo.http.upload)
    -- job.title, job.description, job.visibility
    -- job.chapters     the chapter text made from markers
    local token = tempo.secrets.get("token") or sign_in()
    ...
    tempo.ui.progress(sent / total, "Uploading to YouTube")
end
```

- Each plugin author must register with the platform and follow its rules. Tempo's core
  contains no platform keys.
- When an uploader is installed, Render Settings shows an **Upload directly to {name}**
  check box for presets that name it (`uploader = "youtube"` in the preset file).
- Upload plugins are built after 1.0, YouTube first. Until then **Share…** opens the
  platform's upload page in the browser (`UI_SPEC.md §7.5`).

---

## 8. Lifecycle

```
Install  → validate → unpack → listed in the Plugins dialog, switched off
Enable   → register declared content (effects, titles, presets, commands, uploaders)
           shaders compile now; Lua does not load yet
First use of a command or uploader → create the Lua state, load main.lua, call the function
Disable  → unregister content, drop the Lua state
Remove   → disable, delete the folder
```

- Plugins are off until the user switches them on. The Plugins dialog shows what each one
  adds and which permissions it asks for.
- A project that uses an effect from a missing or disabled plugin still opens. The effect is
  skipped, the clip shows a small warning icon, and the settings are kept in the project so
  they return when the plugin does.
- Enabled state and plugin settings are stored in `~/.config/tempo/plugins.toml`, not in the
  project.

---

## 9. Built-in plugins

Shipped with the app, in the same format, so they double as examples.

| Plugin | Tier | Adds |
|---|---|---|
| `dev.tempo.transitions` | 1 | Wipe, Slide, Zoom |
| `dev.tempo.filters` | 1 | Brightness/Contrast, Saturation, Blur, Black and White |
| `dev.tempo.titles` | 1 | Four title styles |
| `dev.tempo.remove-silence` | 2 | The command in §7.4 |

Cross Dissolve, Dip to Colour, fades and the two basic titles are part of the core, not plugins.

---

## 10. Not in this version

- Plugins that process audio samples.
- Plugins that add whole panels or pages.
- Keyframe animation of plugin parameters (arrives with the keyframes plugin point, after 1.0).
- AI features that need Python (noise reduction, transcription). To be designed after 1.0 as
  a separate optional helper, not part of the plugin sandbox.

---

## 11. Implementation notes

- `mlua` with features `lua54`, `vendored`, `send`. Create states with
  `Lua::new_with(StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8, ..)`.
- The instruction limit uses `Lua::set_hook` with `HookTriggers::every_nth_instruction`.
- API functions that change the timeline build commands and send them to the UI thread's
  `CommandLog` inside a command group; the script waits for the result.
- Shader validation uses `naga` (already a dependency of wgpu) at install time, so a bad
  shader is refused before it reaches the GPU.
- `wasmtime` and `tempo-compute` are removed from the default build.
