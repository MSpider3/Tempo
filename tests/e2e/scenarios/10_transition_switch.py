"""Inspector Video switch: off makes the dissolve play as a hard cut in the viewer (pixels), Ctrl+Z turns it back on."""

from _helpers import add_transition_at_cut, seek_by_ruler, viewer_colour
from tempo_e2e import center, expect


def run(s):
    s.open_panel("Inspector")
    s.fit_timeline()
    add_transition_at_cut(s, 0)  # red -> lime, 3.5-4.5 s
    seek_by_ruler(s, 4.25)  # three quarters in: mostly lime, some red
    mixed = viewer_colour(s, "1_dissolve_on")
    s.note("viewer_rgb_on", mixed)
    expect(mixed[0] > 30 and mixed[1] > 120, f"the dissolve mixes red into lime: {mixed}")

    switch = s.scroll_into_view(lambda: s.widget(type="GtkSwitch", tooltip="plays as a cut"))
    expect(switch["active"], "the switch starts on")
    s.click(*center(switch["rect"]))
    s.wait_for(lambda: s.transitions("V1")[0]["enabled"] is False, "the transition to be switched off")
    cut = viewer_colour(s, "2_switched_off")
    s.note("viewer_rgb_off", cut)
    expect(cut[0] < 15 and cut[1] > 230, f"switched off, the viewer shows the next clip alone (a cut): {cut}")
    expect(len(s.transitions("V1")) == 1, "the transition stays on the timeline")

    s.key("ctrl+z")
    s.wait_for(lambda: s.transitions("V1")[0]["enabled"] is True, "Ctrl+Z to switch it back on")
    back = viewer_colour(s, "3_undone")
    expect(abs(back[0] - mixed[0]) < 12 and abs(back[1] - mixed[1]) < 12, f"the dissolve is back: {back} vs {mixed}")
