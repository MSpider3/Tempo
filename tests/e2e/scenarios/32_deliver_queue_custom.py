"""Deliver page: Render 1 vs Render All selection/deselection, Custom resolution, MKV container, and YouTube -14 LUFS normalization."""

import json
import subprocess

from tempo_e2e import center, expect, expect_eq


def job_cards(s, geo=None):
    return s.widgets(css="job-card", geometry=geo)


def run(s):
    s.click(*center(s.widget(tooltip="Export page")["rect"]))
    s.wait_for(lambda: s.widgets(contains="Render Settings"), "the Export page")

    # UploadForm stays hidden when no Lua uploader plugin is installed.
    expect(not s.widgets(contains="Upload directly to"), "UploadForm is hidden without an uploader plugin")

    # Updated preset labels and Custom card exist in the format strip.
    geo = s.geometry()
    expect(s.widgets(text="YouTube 1080p, Vimeo", type="GtkLabel", geometry=geo, hidden=True), "16:9 YouTube 1080p label")
    expect(s.widgets(text="YouTube 2160p", type="GtkLabel", geometry=geo, hidden=True), "16:9 4K YouTube 2160p label")
    expect(s.widgets(text="Custom", type="GtkLabel", geometry=geo, hidden=True), "Custom preset card")
    expect(not s.widgets(text="Width", type="GtkLabel"), "Width is hidden when a built-in preset is active")

    out_dir = s.tmp / "home" / "renders"
    out_dir.mkdir()
    s.fill(s.row_widget("Location", "GtkEntry"), str(out_dir))

    # Job 1: default 16:9 MP4.
    s.fill(s.row_widget("File Name", "GtkEntry"), "first_mp4", enter=False)
    s.click(*center(s.widget(text="Add to Render Queue")["rect"]))
    s.wait_for(lambda: s.widgets(text="Job 1"), "Job 1 to appear in queue")
    expect(s.widgets(text="Render All", type="GtkButton"), "button reads Render All when no job is selected")

    # Changing Format switches preset to Custom and reveals Width/Height.
    s.click(*center(s.row_widget("Format", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="MKV", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.row_widget("Format", "GtkDropDown")["text"] == "MKV", "Format to switch to MKV")
    s.wait_for(lambda: s.widgets(contains="Render Settings - Custom"), "switching Format selects Custom preset")
    s.wait_for(lambda: s.widgets(text="Width", type="GtkLabel") and s.widgets(text="Height", type="GtkLabel"), "Width and Height fields")

    # Selecting a built-in preset resets Format/Codec/Audio/Normalize back to defaults.
    preset_16_9 = s.ancestor(s.widget(text="16:9", type="GtkLabel"), "GtkToggleButton", s.geometry())
    s.click(*center(preset_16_9["rect"]))
    s.wait_for(lambda: s.row_widget("Format", "GtkDropDown")["text"] == "MP4", "16:9 preset resets Format to MP4")
    s.wait_for(lambda: not s.widgets(text="Width", type="GtkLabel"), "16:9 preset hides Width/Height")

    # Scroll the preset strip horizontally to click Custom directly.
    strip_scroll = s.ancestor(s.widget(text="16:9", type="GtkLabel"), "GtkScrolledWindow", s.geometry())
    sx, sy, sw, _ = strip_scroll["rect"]
    for _ in range(15):
        geo = s.geometry()
        matches = s.widgets(text="Custom", type="GtkLabel", geometry=geo, hidden=True)
        if matches:
            btn = s.ancestor(matches[0], "GtkToggleButton", geo)
            bx, _, bw, _ = btn["rect"]
            if bx >= sx and bx + bw <= sx + sw:
                break
        s.scroll(*center(strip_scroll["rect"]), 4, horizontal=True)
        s.settle(quiet=0.1)
    custom_btn = s.ancestor(s.widget(text="Custom", type="GtkLabel"), "GtkToggleButton", s.geometry())
    s.click(*center(custom_btn["visible_rect"]))
    s.wait_for(lambda: s.widgets(text="Width", type="GtkLabel"), "Custom card shows Width")

    # Configure Custom 640x360 + MKV + H.265 + FLAC + YouTube (-14 LUFS).
    s.fill(s.row_widget("Width", "GtkSpinButton"), "640")
    s.fill(s.row_widget("Height", "GtkSpinButton"), "360")
    s.wait_for(lambda: s.widgets(contains="Render Settings - Custom - 640 × 360"), "Custom title updates to 640 x 360")

    s.click(*center(s.row_widget("Format", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="MKV", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.row_widget("Format", "GtkDropDown")["text"] == "MKV", "MKV selected")

    s.click(*center(s.row_widget("Video Codec", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="H.265", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.row_widget("Video Codec", "GtkDropDown")["text"] == "H.265", "H.265 selected")

    s.click(*center(s.row_widget("Audio Codec", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="FLAC", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.row_widget("Audio Codec", "GtkDropDown")["text"] == "FLAC", "FLAC selected")

    s.click(*center(s.row_widget("Normalize", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="YouTube (-14 LUFS)", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.row_widget("Normalize", "GtkDropDown")["text"] == "YouTube (-14 LUFS)", "YouTube -14 LUFS selected")

    s.fill(s.row_widget("File Name", "GtkEntry"), "custom_mkv", enter=False)
    s.click(*center(s.widget(text="Add to Render Queue")["rect"]))
    s.wait_for(lambda: s.widgets(text="Job 2"), "Job 2 to appear in queue")
    s.screenshot("1_two_jobs")

    # Select Job 2 -> button becomes "Render 1" and card gets .selected.
    cards = job_cards(s)
    expect_eq(len(cards), 2, "two job cards in queue")
    s.click(*center(cards[1]["rect"]))
    s.wait_for(lambda: "selected" in job_cards(s)[1]["css"] and s.widgets(text="Render 1", type="GtkButton"), "Job 2 selected and button reads Render 1")
    s.screenshot("2_selected_render1")

    # Click empty space in the Render Queue scrolled area -> deselects and restores "Render All".
    qscroll = s.ancestor(s.widget(text="Job 1"), "GtkScrolledWindow", s.geometry())
    qx, qy, qw, qh = qscroll["rect"]
    s.click(qx + qw / 2, qy + qh - 20)
    s.wait_for(lambda: all("selected" not in c["css"] for c in job_cards(s)) and s.widgets(text="Render All", type="GtkButton"), "empty space click clears selection")

    # Select Job 2 again and click "Render 1": only Job 2 renders.
    s.click(*center(job_cards(s)[1]["rect"]))
    s.wait_for(lambda: s.widgets(text="Render 1", type="GtkButton"), "Render 1 active for Job 2")
    s.click(*center(s.widget(text="Render 1", type="GtkButton")["rect"]))
    s.wait_for(lambda: s.widgets(text="✓ Completed"), "Job 2 to complete", timeout=240)

    mkv_files = sorted(out_dir.glob("*.mkv"))
    mp4_files = sorted(out_dir.glob("*.mp4"))
    expect_eq(len(mkv_files), 1, "only Job 2 rendered to MKV")
    expect_eq(len(mp4_files), 0, "Job 1 stayed unrendered after Render 1")
    expect(s.widgets(text="Waiting"), "Job 1 still Waiting")
    s.wait_for(lambda: s.widgets(text="Render All", type="GtkButton"), "button returns to Render All after Render 1 completes")
    s.screenshot("3_render1_done")

    probe = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=format_name,duration:stream=codec_name,width,height,sample_rate", "-of", "json", str(mkv_files[0])],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    info = json.loads(probe)
    s.note("mkv_probe", info)
    expect("matroska" in info["format"]["format_name"], f"container is matroska: {info['format']}")
    duration = float(info["format"]["duration"])
    expect(11.0 < duration < 13.0, f"MKV duration matches 12 s timeline: {duration}")
    vstream = next(st for st in info["streams"] if "width" in st)
    astream = next(st for st in info["streams"] if "sample_rate" in st)
    expect_eq((vstream["codec_name"], vstream["width"], vstream["height"]), ("hevc", 640, 360), "MKV video stream")
    expect_eq((astream["codec_name"], astream["sample_rate"]), ("flac", "48000"), "MKV audio stream")

    # Now click "Render All" to verify remaining Job 1 renders to MP4 as well.
    s.click(*center(s.widget(text="Render All", type="GtkButton")["rect"]))
    s.wait_for(lambda: len(s.widgets(text="✓ Completed")) == 2, "Job 1 to complete on Render All", timeout=240)
    expect_eq(len(sorted(out_dir.glob("*.mp4"))), 1, "Job 1 rendered to MP4")
    s.screenshot("4_render_all_done")
