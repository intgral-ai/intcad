"""Tests for the Windows Assistant UIA helpers; run without a VM."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from windows_assistant_uia import assistant_page, center, inside  # noqa: E402

WINDOW = {"role": "Window", "label": "Assistant", "frame": {"x": 528, "y": 166, "w": 380, "h": 381}}
DOCUMENT = {"role": "Document", "label": "Assistant", "frame": {"x": 537, "y": 167, "w": 370, "h": 379}}
CLOSE = {"role": "Button", "label": "Close", "frame": {"x": 857, "y": 177, "w": 35, "h": 35}, "element_token": "s0000000e:6"}
NEW_CHAT = {"role": "Button", "label": "New chat", "frame": {"x": 810, "y": 177, "w": 35, "h": 35}}
OUTSIDE_CLOSE = {"role": "Button", "label": "Close", "frame": {"x": 20, "y": 20, "w": 35, "h": 35}, "element_token": "s1:1"}


class AssistantPage(unittest.TestCase):
    def test_finds_the_real_page_and_its_close(self) -> None:
        found = assistant_page([WINDOW, DOCUMENT, NEW_CHAT, CLOSE])
        self.assertIsNotNone(found)
        self.assertEqual(found["page"], [528, 166, 380, 381])
        self.assertEqual(found["document"], [537, 167, 370, 379])
        self.assertEqual(found["close"], [857, 177, 35, 35])
        self.assertEqual(found["close_token"], "s0000000e:6")
        self.assertEqual(center(found["close"]), (874, 194))
        self.assertTrue(inside(found["page"], found["close"]))

    def test_page_is_not_the_window_edge_rectangle_the_old_acceptance_used(self) -> None:
        # The old Windows acceptance used window.x + window.width - chat.width = 624.
        # The page actually stands 96 px left of that, beside the right rail.
        window_x, window_width, chat_width = 44, 960, 380
        synthetic = [window_x + window_width - chat_width, 207, chat_width, 393]
        found = assistant_page([WINDOW, DOCUMENT, CLOSE])
        self.assertIsNotNone(found)
        self.assertNotEqual(found["page"], synthetic)
        self.assertGreaterEqual(synthetic[0] - found["page"][0], 80)
        self.assertNotEqual(center(found["close"]), (synthetic[0] + 80, synthetic[1] + 29))

    def test_a_close_outside_the_page_is_not_the_page_control(self) -> None:
        self.assertIsNone(assistant_page([WINDOW, OUTSIDE_CLOSE]))
        found = assistant_page([WINDOW, OUTSIDE_CLOSE, CLOSE])
        self.assertIsNotNone(found)
        self.assertEqual(found["close_token"], "s0000000e:6")

    def test_document_alone_still_gives_the_page(self) -> None:
        found = assistant_page([DOCUMENT, CLOSE])
        self.assertIsNotNone(found)
        self.assertEqual(found["page"], [537, 167, 370, 379])
        self.assertEqual(found["document"], [537, 167, 370, 379])

    def test_no_page_or_no_close_is_not_a_clickable_page(self) -> None:
        self.assertIsNone(assistant_page([]))
        self.assertIsNone(assistant_page([WINDOW, NEW_CHAT]))
        self.assertIsNone(assistant_page([OUTSIDE_CLOSE]))


class Geometry(unittest.TestCase):
    def test_center_is_the_middle_of_the_frame(self) -> None:
        self.assertEqual(center([0, 0, 35, 35]), (17, 17))
        self.assertEqual(center([877, 177, 35, 35]), (894, 194))

    def test_inside_allows_only_the_frame_boundary_slack(self) -> None:
        self.assertTrue(inside([0, 0, 100, 100], [10, 10, 20, 20]))
        self.assertTrue(inside([0, 0, 100, 100], [2, 2, 100, 100]))
        self.assertFalse(inside([0, 0, 100, 100], [-3, 0, 20, 20]))
        self.assertFalse(inside([0, 0, 100, 100], [90, 90, 20, 20]))


if __name__ == "__main__":
    unittest.main()
