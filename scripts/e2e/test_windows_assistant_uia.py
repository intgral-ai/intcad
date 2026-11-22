"""Tests for the Windows Assistant UIA helpers; run without a VM."""

from __future__ import annotations

import os
import shutil
import subprocess
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


@unittest.skipUnless(shutil.which("pwsh"), "PowerShell is required for the standalone smoke fixtures")
class StandaloneSmoke(unittest.TestCase):
    """Run the smoke's own functions, without launching a desktop or touching the network."""

    def fixture(self, script: str) -> None:
        smoke = Path(__file__).resolve().parents[2] / "tools/windows-assistant-smoke.ps1"
        load = r"""
            $ErrorActionPreference = 'Stop'
            $tokens = $null; $errors = $null
            $ast = [System.Management.Automation.Language.Parser]::ParseFile(
                $env:ASSISTANT_SMOKE_FIXTURE_SOURCE, [ref]$tokens, [ref]$errors)
            if ($errors) { throw ($errors -join "`n") }
            foreach ($statement in $ast.EndBlock.Statements) {
                if ($statement -is [System.Management.Automation.Language.FunctionDefinitionAst]) {
                    . ([scriptblock]::Create($statement.Extent.Text))
                }
            }
            function Reject([scriptblock]$action, [string]$message) {
                try { & $action } catch {
                    if ($_.Exception.Message -like "*$message*") { return }
                    throw
                }
                throw "expected refusal: $message"
            }
        """
        result = subprocess.run(
            ["pwsh", "-NoProfile", "-NonInteractive", "-Command", load + script],
            env={**os.environ, "ASSISTANT_SMOKE_FIXTURE_SOURCE": str(smoke)},
            text=True, capture_output=True, timeout=15, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_requires_the_current_view_marker_as_typed_json(self) -> None:
        self.fixture("""
            Reject { ReadAssistant @{} } 'invalid ui.inspect.assistant'
            Reject { ReadAssistant @{assistant=@{open=$true; host='available'}} } 'invalid ui.inspect.assistant'
            Reject { ReadAssistant @{assistant=@{open=$true; surface_ready='true'; host='available'}} } 'invalid ui.inspect.assistant'
            Reject { ReadAssistant @{assistant=@{open='false'; surface_ready=$true; host='available'}} } 'invalid ui.inspect.assistant'
            Reject { ReadAssistant @{assistant=@{open=$true; surface_ready=$true; host='unknown'}} } 'invalid ui.inspect.assistant'
            $state = ReadAssistant (@{assistant=@{open=$true; surface_ready=$false; host='available'}})
            if ($state.surface_ready -ne $false) { throw 'false must not become readiness' }
            $state = ReadAssistant (@{assistant=@{open=$true; surface_ready=$true; host='available'}})
            if ($state.surface_ready -ne $true) { throw 'lost readiness' }
        """)

    def test_requires_a_native_creation_diagnostic_not_unrelated_runtime_text(self) -> None:
        self.fixture("""
            Reject { RequireCreationDiagnostic "solidcraft: assistant web view: could not be zoomed`nmissing runtime" } 'native creation diagnostic'
            Reject { RequireCreationDiagnostic 'another runtime could not be created: missing' } 'native creation diagnostic'
            RequireCreationDiagnostic 'solidcraft: assistant web view: could not be created: system error'
        """)

    def test_page_close_never_clicks_an_outside_or_inactive_hit(self) -> None:
        self.fixture("""
            $page = @{ Left=528; Top=166; Right=908; Bottom=547 }
            $outside = @{ Left=20; Top=20; Right=55; Bottom=55; Enabled=$true; Offscreen=$false }
            $empty = @{ Left=857; Top=177; Right=857; Bottom=212; Enabled=$true; Offscreen=$false }
            $disabled = @{ Left=857; Top=177; Right=892; Bottom=212; Enabled=$false; Offscreen=$false }
            $hidden = @{ Left=857; Top=177; Right=892; Bottom=212; Enabled=$true; Offscreen=$true }
            $valid = @{ Left=857; Top=177; Right=892; Bottom=212; Enabled=$true; Offscreen=$false }
            foreach ($invalid in @($outside,$empty,$disabled,$hidden)) {
                Reject { PageClosePoint $page @($invalid) } 'no unique clickable page Close'
            }
            $point = PageClosePoint $page @($outside,$valid)
            if ($point.X -ne 874 -or $point.Y -ne 194) { throw "wrong close point: $point" }
        """)

    def test_page_close_requires_closed_panel_hidden_child_and_live_app(self) -> None:
        self.fixture("""
            $closed = @{ open=$false; surface_ready=$true; host='available' }
            if (PageClosed @{open=$true} $null $true) { throw 'accepted open panel' }
            if (PageClosed $closed @{Window=1} $true) { throw 'accepted visible page' }
            Reject { PageClosed $closed $null $false } 'app exited during page Close'
            if (-not (PageClosed $closed $null $true)) { throw 'rejected a retained hidden page' }
        """)

    def test_outer_close_rejects_timeout_nonzero_and_exit_before_input(self) -> None:
        self.fixture("""
            $process = [pscustomobject]@{ HasExited=$false; ExitCode=0; Exits=$false }
            $process | Add-Member ScriptMethod WaitForExit {
                param([int]$milliseconds)
                if ($milliseconds -ne 60000) { throw 'wrong close wait' }
                $this.Exits
            }
            Reject { WaitCleanExit $process { throw 'close was never delivered' } } 'close was never delivered'
            Reject { WaitCleanExit $process {} } 'did not exit within 60 s'
            $process.Exits = $true; $process.ExitCode = 42
            Reject { WaitCleanExit $process {} } 'exit code 0x2a'
            $process.ExitCode = 0
            WaitCleanExit $process {}
            $process.HasExited = $true
            Reject { WaitCleanExit $process {} } 'app exited before main window close'
        """)

    def test_native_target_uses_last_exact_match_and_client_pixel_scale(self) -> None:
        self.fixture("""
            $result = @{pixelsPerPoint=1.25; rects=@(
                @{ name='titlebar:assistant'; x=1; y=2; width=20; height=40 },
                @{ name='titlebar:assistant'; x=20; y=40; width=40; height=20 },
                @{ name='titlebar:assistant_badge'; x=90; y=10; width=5; height=5 }
            )}
            $rect = ScreenRect $result 'titlebar:assistant' @{X=44; Y=50}
            if ($rect.Left -ne 69 -or $rect.Top -ne 100 -or $rect.Right -ne 119 -or $rect.Bottom -ne 125) {
                throw 'wrong screen rectangle'
            }
        """)

    def test_visible_child_cannot_make_up_for_false_readiness(self) -> None:
        self.fixture("""
            $script:observations = 0
            function Call {
                if (++$script:observations -gt 1) { throw 'polled again for readiness' }
                @{assistant=@{open=$true; surface_ready=$false; host='available'}; ui=@{layout='floating'}}
            }
            function Views { throw 'must not accept visibility without current-view readiness' }
            function Start-Sleep {}
            Reject { WaitPage ([datetime]::UtcNow.AddSeconds(10)) 'floating' } 'polled again for readiness'
        """)


if __name__ == "__main__":
    unittest.main()
