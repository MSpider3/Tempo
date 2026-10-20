#!/usr/bin/env bash
# Playback benchmark: dropped frames, peak memory, CPU and timeline redraws while
# Tempo plays about 10 s of generated footage. Results go to docs/PERF.md by hand.
#
#   ./scripts/bench_playback.sh                 # every case, 3 runs each (median)
#   RUNS=1 CASES="1080p30 2160p30" ./scripts/bench_playback.sh
#   HW=true ./scripts/bench_playback.sh         # with hardware (VA-API) decoding on
#
# Each run uses its own settings, project list and cache, never the user's.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

RUNS=${RUNS:-3}
HW=${HW:-false}
CASES=${CASES:-"1080p30 1080p60 2160p30 2160p30_hevc 1080p30_full 2160p60_grain"}
media=target/bench-media
work=target/bench
mkdir -p "$media" "$work"

fixture() { # name, size, rate, encoder, [video filter], [encoder options]
    local out="$media/$1.mp4"
    [ -f "$out" ] && return 0
    echo "making $out" >&2
    ffmpeg -v error -y -f lavfi -i "testsrc2=size=$2:rate=$3:duration=20" \
        -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=20" \
        -vf "${5:-null}" -c:v "$4" ${6:-} -preset veryfast -g 60 -pix_fmt yuv420p -c:a aac -b:a 128k -ac 2 -shortest "$out"
}

cargo build -q --bin tempo
fixture 1080p30 1920x1080 30 libx264
fixture 1080p60 1920x1080 60 libx264
fixture 2160p30 3840x2160 30 libx264
fixture 2160p30_hevc 3840x2160 30 libx265
# Moving grain at a phone camera's 80 Mb/s makes every frame expensive to decode.
fixture 2160p60_grain 3840x2160 60 libx264 "noise=alls=16:allf=t" "-b:v 80M -maxrate 80M -bufsize 160M"

# case -> media file, project fps, preview height (0 = full size from the original),
# proxy (none, audio = proxy with sound, silent = proxy without, so sound comes from the original)
spec() {
    case $1 in
        1080p30) echo "1080p30 30 540 none" ;;
        1080p60) echo "1080p60 60 540 none" ;;
        2160p30) echo "2160p30 30 540 none" ;;
        2160p30_hevc) echo "2160p30_hevc 30 540 none" ;;
        1080p30_full) echo "1080p30 30 0 none" ;;
        2160p60_grain) echo "2160p60_grain 60 540 none" ;;
        2160p60_grain_proxy) echo "2160p60_grain 60 540 audio" ;;
        2160p60_grain_proxy_silent) echo "2160p60_grain 60 540 silent" ;;
        *) echo "unknown case $1" >&2; exit 1 ;;
    esac
}

median() { sort -n | awk '{a[NR]=$1} END {print a[int((NR+1)/2)]}'; }

waits=$(printf 'dump;%.0s' $(seq 20)) # 3 s for the project to open
printf '| case | preview | hw decode | played s | dropped (median) | dropped %% | peak RSS MB | CPU %% | canvas draws |\n'
printf '|---|---|---|---|---|---|---|---|---|\n'
for name in $CASES; do
    read -r file fps height proxy < <(spec "$name")
    project="$work/$name.tempo"
    rm -f "$project"
    if [ "$proxy" = none ]; then
        TEMPO_DEMO_FPS=$fps cargo run -q -p tempo-app --example make_demo_project -- "$project" "$media/$file.mp4" >/dev/null
    else
        TEMPO_DEMO_PROXY=$proxy TEMPO_DEMO_FPS=$fps cargo run -q -p tempo-app --example make_demo_project -- "$project" "$media/$file.mp4" >/dev/null
    fi
    drops=() rss=() cpu=() played=() draws=()
    for _ in $(seq "$RUNS"); do
        xdg="$work/xdg"
        rm -rf "$xdg"
        mkdir -p "$xdg/config/tempo"
        printf '{"playback_height": %s, "auto_proxies": false, "hardware_decode": %s}\n' "$height" "$HW" >"$xdg/config/tempo/settings.json"
        /usr/bin/time -v -o "$work/time.txt" env XDG_CONFIG_HOME="$xdg/config" XDG_DATA_HOME="$xdg/data" XDG_CACHE_HOME="$xdg/cache" \
            TEMPO_NO_PROXY=1 TEMPO_ACTIONS="${waits}win.play-toggle" TEMPO_SCREENSHOT="$work/$name.png" TEMPO_SCREENSHOT_DELAY_MS=14000 \
            RUST_LOG=info target/debug/tempo "$project" >"$work/log.txt" 2>&1 || true
        stats=$(grep -o 'stats .*' "$work/log.txt" | tail -1)
        [ -n "$stats" ] || { echo "no stats for $name; see $work/log.txt" >&2; exit 1; }
        get() { sed -n "s/.*$1=\([0-9]*\).*/\1/p" <<<"$stats"; }
        drops+=("$(get dropped_frames)")
        played+=("$(get position_us)")
        draws+=("$(get canvas_draws)")
        rss+=("$(awk -F': ' '/Maximum resident/ {print int($2/1024)}' "$work/time.txt")")
        cpu+=("$(awk -F': ' '/Percent of CPU/ {sub("%","",$2); print $2}' "$work/time.txt")")
    done
    d=$(printf '%s\n' "${drops[@]}" | median)
    p=$(printf '%s\n' "${played[@]}" | median)
    pct=$(awk -v d="$d" -v p="$p" -v f="$fps" 'BEGIN {n = p / 1e6 * f; printf "%.1f", (n > 0 ? 100 * d / n : 0)}')
    printf '| %s | %s | %s | %.1f | %s | %s | %s | %s | %s |\n' "$name" "$([ "$height" = 0 ] && echo Full || echo "${height}p")" "$HW" \
        "$(awk -v p="$p" 'BEGIN {print p / 1e6}')" "$d" "$pct" \
        "$(printf '%s\n' "${rss[@]}" | median)" "$(printf '%s\n' "${cpu[@]}" | median)" "$(printf '%s\n' "${draws[@]}" | median)"
done
