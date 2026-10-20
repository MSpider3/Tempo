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

**Hardware decoding is not available on this machine as set up.** Only the old `i965` VA-API
driver is installed, and it does not support Ice Lake. The `iHD` driver (`intel-media-driver`,
RPM Fusion nonfree) is missing, so Tempo falls back to the processor. Rows marked "hw decode
true" below are therefore software decoding too; they only confirm the fallback works.

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
