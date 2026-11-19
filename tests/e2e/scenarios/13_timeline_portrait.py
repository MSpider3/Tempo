"""Create New Timeline with Use Project Settings off: 1080x1920 at 25 fps; the state has them and the viewer picture is portrait."""

from tempo_e2e import center, expect, expect_eq


def black_extent(image, rect):
    """Width and height of the black picture through the middle of `rect`."""
    x, y, w, h = (int(v) for v in rect)
    cx, cy = x + w // 2, y + h // 2
    dark = lambda px: sum(px) < 12  # noqa: E731
    width = sum(dark(image.pixel(i, cy)) for i in range(x, x + w))
    height = sum(dark(image.pixel(cx, j)) for j in range(y, y + h))
    return width, height


def run(s):
    s.key("ctrl+n")
    s.widget(text="Create New Timeline", type="GtkLabel")
    s.fill(s.row_widget("Timeline Name", "GtkEntry"), "Portrait", enter=False)
    switch = s.row_widget("Use Project Settings", "GtkSwitch")
    expect(switch["active"], "Use Project Settings starts on")
    s.click(*center(switch["rect"]))
    s.fill(s.row_widget("Width", "GtkSpinButton"), "1080", enter=False)
    s.fill(s.row_widget("Height", "GtkSpinButton"), "1920", enter=False)
    s.click(*center(s.row_widget("Timeline Frame Rate", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="25", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.row_widget("Timeline Frame Rate", "GtkDropDown")["text"] == "25", "25 to be picked")
    s.screenshot("1_dialog")
    s.click(*center(s.widget(text="Create")["rect"]))
    s.wait_for(lambda: s.timeline()["name"] == "Portrait", "the new timeline to open")

    timeline = s.timeline()
    expect_eq(timeline["fps"], {"num": 25, "den": 1}, "timeline frame rate")
    expect_eq(timeline["resolution"], [1080, 1920], "timeline size")
    expect_eq(s.state()["fps"], 25.0, "the app's frame rate")
    surface = s.widget(type="TempoVideoSurface")
    width, height = black_extent(s.screenshot("2_portrait"), surface["rect"])
    s.note("viewer_picture_px", [width, height])
    expect(height > width * 1.5, f"the viewer picture is portrait: {width}x{height} px")
