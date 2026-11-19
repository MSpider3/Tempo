"""Markers: click the ruler, Ctrl+M, type a name and a keyword, Enter; the marker is saved with both (also on disk after Ctrl+S)."""

import sqlite3

from tempo_e2e import expect, expect_eq, expect_near

NAME = "Intro Scene"
KEYWORD = "chapter-one"


def run(s):
    s.fit_timeline()
    ruler = s.geometry()["timeline"]["ruler"]
    s.click(ruler[0] + ruler[2] * 0.2, ruler[1] + ruler[3] / 2)
    playhead = s.wait_for(lambda: s.state()["playhead_us"] or None, "the playhead to move")

    s.key("ctrl+m")
    s.widget(text="Markers", type="GtkLabel")
    # Ctrl+M selects the default name in the Name field: typing replaces it.
    s.type_text(NAME)
    s.fill(s.row_widget("Keyword", "GtkEntry"), KEYWORD, enter=False)
    s.screenshot("1_dialog")
    s.key("Return")
    s.wait_for(lambda: not s.widgets(text="Markers", type="GtkLabel"), "the dialog to close")

    markers = s.wait_for(lambda: s.timeline()["markers"], "a marker")
    expect_eq(len(markers), 1, "marker count")
    m = markers[0]
    expect_eq(m["name"], NAME, "marker name")
    expect_eq(m["keyword"], KEYWORD, "marker keyword")
    expect_near(m["position_us"], playhead, 34_000, "marker at the playhead")
    geo = s.geometry()["timeline"]
    expect(geo["ruler"][0] <= geo["markers"][0]["x"] <= geo["ruler"][0] + geo["ruler"][2], "the marker is on the visible ruler")
    s.screenshot("2_marker")

    # Saved to the project file.
    s.key("ctrl+s")
    s.wait_for(lambda: not s.state()["dirty"], "Ctrl+S to save", timeout=10)
    db = sqlite3.connect(f"file:{s.project_copy}?mode=ro", uri=True)
    try:
        dump = "\n".join(db.iterdump())
    finally:
        db.close()
    expect(KEYWORD in dump and NAME in dump, "the marker's name and keyword are in the saved project")
