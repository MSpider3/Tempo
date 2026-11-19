"""Create New Timeline (Ctrl+N): Start Timecode 01:00:00:00, Audio Track Type Mono; the new timeline opens and its timecode reads 01:00:00:00."""

from tempo_e2e import center, expect, expect_eq

NAME = "E2E Timeline"
ONE_HOUR_US = 3_600_000_000


def run(s):
    s.key("ctrl+n")
    s.widget(text="Create New Timeline", type="GtkLabel")
    s.fill(s.row_widget("Start Timecode", "GtkEntry"), "01:00:00:00", enter=False)
    s.fill(s.row_widget("Timeline Name", "GtkEntry"), NAME, enter=False)
    s.click(*center(s.row_widget("Audio Track Type", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="Mono", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.row_widget("Audio Track Type", "GtkDropDown")["text"] == "Mono", "Mono to be picked")
    s.screenshot("1_dialog")
    s.click(*center(s.widget(text="Create")["rect"]))
    s.wait_for(lambda: s.timeline()["name"] == NAME, "the new timeline to open")

    state = s.state()
    timeline = state["project"]["timeline"]
    expect_eq(timeline["start_tc_us"], ONE_HOUR_US, "start timecode")
    expect_eq(state["start_tc_us"], ONE_HOUR_US, "the app's start timecode")
    audio = [t for t in timeline["tracks"] if t["kind"] == "Audio"]
    expect(audio and all(t["audio_type"] == "Mono" for t in audio), f"audio tracks are mono: {[t.get('audio_type') for t in audio]}")
    expect_eq(state["playhead_tc"], "01:00:00:00", "playhead timecode")
    s.wait_for(lambda: s.widgets(text="01:00:00:00", type="GtkLabel"), "a timecode label reading 01:00:00:00")
    expect(any(t["name"] == "Timeline 1" for t in state["project"]["timelines"]), "Timeline 1 is still in the project")
    s.screenshot("2_created")
