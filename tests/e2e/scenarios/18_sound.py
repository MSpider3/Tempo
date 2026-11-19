"""Sound, recorded from the private PipeWire: a Cross Fade on A1 blends 330 Hz into 660 Hz, a mono clip plays centred (L = R), a gap is silent."""

import os

from e2e_audio import onset, rms, tone_level, windows
from _helpers import SYSTEM_SOCKETS, environ, seek_by_ruler, select_cut, unix_peers
from tempo_e2e import CHECKOUT_BIN, center, expect, expect_eq

AUDIO = True
PROJECT = str(CHECKOUT_BIN / "audio.tempo")
LOW, HIGH, MONO = 330, 660, 440


def play_and_record(s, name, from_s, seconds):
    """Seek to `from_s`, play for about `seconds` and return the recorded [left, right]."""
    seek_by_ruler(s, from_s)
    rec = s.record_audio(name)
    s.key("space")
    s.wait_for(lambda: s.state()["playing"], "playback to start")
    s.wait_for(lambda: s.state()["playhead_us"] > (from_s + seconds) * 1e6, "the playhead to move on", timeout=seconds + 8)
    s.key("space")
    s.wait_for(lambda: not s.state()["playing"], "playback to stop")
    return rec.stop()


def run(s):
    s.fit_timeline()
    # The app has sound on and its only way out is the private PipeWire (as 09_isolation proves for the default mode).
    app_env = environ(s.pid("app"))
    expect("TEMPO_NO_AUDIO" not in app_env, "sound is on in this scenario")
    expect(app_env["XDG_RUNTIME_DIR"] == str(s.run_dir) and "PIPEWIRE_REMOTE" not in app_env, "the app reaches PipeWire only through the private runtime dir")
    peers = unix_peers(s.pid("app"))
    s.note("app_unix_peers", peers)
    expect(f"{s.run_dir}/pipewire-0" in peers, f"the app plays into the private PipeWire: {peers}")
    allowed = tuple(p.format(uid=os.getuid()) for p in SYSTEM_SOCKETS)
    outside = [p for p in peers if not p.startswith(str(s.tmp)) and not p.startswith(allowed)]
    expect(not outside, f"the app talks only to private sockets, not {outside}")

    # 1. Cross Fade (audio only) on the first cut (4 s).
    select_cut(s, 0)
    s.key("shift+t")
    fade = s.wait_for(lambda: s.transitions("A1"), "Shift+T to add the Cross Fade")[0]
    expect_eq(fade["kind"], "Crossfade", "audio transition kind")
    geo = s.transition_geo("A1", 0)
    length = (geo["end_us"] - geo["start_us"]) / 1e6
    start = geo["start_us"] / 1e6
    lead = 1.0
    left, right = play_and_record(s, "crossfade", max(start - lead, 0.0), length + 2 * lead)
    x = left
    t0 = onset(x)
    expect(t0 is not None, "the recording has sound in it")
    x = x[t0:]
    rows = [(tone_level(w, LOW), tone_level(w, HIGH)) for w in windows(x)]
    s.note("crossfade_levels_330_660", [(round(a, 4), round(b, 4)) for a, b in rows])
    while rows and rows[-1][0] + rows[-1][1] < 0.02:  # the recorder runs a little past the stop
        rows.pop()
    rows = rows[:-1]  # the last window is where playback stopped: often only part sound
    full_low, full_high = max(a for a, _ in rows), max(b for _, b in rows)
    expect(full_low > 0.02 and full_high > 0.02, f"both tones were heard: {full_low:.3f}, {full_high:.3f}")
    low = [a / full_low for a, _ in rows]
    high = [b / full_high for _, b in rows]
    expect(high[0] < 0.1 and low[0] > 0.8, f"it starts on the first clip only: low {low[0]:.2f}, high {high[0]:.2f}")
    expect(low[-1] < 0.1 and high[-1] > 0.8, f"it ends on the second clip only: low {low[-1]:.2f}, high {high[-1]:.2f}")
    expect(any(a > 0.15 and b > 0.15 for a, b in zip(low, high)), f"both blend somewhere in the middle: {list(zip(low, high))}")
    expect(all(b >= a - 0.2 for a, b in zip(high, high[1:])), f"the second tone only rises: {high}")
    expect(all(b <= a + 0.2 for a, b in zip(low, low[1:])), f"the first tone only falls: {low}")
    # The audio Cross Fade is equal power (+3 dB, as Resolve's): the squares of the two levels add up to
    # about 1 all through the blend, so the loudness neither dips nor jumps (generous bounds).
    power = [round(a * a + b * b, 2) for a, b in zip(low, high) if 0.15 < a < 0.85]
    s.note("crossfade_power_sums", power)
    expect(power and all(0.7 < p < 1.3 for p in power), f"the blend keeps its power: {power}")

    # 2. The mono clip (8-12 s) plays centred.
    left, right = play_and_record(s, "mono", 9.0, 1.5)
    t0 = onset(left, 0.01)
    expect(t0 is not None, "the mono clip made sound")
    body_l, body_r = left[t0 + 4800:t0 + 48000], right[t0 + 4800:t0 + 48000]
    level_l, level_r = tone_level(body_l, MONO), tone_level(body_r, MONO)
    s.note("mono_levels_lr", [round(level_l, 4), round(level_r, 4), round(rms(body_l), 4), round(rms(body_r), 4)])
    expect(level_l > 0.02 and level_r > 0.02, f"the 440 Hz tone is on both sides: {level_l:.3f}, {level_r:.3f}")
    expect(abs(level_l - level_r) < 0.1 * max(level_l, level_r), f"left and right are as loud: {level_l:.3f} vs {level_r:.3f}")
    diff = rms([a - b for a, b in zip(body_l, body_r)])
    expect(diff < 0.1 * rms(body_l), f"left and right carry the same signal: difference {diff:.4f}")

    # 3. Lift the second clip (4-8 s): the gap is silent.
    s.click(*center(s.clip("V1", 1)["rect"]))
    s.key("BackSpace")
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 2 and len(s.track("A1")["clips"]) == 2, "Backspace to lift the clip")
    left, right = play_and_record(s, "gap", 5.0, 1.2)
    quiet = max(rms(left), rms(right))
    s.note("gap_rms", round(quiet, 5))
    expect(quiet < 0.002, f"the gap is silent: rms {quiet:.5f}")
