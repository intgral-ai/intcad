"""Find the Assistant WebView2 page and its controls in a Cua Driver UIA tree.

The Windows desktop acceptance must click the page's own controls. A page rectangle
synthesised from the window's right edge is wrong: the floating Assistant stands
beside the right rail, and its top is below the ribbon. WebView2 publishes the real
page and its buttons through UIA; these helpers turn one ``get_window_state`` reply
into the page frame and the centre of its ``Close`` button.

They are pure functions over the JSON Cua returns, so they are testable without a
VM. The machine-local acceptance driver in ``.dev/e2e`` uses them.
"""

from __future__ import annotations

from typing import Any, Iterable, Mapping

Frame = list[int]


def _frame(value: Mapping[str, Any]) -> Frame:
    """A Cua element frame as ``[x, y, w, h]`` integers."""
    frame = value["frame"]
    return [int(frame[k]) for k in ("x", "y", "w", "h")]


def center(frame: Frame) -> tuple[int, int]:
    """The middle of ``[x, y, w, h]``, the point a real pointer click should reach."""
    x, y, w, h = frame
    return x + w // 2, y + h // 2


def inside(outer: Frame, inner: Frame, tolerance: int = 2) -> bool:
    """Whether ``inner`` lies within ``outer``, allowing a small frame-boundary slack."""
    ox, oy, ow, oh = outer
    ix, iy, iw, ih = inner
    return (
        ix >= ox - tolerance
        and iy >= oy - tolerance
        and ix + iw <= ox + ow + tolerance
        and iy + ih <= oy + oh + tolerance
    )


def _page_element(elements: Iterable[Mapping[str, Any]]) -> Mapping[str, Any] | None:
    """The WebView2 window, or the page document when the window is not in the tree."""
    document = None
    for element in elements:
        if element.get("label") != "Assistant":
            continue
        role = element.get("role")
        if role == "Window":
            return element
        if role == "Document" and document is None:
            document = element
    return document


def assistant_page(elements: Iterable[Mapping[str, Any]]) -> dict[str, Any] | None:
    """The Assistant page and its Close button, or ``None`` while the page is not ready.

    ``Close`` is scoped to the page: a dialog elsewhere in the same window may have
    its own Close button, and clicking that would say nothing about the page.
    """
    elements = list(elements)
    page_element = _page_element(elements)
    if page_element is None:
        return None
    page = _frame(page_element)
    document = next(
        (e for e in elements if e.get("role") == "Document" and e.get("label") == "Assistant"),
        None,
    )
    close = next(
        (
            e
            for e in elements
            if e.get("role") == "Button" and e.get("label") == "Close" and inside(page, _frame(e))
        ),
        None,
    )
    if close is None:
        return None
    return {
        "page": page,
        "document": _frame(document) if document is not None else None,
        "close": _frame(close),
        "close_token": close.get("element_token"),
    }
