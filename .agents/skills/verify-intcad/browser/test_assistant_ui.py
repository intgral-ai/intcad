"""User actions on the bundled Assistant page, with a fixture at the desktop IPC boundary."""

from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread

import pytest
from playwright.sync_api import expect


HERE = Path(__file__).resolve().parent


@pytest.fixture(scope="session")
def site():
    dist = HERE.parents[3] / "packages" / "assistant-web" / "dist"
    with ThreadingHTTPServer(("127.0.0.1", 0), partial(SimpleHTTPRequestHandler, directory=dist)) as server:
        worker = Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            yield f"http://127.0.0.1:{server.server_port}"
        finally:
            server.shutdown()
            worker.join()


@pytest.fixture(scope="session")
def browser_context_args(browser_context_args):
    # Exercise the width of a docked panel, including wrapping and scroll-to-control.
    return {**browser_context_args, "viewport": {"width": 380, "height": 700}}


@pytest.fixture
def assistant(page, site):
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.add_init_script(path=HERE / "host.js")
    page.goto(site)
    expect(page.get_by_role("textbox", name="Message")).to_be_visible()
    yield page
    assert not errors, errors


def show(page, **state):
    page.evaluate("state => window.assistantFixture.show(state)", state)


def actions(page):
    return page.evaluate("window.assistantFixture.actions.splice(0)")


def test_centred_composer_uses_same_scale_across_agent_types(assistant):
    assistant.set_viewport_size({"width": 520, "height": 700})
    agents = [
        {"id": "fixture", "name": "Fixture", "detail": "", "hint": None},
        {"id": "built-in", "name": "Built-in", "detail": "", "hint": None},
    ]
    connect = {"plans": [], "keys": [], "custom": [], "login": None, "problem": None,
               "error": None, "keySaves": 0, "dates": {}, "connected": True}
    root = assistant.locator("html")
    box = assistant.get_by_role("textbox", name="Message")
    show(assistant, centre=600, agents=agents, agent="fixture")
    expect(assistant.locator("html.centre:not([data-sizes])")).to_have_count(1)
    box.fill("Inspect this")
    expect(box).to_be_focused()

    def sizing():
        return box.evaluate("""input => ({
            root: getComputedStyle(document.documentElement).fontSize,
            input: getComputedStyle(input).fontSize,
            height: input.closest('[data-slot="composer-box"]').getBoundingClientRect().height,
        })""")

    external = sizing()
    # Acknowledge agent selection through the desktop state bridge while the draft has focus.
    show(assistant, centre=600, agents=agents, agent="built-in", connect=connect)
    expect(assistant.locator('html.centre[data-sizes="mastra"]')).to_have_count(1)
    expect(box).to_have_value("Inspect this")
    expect(box).to_be_focused()
    built_in = sizing()

    assistant.set_viewport_size({"width": 380, "height": 700})
    show(assistant, agents=agents, agent="fixture")
    expect(assistant.locator("html:not(.centre):not([data-sizes])")).to_have_count(1)
    expect(root).to_have_css("font-size", "20px")

    assert external["root"] == "16px", external
    assert built_in["root"] == "16px", built_in
    assert external["input"] == "14px", external
    assert built_in["input"] == "14px", built_in
    assert abs(external["height"] - built_in["height"]) <= 1, (external, built_in)


@pytest.mark.parametrize(("choice", "allow", "outcome"), [
    ("Deny", False, "Denied"), ("Allow once", True, "Allowed"), ("Always allow", "always", "Allowed"),
])
def test_tool_approval(assistant, choice, allow, outcome):
    item = {"kind": "approval", "id": 73, "title": "Change the plate?", "answer": None}
    show(assistant, items=[item])
    assistant.get_by_role("button", name=choice, exact=True).click()
    assert actions(assistant) == [{"type": "approve", "id": 73, "allow": allow}]

    show(assistant, items=[{**item, "answer": bool(allow)}])
    expect(assistant.get_by_text(outcome, exact=True)).to_be_visible()
    for label in ("Deny", "Allow once", "Always allow"):
        expect(assistant.get_by_role("button", name=label, exact=True)).to_have_count(0)


@pytest.mark.parametrize(("accept", "choice", "outcome"), [
    (False, "Reject changes", "Rejected"), (True, "Accept changes", "Accepted"),
])
def test_preview_decision(assistant, accept, choice, outcome):
    preview = {"kind": "review", "index": 3, "review": 81, "steps": ["Resize the plate"],
               "more": 0, "made": [], "changed": [], "removed": [], "outcome": "Waiting"}
    show(assistant, items=[preview])
    assistant.get_by_role("button", name=choice).click()
    assert actions(assistant) == [{"type": "review", "id": 81, "accept": accept}]

    show(assistant, items=[{**preview, "outcome": outcome}])
    expect(assistant.get_by_text(f"Changes {outcome.lower()}", exact=True)).to_be_visible()
    for label in ("Accept changes", "Reject changes"):
        expect(assistant.get_by_role("button", name=label)).to_have_count(0)


def test_dfm_finding_details_and_selection(assistant):
    hole = {"id": "F17", "body": "Plate", "state": "failed", "rules": ["hole_min_diameter"],
            "measured": {"value": 2, "unit": "mm"}, "requirement": {
                "value": 3, "unit": "mm", "layer": "factory", "source": "Shop profile"}}
    edge = {"id": "F29", "body": "Flange", "state": "unsupported", "rules": ["edge_clearance"],
            "reason": "No clearance threshold in the profile"}
    report = {"runId": "run-42", "status": "done", "freshness": "stale", "findings": [hole, edge],
              "counts": {"failed": 1, "advisory": 0, "checked": 1, "unsupported": 1}}
    card = {"kind": "check", "id": "card-9", "status": "done", "card": report, "display": "all",
            "selected": None}
    show(assistant, items=[card])
    expect(assistant.get_by_text("Out of date", exact=True)).to_be_visible()
    hole_row = assistant.get_by_role("button", name="hole_min_diameter", exact=False)
    edge_row = assistant.get_by_role("button", name="edge_clearance", exact=False)
    hole_row.click()
    assert actions(assistant) == [{"type": "finding", "id": "F17", "runId": "run-42"}]
    expect(hole_row).to_have_attribute("aria-expanded", "true")
    expect(assistant.get_by_text("Measured: 2 mm", exact=True)).to_be_visible()
    expect(assistant.get_by_text("Requirement: 3 mm · factory · Shop profile", exact=True)).to_be_visible()

    edge_row.click()
    assert actions(assistant) == [{"type": "finding", "id": "F29", "runId": "run-42"}]
    expect(hole_row).to_have_attribute("aria-expanded", "false")
    expect(edge_row).to_have_attribute("aria-expanded", "true")
    expect(assistant.get_by_text("Measured: 2 mm", exact=True)).to_have_count(0)
    expect(assistant.get_by_text(edge["reason"], exact=True)).to_be_visible()
    show(assistant, items=[{**card, "selected": {"runId": "run-42", "findingId": "F29"}}])
    expect(edge_row.get_by_text("Selected finding", exact=True)).to_be_visible()
    expect(hole_row.get_by_text("Selected finding", exact=True)).to_have_count(0)
    assistant.get_by_role("button", name="Selected only").click()
    assert actions(assistant) == [{"type": "dfm_display", "mode": "selected"}]


def test_attachment_loading_removal_and_file_only_send(assistant):
    box = assistant.get_by_role("textbox", name="Message")
    send = assistant.get_by_role("button", name="Send", exact=True)
    show(assistant, loading=["plate.step"])
    box.fill("Inspect this")
    expect(send).to_be_disabled()
    box.press("Enter")
    assert actions(assistant) == []
    expect(box).to_have_value("Inspect this")

    files = [{"name": name, "picture": None, "report": None} for name in ("plate.step", "notes.txt")]
    show(assistant, attachments=files)
    box.fill("")
    assistant.get_by_role("button", name="Remove notes.txt").click()
    assert actions(assistant) == [{"type": "remove_file", "index": 1}]
    show(assistant, attachments=files[:1])
    expect(assistant.get_by_text("notes.txt", exact=True)).to_have_count(0)
    expect(assistant.get_by_text("plate.step", exact=True)).to_be_visible()
    send.click()
    assert actions(assistant) == [{"type": "send", "text": ""}]


@pytest.mark.parametrize("route", ["button", "Escape"])
def test_stop_active_turn(assistant, route):
    stop = assistant.get_by_role("button", name="Stop", exact=True)
    show(assistant, state="busy")
    expect(stop).to_be_visible()
    if route == "button":
        stop.click()
    else:
        assistant.get_by_role("textbox", name="Message").press("Escape")
    assert actions(assistant) == [{"type": "stop"}]
    show(assistant, state="ready")
    expect(stop).to_have_count(0)
    assistant.get_by_role("textbox", name="Message").press("Escape")
    assert actions(assistant) == []


@pytest.mark.parametrize("confirm", [False, True], ids=["cancel", "confirm"])
def test_new_chat_confirmation(assistant, confirm):
    note = {"kind": "notice", "text": "Keep this machining note", "error": False}
    show(assistant, items=[note])
    new_chat = assistant.get_by_role("button", name="New chat", exact=True)
    new_chat.click()
    assert actions(assistant) == [{"type": "new_chat"}]

    show(assistant, items=[note], confirmNew=True)
    expect(assistant.get_by_text("Start a new conversation?", exact=True)).to_be_visible()
    expect(assistant.get_by_text(note["text"], exact=True)).to_be_visible()
    choice = new_chat.filter(has_text="New chat") if confirm else assistant.get_by_role("button", name="Cancel")
    choice.click()
    assert actions(assistant) == [{"type": "confirm_new", "yes": confirm}]
    show(assistant, items=[] if confirm else [note])
    expect(assistant.get_by_text("Start a new conversation?", exact=True)).to_have_count(0)
    expect(assistant.get_by_text(note["text"], exact=True)).to_have_count(0 if confirm else 1)
