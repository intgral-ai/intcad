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


@pytest.fixture
def x11_shape_page(page, site):
    page.set_viewport_size({"width": 520, "height": 300})
    page.add_init_script(path=HERE / "host.js")
    page.add_init_script("window.__assistantX11Shape = true;")
    page.goto(site)
    expect(page.get_by_role("textbox", name="Message")).to_be_visible()
    show(page, centre=300, items=[{"kind": "notice", "text": "Status", "error": True}])
    return page


FOOTPRINT = """window.assistantFixture.actions.filter(action => action.type === 'painted-footprint')"""


def test_x11_shape_covers_busy_glow_without_covering_the_viewport(x11_shape_page):
    page = x11_shape_page
    last = f"{FOOTPRINT}.at(-1)"
    ring = page.locator('[data-slot="composer-ring"]')

    def exact_ring():
        return ring.evaluate("""el => {
            const {left, top, right, bottom} = el.getBoundingClientRect();
            const style = getComputedStyle(el);
            const radii = ['borderTopLeftRadius', 'borderTopRightRadius', 'borderBottomRightRadius',
              'borderBottomLeftRadius'].flatMap(name => [parseFloat(style[name]), parseFloat(style[name])]);
            return [left, top, right, bottom, ...radii];
        }""")

    page.wait_for_function(f"() => {last}?.surfaces.length === 2", timeout=3000)
    idle_ring = exact_ring()
    assert page.evaluate(f"() => {last}").get("surfaces")[-1] == idle_ring
    show(page, centre=300, state="busy")
    expect(ring).to_have_attribute("data-busy", "true")
    # Once the transition reaches its authored shadow (0 4px 20px -4px), the footprint
    # must extend beyond the ring without including the empty upper viewport.
    page.wait_for_function(f"() => {last}?.surfaces.some(s => s[0] <= -20)", timeout=3000)
    footprint = page.evaluate(f"() => {last}")
    glow = footprint["surfaces"][-1]
    assert glow[0] <= idle_ring[0] - 20 and glow[2] >= idle_ring[2] + 20
    assert glow[1] < idle_ring[1] and glow[3] > idle_ring[3]
    assert all(s[1] > 0 for s in footprint["surfaces"]), footprint
    assert footprint["popup"] is None

    show(page, centre=300, items=[{"kind": "notice", "text": "Status", "error": True}])
    page.wait_for_function(f"expected => JSON.stringify({last}?.surfaces.at(-1)) === JSON.stringify(expected)",
                           arg=idle_ring, timeout=3000)
    page.emulate_media(forced_colors="active")
    page.wait_for_function(f"() => {last}?.surfaces.some(s => s[0] === -1)", timeout=3000)
    assert all(s[1] > 0 for s in page.evaluate(f"() => {last}").get("surfaces"))


def test_x11_shape_retries_menu_ack_and_fails_open(x11_shape_page):
    page = x11_shape_page
    page.wait_for_function(f"() => {FOOTPRINT}.at(-1)?.surfaces.length === 2", timeout=3000)
    page.evaluate("""() => {
      const send = window.ipc.postMessage;
      window.ipc.postMessage = raw => {
        const message = JSON.parse(raw);
        if (message.type === 'painted-footprint' && message.popup) {
          window.ipc.postMessage = send;
          throw Error('one lost popup request');
        }
        send(raw);
      };
    }""")
    edits = page.get_by_role("button", name="modeAsk", exact=True)
    edits.click()
    menu = page.locator('[role="menu"]')
    menu.wait_for(state="attached")
    token = page.wait_for_function(f"() => {FOOTPRINT}.at(-1)?.popup", timeout=3000).json_value()
    expect(menu).to_have_attribute("data-centre-wait", "")
    # Drop one request above, then withhold the first ack. The same token must be retried.
    page.wait_for_function(f"token => {FOOTPRINT}.filter(action => action.popup === token).length >= 2", arg=token, timeout=3000)
    page.evaluate("token => window.__assistantX11Unmasked(token)", token)
    expect(menu).not_to_have_attribute("data-centre-wait", "")
    page.keyboard.press("Escape")
    page.wait_for_function(f"() => {FOOTPRINT}.at(-1)?.popup === null", timeout=3000)

    edits.click()
    menu.wait_for(state="attached")
    next_token = page.wait_for_function(f"token => {FOOTPRINT}.at(-1)?.popup > token && {FOOTPRINT}.at(-1).popup",
                                       arg=token, timeout=3000).json_value()
    page.evaluate("token => window.__assistantX11Unmasked(token)", token)
    expect(menu).to_have_attribute("data-centre-wait", "")
    # No ack for the new token: the portal must still become interactive.
    expect(menu).not_to_have_attribute("data-centre-wait", "", timeout=4000)
    expect(menu).to_have_css("pointer-events", "auto")
    assert page.evaluate(f"token => {FOOTPRINT}.filter(action => action.popup === token).length", next_token) >= 2


def test_x11_shape_samples_square_clipped_edge_and_skips_unresolved_radii(x11_shape_page):
    page = x11_shape_page
    page.wait_for_function(f"() => {FOOTPRINT}.at(-1)?.surfaces.length === 2", timeout=3000)
    page.evaluate("""() => {
      const surface = document.querySelector('[data-slot="composer-header-surface"]');
      surface.style.borderRadius = '20px';
      surface.parentElement.style.height = '20px';
      window.dispatchEvent(new Event('resize'));
    }""")
    footprint = page.evaluate(f"() => {FOOTPRINT}.at(-1)")
    assert footprint["surfaces"][0][4:] == [20, 20, 20, 20, 0, 0, 0, 0], footprint
    page.evaluate("""() => {
      document.querySelector('[data-slot="composer-header-surface"]').style.borderRadius = 'calc(20% + 2px)';
      window.dispatchEvent(new Event('resize'));
    }""")
    footprint = page.evaluate(f"() => {FOOTPRINT}.at(-1)")
    assert len(footprint["surfaces"]) == 1 and None not in footprint["surfaces"][0], footprint


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


COMMANDS = [
    {"name": "skill/solidcraft-modeling", "description": "Build a part"},
    {"name": "skill/solidcraft-editing", "description": "Change a design"},
]


def test_slash_menu_offers_and_inserts_advertised_commands(assistant):
    box = assistant.get_by_role("textbox", name="Message")
    show(assistant, commands=COMMANDS)
    box.fill("/")
    modeling = assistant.get_by_role("option", name="skill/solidcraft-modeling")
    editing = assistant.get_by_role("option", name="skill/solidcraft-editing")
    expect(modeling).to_be_visible()
    expect(editing).to_be_visible()

    # Selecting a command writes its name, keeps focus, closes the menu and sends nothing.
    modeling.click()
    expect(box).to_have_value("skill/solidcraft-modeling")
    expect(box).to_be_focused()
    expect(modeling).to_have_count(0)
    assert actions(assistant) == []

    # With the menu closed Enter sends the draft.
    box.press("Enter")
    assert actions(assistant) == [{"type": "send", "text": "skill/solidcraft-modeling"}]

    # Typing filters by the text after `/`, case-sensitively.
    box.fill("/skill/solidcraft-m")
    expect(modeling).to_be_visible()
    expect(editing).to_have_count(0)

    # Up/Down move the highlight; Enter chooses the highlighted one.
    box.fill("/")
    expect(modeling).to_have_attribute("aria-selected", "true")
    box.press("ArrowDown")
    expect(editing).to_have_attribute("aria-selected", "true")
    expect(modeling).to_have_attribute("aria-selected", "false")
    box.press("Enter")
    assert actions(assistant) == []
    expect(box).to_have_value("skill/solidcraft-editing")
    expect(box).to_be_focused()
    expect(editing).to_have_count(0)

    # Escape closes the menu and keeps the draft; a slash with no match shows nothing.
    box.fill("/")
    expect(modeling).to_be_visible()
    box.press("Escape")
    expect(modeling).to_have_count(0)
    expect(box).to_have_value("/")
    assert actions(assistant) == []
    box.fill("/no-such-command")
    expect(modeling).to_have_count(0)
    box.fill("not a command")
    expect(modeling).to_have_count(0)


@pytest.mark.parametrize("combo", ["Shift+Enter", "Control+Enter", "Control+Shift+Enter"])
def test_every_enter_variant_selects_while_the_slash_menu_is_open(assistant, combo):
    box = assistant.get_by_role("textbox", name="Message")
    show(assistant, commands=COMMANDS)
    box.fill("/")
    expect(assistant.get_by_role("option", name="skill/solidcraft-modeling")).to_be_visible()
    box.press(combo)
    assert actions(assistant) == [], f"{combo} sent instead of selecting"
    expect(box).to_have_value("skill/solidcraft-modeling")
    expect(box).to_be_focused()


def test_slash_menu_ignores_repeated_and_composing_enter(assistant):
    box = assistant.get_by_role("textbox", name="Message")
    show(assistant, commands=COMMANDS)
    box.fill("/")
    option = assistant.get_by_role("option", name="skill/solidcraft-modeling")
    expect(option).to_be_visible()
    box.press("Enter")
    expect(box).to_have_value("skill/solidcraft-modeling")
    assert actions(assistant) == []
    # A held Enter repeats keydowns; a repeat must never send the inserted draft.
    box.dispatch_event("keydown", {"key": "Enter", "repeat": True})
    assert actions(assistant) == []
    expect(box).to_have_value("skill/solidcraft-modeling")

    # Enter during IME composition neither selects nor sends, and must not be consumed.
    def press_composing_enter(repeat):
        return box.evaluate(
            """(el, repeat) => {
                const event = new KeyboardEvent('keydown', {key: 'Enter', repeat, isComposing: true,
                    bubbles: true, cancelable: true});
                el.dispatchEvent(event);
                return event.defaultPrevented;
            }""",
            repeat,
        )

    box.fill("/")
    expect(option).to_be_visible()
    assert press_composing_enter(False) is False, "a composing Enter must reach the IME"
    assert press_composing_enter(True) is False, "a repeated composing Enter must reach the IME"
    expect(box).to_have_value("/")
    expect(option).to_be_visible()
    assert actions(assistant) == []
    # Escape and arrows during composition also leave the menu and highlight alone.
    box.dispatch_event("keydown", {"key": "Escape", "isComposing": True})
    expect(option).to_be_visible()
    box.dispatch_event("keydown", {"key": "ArrowDown", "isComposing": True})
    expect(option).to_have_attribute("aria-selected", "true")


def test_slash_menu_options_are_arrow_only_and_keep_the_highlight_visible(assistant):
    box = assistant.get_by_role("textbox", name="Message")
    many = [{"name": f"skill/command-{n:02d}", "description": f"Command {n}"} for n in range(20)]
    show(assistant, commands=many)
    box.fill("/")
    first = assistant.get_by_role("option", name="skill/command-00")
    expect(first).to_be_visible()
    # Arrow-only navigation: a focused textarea moves the highlight; the options are not Tab stops.
    expect(first).to_have_attribute("tabindex", "-1")
    for _ in range(15):
        box.press("ArrowDown")
    highlighted = assistant.get_by_role("option", name="skill/command-15")
    expect(highlighted).to_have_attribute("aria-selected", "true")
    # The highlighted row is scrolled into the list's viewport.
    assert highlighted.evaluate(
        """el => {
            const list = el.closest('[role=listbox]').getBoundingClientRect();
            const row = el.getBoundingClientRect();
            return row.top >= list.top - 1 && row.bottom <= list.bottom + 1;
        }"""
    )


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
