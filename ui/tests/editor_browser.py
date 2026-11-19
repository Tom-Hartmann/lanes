"""Run against Vite with a synthetic 10-second MP4; no real Tauri operations.
Usage: python ui/tests/editor_browser.py http://127.0.0.1:5179 /@fs/C:/.../fixture.mp4
Requires Python Playwright and Microsoft Edge.
"""
import sys
from pathlib import Path
from urllib.parse import quote
from playwright.sync_api import sync_playwright, expect

base, media = sys.argv[1:3]
url = f"{base}/tests/editor.html?media={quote(media)}"
output = Path(__file__).resolve().parents[2] / "target"

with sync_playwright() as p:
    browser = p.chromium.launch(channel="msedge", headless=True, args=["--autoplay-policy=no-user-gesture-required"])
    page = browser.new_page(viewport={"width": 1180, "height": 900})
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.goto(url)
    page.wait_for_load_state("networkidle")
    page.wait_for_function("document.querySelector('video')?.readyState >= 2")
    page.locator("video").evaluate("v => { v.pause(); v.currentTime = 3; }")
    page.wait_for_function("Math.abs(document.querySelector('video').currentTime - 3) < .01")
    page.get_by_role("button", name="Split at playhead", exact=True).click()
    expect(page.get_by_role("button", name="Remove segment 2", exact=True)).to_be_visible()
    page.locator("video").evaluate("v => { v.currentTime = 7; }")
    page.wait_for_function("document.querySelector('video').seeking === false")
    page.get_by_role("button", name="Split at playhead", exact=True).click()
    expect(page.get_by_role("button", name="Remove segment 3", exact=True)).to_be_visible()
    page.get_by_role("button", name="Remove segment 2", exact=True).click()
    expect(page.get_by_text("2 kept segments", exact=False)).to_be_visible()
    page.get_by_role("button", name="Undo", exact=True).click()
    expect(page.get_by_role("button", name="Remove segment 3", exact=True)).to_be_visible()
    page.get_by_role("button", name="Redo", exact=True).click()
    expect(page.get_by_role("button", name="Remove segment 3", exact=True)).to_have_count(0)
    page.get_by_role("button", name="Portrait 9:16", exact=True).click()
    expect(page.get_by_label("Left (%)")).to_have_value("34.18")
    expect(page.get_by_role("button", name="Create share copy", exact=True)).to_have_count(0)
    page.screenshot(path=str(output / "editor-crop.png"))

    # The real video advances through 0..3, jumps over 3..7, then stops at 10.
    page.get_by_role("button", name="Preview kept segments", exact=True).click()
    page.wait_for_function("document.querySelector('video').currentTime > 7", timeout=7000)
    page.wait_for_function("document.querySelector('video').paused", timeout=6000)
    assert page.locator("video").evaluate("v => v.currentTime") >= 9.9

    page.get_by_role("button", name="Save copy", exact=True).click()
    page.wait_for_function("window.__editorTest.opened === 'edited.mp4'")
    args = page.evaluate("window.__editorTest.calls.find(c => c.cmd === 'render_clip_edit').args")
    assert args["edit"]["segments"] == [[0, 3], [7, 10]], args
    assert abs(args["edit"]["crop"]["left"] - 0.341796875) < 1e-9

    # Persisted cuts and crop survive destroying/recreating the viewer.
    page.reload()
    page.wait_for_load_state("networkidle")
    expect(page.get_by_text("Draft restored and saved on this device", exact=True)).to_be_visible()
    expect(page.get_by_label("Left (%)")).to_have_value("34.18")
    expect(page.get_by_role("button", name="Remove segment 2", exact=True)).to_have_count(1)
    page.get_by_label("Fade in seconds", exact=True).fill("0.5")
    page.get_by_label("Fade out seconds", exact=True).fill("0.7")
    page.get_by_role("button", name="Move segment 2 earlier", exact=True).click()
    page.get_by_label("Cut crossfade", exact=True).select_option("0.25")
    page.get_by_role("button", name="Save copy", exact=True).click()
    page.wait_for_function("window.__editorTest.opened === 'edited.mp4'")
    args = page.evaluate("window.__editorTest.calls.find(c => c.cmd === 'render_clip_edit').args")
    assert args["edit"]["segments"] == [[7, 10], [0, 3]]
    assert args["edit"]["audio"]["fadeIn"] == .5
    assert args["edit"]["audio"]["fadeOut"] == .7
    assert args["edit"]["audio"]["crossfade"] == .25
    page.get_by_role("button", name="Reset edits", exact=True).click()
    handle = page.get_by_role("button", name="Drag left crop edge", exact=True)
    handle.scroll_into_view_if_needed()
    box = handle.bounding_box()
    page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    page.mouse.down()
    page.mouse.move(box["x"] + box["width"] / 2 + 25, box["y"] + box["height"] / 2)
    page.mouse.up()
    assert float(page.get_by_label("Left (%)").input_value()) > 0
    page.get_by_role("button", name="Reset edits", exact=True).click()

    # Regression: a programmatic seek must not cancel selection preview.
    page.reload()
    page.wait_for_load_state("networkidle")
    page.wait_for_function("document.querySelector('video')?.readyState >= 2")
    page.locator("video").evaluate("v => { v.pause(); v.currentTime = 2; }")
    page.wait_for_function("document.querySelector('video').seeking === false")
    page.get_by_role("button", name="Set to playhead", exact=True).nth(0).click()
    page.locator("video").evaluate("v => { v.currentTime = 3; }")
    page.wait_for_function("document.querySelector('video').seeking === false")
    page.get_by_role("button", name="Set to playhead", exact=True).nth(1).click()
    page.get_by_role("button", name="Preview selection", exact=True).click()
    page.wait_for_function("document.querySelector('video').paused && document.querySelector('video').currentTime >= 2.9", timeout=5000)
    assert page.locator("video").evaluate("v => v.currentTime") <= 3.1

    page.goto(url + "&cancel=1")
    page.wait_for_load_state("networkidle")
    page.wait_for_function("document.querySelector('video')?.readyState >= 2")
    page.get_by_role("button", name="Square", exact=True).click()
    page.get_by_role("button", name="Save copy", exact=True).click()
    expect(page.get_by_role("progressbar", name="Export progress")).to_have_attribute("value", "0.4")
    page.get_by_role("button", name="Cancel export", exact=True).click()
    expect(page.get_by_role("button", name="Save copy", exact=True)).to_be_enabled()
    assert page.evaluate("window.__editorTest.calls.some(c => c.cmd === 'cancel_clip_edit')")
    page.set_viewport_size({"width": 820, "height": 520})
    page.get_by_label("Left (%)").scroll_into_view_if_needed()
    page.screenshot(path=str(output / "editor-compact.png"))
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
    page.goto(url)
    page.wait_for_load_state("networkidle")
    page.get_by_role("button", name="Render preview", exact=True).click()
    expect(page.get_by_role("dialog", name="Rendered edit preview")).to_be_visible()
    page.get_by_role("dialog").locator("video").evaluate("v => v.pause()")
    assert page.evaluate("window.__editorTest.calls.some(c => c.cmd === 'render_clip_edit' && c.args.preview)")
    page.get_by_role("button", name="Close preview", exact=True).click()
    expect(page.get_by_role("dialog", name="Rendered edit preview")).to_have_count(0)
    assert not errors, errors
    browser.close()
    print("PASS: split/remove, undo/redo, crop, export payload, preview sequence, selection preview, cancellation, draft recovery, fades, reordering, crop dragging, compact layout")
