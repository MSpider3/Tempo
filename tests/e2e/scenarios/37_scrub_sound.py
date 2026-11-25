"""(`AUDIO`) Audio scrubbing: with Shift+S on and playback stopped, stepping frames with Right plays a short burst of sound at each step; with it off the steps are silent."""

from e2e_audio import rms, windows
from tempo_e2e import CHECKOUT_BIN, expect

from _helpers import seek_by_ruler

AUDIO = True
PROJECT = str(CHECKOUT_BIN / "audio.tempo")
STEPS = 15


def step_and_record(s, name):
    """Step STEPS frames while recording; the number of 10 ms windows with sound in them."""
    rec = s.record_audio(name)
    for _ in range(STEPS):
        s.key("Right")
    left, right = rec.stop()
    return sum(1 for a, b in zip(windows(left, 0.01), windows(right, 0.01)) if max(rms(a), rms(b)) > 0.01)


def run(s):
    s.fit_timeline()
    seek_by_ruler(s, 1.0)
    quiet = step_and_record(s, "off")
    s.key("shift+s")
    loud = step_and_record(s, "on")
    s.note("sounding_10ms_windows_off_on", [quiet, loud])
    expect(not s.state()["playing"], "stepping does not start playback")
    expect(quiet == 0, f"without scrubbing the steps are silent: {quiet} windows with sound")
    # Each step plays 30 ms (three windows); most of the steps must be heard.
    expect(loud >= 2 * STEPS, f"with scrubbing on the steps are heard: {loud} windows with sound for {STEPS} steps")
