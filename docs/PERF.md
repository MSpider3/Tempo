# Playback performance

Numbers from `scripts/bench_playback.sh`, which plays about 10 s of generated footage in
Tempo and records:

- dropped frames, as a count and a percentage of the frames that should have shown;
- peak memory (RSS);
- CPU use (100 % = one core);
- timeline canvas redraws.

Each value is the median of 3 runs. Later changes must not make these numbers worse.
Every attempt is logged below, including the ones that were reverted.

## Reference machine

The development laptop: Intel Core i3-1005G1 (2 cores / 4 threads, 1.2 GHz base), Intel Iris
Plus G1 (Ice Lake) graphics, 19 GB RAM, Fedora 44, Tempo's `dev` profile (optimised + debug info).

Until 2026-10-20 evening only the old `i965` VA-API driver was installed, which does not
support Ice Lake, so the "hw decode true" rows in the baseline are really software decoding.
The `iHD` driver (`intel-media-driver` 26.1.5) was then installed; see
[Hardware acceleration](#hardware-acceleration--2026-10-20-ihd-driver).

## Footage

| case | file | notes |
|---|---|---|
| 1080p30 | testsrc2 1920×1080 30 fps, x264 veryfast, AAC | easy to decode |
| 1080p60 | same at 60 fps, played in a 60 fps project | |
| 2160p30 | 3840×2160 30 fps x264 | |
| 2160p30_hevc | 3840×2160 30 fps x265 | |
| 1080p30_full | 1080p30 at preview quality Full (original, full size) | |
| 2160p60_grain | 3840×2160 60 fps with moving grain at 80 Mb/s (like a phone camera), 60 fps project | the stress case |

The test pattern alone is much easier to decode than camera footage, which is why the grain
case exists.

## Baseline — 2026-10-20, branch `feat/p0-p3-resolve-parity-lowspec` after Phase 0

| case | preview | hw decode | played s | dropped | dropped % | peak RSS MB | CPU % | canvas draws |
|---|---|---|---|---|---|---|---|---|
| 1080p30 | 540p | false | 10.0 | 0 | 0.0 | 269 | 38 | 295 |
| 1080p60 | 540p | false | 10.0 | 0 | 0.0 | 272 | 60 | 444 |
| 2160p30 | 540p | false | 9.9 | 0 | 0.0 | 442 | 82 | 293 |
| 2160p30_hevc | 540p | false | 10.0 | 0 | 0.0 | 449 | 115 | 294 |
| 1080p30_full | Full | false | 10.0 | 0 | 0.0 | 302 | 52 | 295 |
| 2160p60_grain | 540p | false | 9.5 | 473 | 82.9 | 466 | 240 | 158 |
| 1080p60 | 540p | true (fell back) | 10.0 | 3 | 0.5 | 273 | 60 | 445 |
| 2160p30 | 540p | true (fell back) | 9.9 | 0 | 0.0 | 447 | 83 | 297 |
| 2160p60_grain | 540p | true (fell back) | 9.4 | 482 | 85.6 | 471 | 210 | 35 |

Observations:

- Ordinary footage up to 4K30 plays without drops on this machine. The 4K60 stress case drops
  most frames, because decoding alone can't keep up.
- The timeline canvas redraws on every frame shown (≈ 30 per second at 30 fps).
- Peak memory stays far under the 1.5 GB budget.

## Experiment log

| date | change | before → after | verdict |
|---|---|---|---|
| 2026-10-20 | baseline | — | recorded |
| 2026-10-20 | P1-T2 adaptive decode: after 3 frames over 1.25× their time, skip the deblocking filter and halve the decode height until playback stops or seeks | 2160p60_grain: 473 → 49 dropped (82.9 % → 8.5 %), CPU 240 → 174 %; 2160p30 and 1080p60 unchanged (0 dropped, never triggered) | kept |
| 2026-10-20 | P1-T3 playhead on its own overlay layer; the canvas is redrawn only when clips, scroll or zoom change | canvas draws in 10 s playback: 1080p30 295 → 2, 1080p60 448 → 2; CPU 38 → 37 %, 59 → 59 % (within noise on this short timeline; the saving grows with clip count) | kept |
| 2026-10-20 | P1-T4 decoded frames shared (`Arc`) instead of copied into the repeat cache and again into GTK | median of 5 vs previous commit: 1080p30 Full CPU 49 → 43 %, RSS 304 → 296 MB; 2160p30 at 540p 80 → 78 % (noise: 2 MB frames) | kept |
| 2026-10-20 | P1-T6 proxies made only for footage on the timeline; VA-API encode when the start-up check passes (not on this machine), software otherwise; AAC sound inside the proxy, read by the audio feeder with the original as fallback | 2160p60_grain from a proxy: 0 dropped, CPU 34 % with sound from the proxy vs 36 % with sound from the 80 Mb/s original (both 0 dropped; median of 5). Proxy vs original file: 240 % → 34 % CPU, 82.9 % → 0 % dropped. Sound stays within ±1 ms (test). The HDD case it is meant for could not be measured here | kept |
| 2026-10-20 | P1-T7 DMA-BUF zero-copy display (spike) | not measurable here: it only applies with hardware decoding, and this machine lacks the `iHD` VA-API driver. Not implemented. To run it: `sudo dnf install intel-media-driver` (RPM Fusion nonfree), confirm `vainfo` lists H.264 decode, then measure `HW=true ./scripts/bench_playback.sh` before and after; keep only if CPU falls ≥ 30 % and colours match within 3/255 | blocked |

## After Phase 1 — 2026-10-20

| case | preview | hw decode | played s | dropped | dropped % | peak RSS MB | CPU % | canvas draws |
|---|---|---|---|---|---|---|---|---|
| 1080p30 | 540p | false | 10.0 | 0 | 0.0 | 274 | 29 | 2 |
| 1080p60 | 540p | false | 10.0 | 0 | 0.0 | 273 | 44 | 2 |
| 2160p30 | 540p | false | 9.9 | 0 | 0.0 | 443 | 67 | 2 |
| 2160p30_hevc | 540p | false | 10.0 | 0 | 0.0 | 452 | 96 | 2 |
| 1080p30_full | Full | false | 10.0 | 0 | 0.0 | 308 | 33 | 2 |
| 2160p60_grain | 540p | false | 9.6 | 55 | 9.5 | 462 | 165 | 2 |
| 2160p60_grain_proxy | 540p | false | 10.0 | 0 | 0.0 | 245 | 23 | 2 |

Against the baseline:

- The 4K60 stress case drops 83 % → 9.5 % of frames from the original, and 0 % from its proxy.
- Peak memory stays under 0.5 GB.

**Correction (later the same day):** the CPU column of this table was an unusually good run
and does not show a real gain. Re-measured back to back (A/B, below), the commit this table
was taken from gives 58 / 79 / 44 % CPU for 1080p60 / 2160p30 / 1080p30 Full, not 44 / 67 /
33 %. The CPU changes that hold up are:

- Full preview: 52 → 45 %, from sharing frames (P1-T4).
- 4K60 stress: 240 → 170 %, from adaptive decode (P1-T2), and 33 % from a proxy.

Removing the per-frame timeline redraw (295 → 2 redraws) did not measurably change CPU on
this short three-clip timeline. Compare runs taken back to back only.

## Final — 2026-10-20, after Phases 2–3 and the review fixes

| case | preview | hw decode | played s | dropped | dropped % | peak RSS MB | CPU % | canvas draws |
|---|---|---|---|---|---|---|---|---|
| 1080p30 | 540p | false | 10.0 | 0 | 0.0 | 277 | 38 | 2 |
| 1080p60 | 540p | false | 10.0 | 0 | 0.0 | 272 | 59 | 2 |
| 2160p30 | 540p | false | 9.9 | 0 | 0.0 | 444 | 79 | 2 |
| 2160p30_hevc | 540p | false | 10.0 | 0 | 0.0 | 456 | 114 | 2 |
| 1080p30_full | Full | false | 10.0 | 0 | 0.0 | 302 | 45 | 2 |
| 2160p60_grain | 540p | false | 9.6 | 44 | 7.7 | 464 | 170 | 2 |
| 2160p60_grain_proxy | 540p | false | 10.0 | 0 | 0.0 | 247 | 33 | 2 |

A/B, run back to back with the same cases: the end of Phase 1 (`03856db`) gives 58 / 79 / 44 %
CPU for 1080p60 / 2160p30 / 1080p30 Full, and this branch gives 59 / 79 / 45 %. Phases 2–3
did not slow playback.

## Hardware acceleration — 2026-10-20, iHD driver

All runs are back to back on the same build (`1c2e292` + later fixes); medians of 3.

### Playback: software vs VA-API decoding (preview 540p)

| case | software CPU / dropped | VA-API CPU / dropped |
|---|---|---|
| 1080p60 | 58 % / 0 | 46 % / 0 |
| 2160p30 H.264 | 79 % / 0 | 51 % / 4 (1.3 %) |
| 2160p30 HEVC | 113 % / 0 | 52 % / 15 (5.0 %) |
| 2160p60 grain | 174 % / 39 (6.8 %) | 85 % / 356 (61.9 %) |

Peak memory also falls, by 70–90 MB on the 4K cases.

**Verdict:** hardware decoding halves the processor's work but drops *more* frames at 4K.
CPU falls while drops rise, so playback is waiting rather than computing. The likely
cause is copying every full 4K picture back from graphics memory before shrinking it on the
processor; decode-time skipping (adaptive decode) can't help that.

The setting stays off by default and marked experimental. It suits 1080p, where it saves
CPU with no drops. The DMA-BUF zero-copy spike (P1-T7) is what would remove the copy back,
and it can now be measured on this machine.

### Proxies: 20 s 4K60 grain clip to a 540p proxy with sound

| path | time | CPU | peak RSS |
|---|---|---|---|
| software (decode, scale and x264 on the processor) | 37.1 s | 130 % | 198 MB |
| VA-API encode only (decode on the processor, upload full frames) | 38.8 s | 136 % | 240 MB |
| **VA-API throughout (decode, scale_vaapi, h264_vaapi)** | **6.9 s** | **45 %** | **135 MB** |

Tempo now uses the last one when the start-up check finds VA-API encoding works, and falls
back to software when a file can't be decoded on the chip.

### Export: 20 s 4K30 project to 1080p H.264

| path | time | CPU | peak RSS |
|---|---|---|---|
| software x264 | 15.9 s | 346 % | 645 MB |
| VA-API encode | 14.0 s | 304 % | 381 MB |

Modest, because decoding the 4K source and compositing still run on the processor; memory
falls by 40 %.

### Bugs found by testing with the driver

- Hardware decoding failed when asked for a time past the end of a file: the held frame
  was a pixel copy that had lost its GPU surface. Fixed by keeping a reference
  (`av_frame_ref`), which also removed a full frame copy per decoded frame in software
  decoding.
- A size-capped export with the hardware setting on failed. Fixed.

| date | change | before → after | verdict |
|---|---|---|---|
| 2026-10-20 | Proxies decoded on the processor, encoded with VA-API | 37.1 → 38.8 s (no gain) | replaced |
| 2026-10-20 | Proxies decoded, scaled and encoded with VA-API | 37.1 → 6.9 s, 130 → 45 % CPU | kept |
