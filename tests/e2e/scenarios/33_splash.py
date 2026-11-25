"""BTS-of-memes splash screen: verifies artwork canvas, BTS caption, Resolve tip, progress bar, and instant Skip."""

from tempo_e2e import center, expect

PROJECT = None
ENV = {"TEMPO_SHOW_SPLASH": "1"}


def run(s):
    s.wait_for(lambda: s.widgets(text="TEMPO", type="GtkLabel"), "splash brand title")
    s.widget(text="BEHIND THE SCENES", type="GtkLabel")
    s.widget(contains="BTS:", type="GtkLabel")
    s.widget(contains="Resolve Tip:", type="GtkLabel")
    s.widget(type="TempoSplashCanvas")
    s.widget(type="GtkProgressBar")
    skip_btn = s.widget(text="Skip", type="GtkButton")
    expect(skip_btn["sensitive"], "Skip button is sensitive")
    s.screenshot("1_splash")

    s.click(*center(skip_btn["rect"]))
    s.wait_for(lambda: s.widgets(text="Projects", type="GtkLabel"), "Project Manager after skipping splash")
    s.screenshot("2_after_skip")

