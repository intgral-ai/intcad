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
            if (PageClosed @{open=$true} $null $true $true) { throw 'accepted open panel' }
            if (PageClosed $closed @{Window=1} $true $true) { throw 'accepted visible page' }
            Reject { PageClosed $closed $null $false $true } 'app exited during page Close'
            if (PageClosed $closed $null $true $false) { throw 'accepted destroyed or detached original child' }
            if (PageClosed @{open=$false; surface_ready=$false; host='available'} $null $true $true) {
                throw 'accepted lost retained readiness'
            }
            if (-not (PageClosed $closed $null $true $true)) { throw 'rejected a retained hidden page' }
        """)

    def test_close_reports_only_bounded_counts_and_does_not_click_on_refusal(self) -> None:
        self.fixture("""
            $view = @{Window=[IntPtr]42; Rect=@{Left=10; Top=20; Right=110; Bottom=120}}
            $valid = @{Left=20; Top=30; Right=40; Bottom=50; Enabled=$true; Offscreen=$false; Name='private draft'}
            $disabled = $valid.Clone(); $disabled.Enabled = $false
            $offscreen = $valid.Clone(); $offscreen.Offscreen = $true
            $zero = $valid.Clone(); $zero.Right = $zero.Left
            $outside = $valid.Clone(); $outside.Right = 111
            function FindPageClose { $query }
            function Click { throw 'unexpected input on refusal' }
            function Write-Host($message) { $script:diagnostic = $message }
            foreach ($case in @(
                @{Query=@{matched=0; empty=0; candidates=@()}; Counts='matched=0 empty=0 disabled=0 offscreen=0 nonpositive=0 out_of_child=0 valid=0'},
                @{Query=@{matched=5; empty=1; candidates=@($disabled,$offscreen,$zero,$outside)}; Counts='matched=5 empty=1 disabled=1 offscreen=1 nonpositive=1 out_of_child=1 valid=0'},
                @{Query=@{matched=2; empty=0; candidates=@($valid,$valid)}; Counts='matched=2 empty=0 disabled=0 offscreen=0 nonpositive=0 out_of_child=0 valid=2'},
                @{Query=@{matched=65; empty=0; candidates=(@($valid) * 65)}; Counts='matched=>64 empty=0 disabled=0 offscreen=0 nonpositive=0 out_of_child=0 valid=>64'}
            )) {
                $query = $case.Query
                $script:diagnostic = ''
                Reject { ClosePage $view } 'no unique clickable page Close inside the live WebView'
                if ($script:diagnostic -cne "page Close UIA: $($case.Counts); webview=10,20-110,120") {
                    throw "unexpected diagnostic: $script:diagnostic"
                }
            }
            # The count bound is for logging, not a limit on the candidates searched.
            $query = @{matched=65; empty=1; candidates=((@($disabled) * 64) + @($valid))}
            $script:diagnostic = ''
            function Click($x, $y, $window) {
                if ($x -eq 30 -and $y -eq 40 -and $window -eq $view.Window) { throw 'fixture reached unchanged click' }
                throw 'unexpected click point'
            }
            Reject { ClosePage $view } 'fixture reached unchanged click'
            if ($script:diagnostic) { throw 'unique Close should not emit a refusal diagnostic' }
        """)

    def test_only_exact_cold_zero_compares_roots_and_preserves_the_exact_refusal(self) -> None:
        self.fixture("""
            $view = @{Window=[IntPtr]42; Rect=@{Left=10; Top=20; Right=110; Bottom=120}}
            $refusal = 'no unique clickable page Close inside the live WebView'
            $valid = @{Left=20; Top=30; Right=40; Bottom=50; Enabled=$true; Offscreen=$false}
            $disabled = $valid.Clone(); $disabled.Enabled = $false
            $counts = 'ok wrapper.close_name_any_role=0 wrapper.close_name_button=0 wrapper.documents=65 widget.close_name_any_role=1 widget.close_name_button=1 widget.documents=1'
            function FindPageClose { $query }
            function Click { throw 'diagnostics must not authorize input' }
            function Write-Host($message) {
                if ($case.LogFails -and $message -clike 'page Close UIA roots:*') { throw 'comparison logging failed' }
                $script:messages.Add($message)
            }
            function ReadPageCloseComparison($target) {
                if ($target.Window -ne $view.Window) { throw 'wrong WRY wrapper' }
                $script:comparisons++
                if ($readerThrows) { throw 'private provider text' }
                $receipt
            }
            foreach ($case in @(
                @{Query=('{"matched":0,"empty":0,"candidates":[]}' | ConvertFrom-Json -AsHashtable); Reader=$counts; Status=$counts},
                @{Query=@{matched=0; empty=0; candidates=@()}; Throws=$true; Status='error'},
                @{Query=@{matched=0; empty=0; candidates=@()}; Reader='unsupported'; Status='unsupported'},
                @{Query=@{matched=0; empty=0; candidates=@()}; Reader='stale'; Status='stale'},
                @{Query=@{matched=0; empty=0; candidates=@()}; Reader="$counts private title"; Status='error'},
                @{Query=@{matched=0; empty=0; candidates=@()}; Reader=$counts.Replace('documents=65', 'documents=66'); Status='error'},
                @{Query=@{matched=0; empty=0; candidates=@()}; Reader=$counts.Replace('widget.close_name_any_role=1', 'widget.close_name_any_role=0'); Status='error'},
                @{Query=@{matched=0; empty=0; candidates=@()}; Reader=$counts; LogFails=$true},
                @{Query=@{matched=0; empty=0; candidates=@(); unrelated='private'}},
                @{Query=@{matched='0'; empty=0; candidates=@()}},
                @{Query=@{matched=$false; empty=0; candidates=@()}},
                @{Query=@{matched=0; empty=0; candidates=$null}},
                @{Query=@{matched=0; empty=1; candidates=@()}},
                @{Query=@{matched=0; empty=0; candidates=@($disabled)}},
                @{Query=@{matched=1; empty=1; candidates=@()}},
                @{Query=@{matched=1; empty=0; candidates=@($disabled)}},
                @{Query=@{matched=2; empty=0; candidates=@($valid,$valid)}}
            )) {
                $query = $case.Query; $receipt = $case.Reader; $readerThrows = $case.Throws
                $script:comparisons = 0
                $script:messages = [Collections.Generic.List[string]]::new()
                $errorText = try { ClosePage $view } catch { $_.Exception.Message }
                if ($errorText -cne $refusal) { throw "changed refusal: $errorText" }
                if ($case.LogFails) {
                    if ($script:comparisons -ne 1 -or $script:messages.Count -ne 1) {
                        throw 'a failed comparison log must leave only the original diagnostic'
                    }
                } elseif ($case.Status) {
                    if ($script:comparisons -ne 1) { throw 'exact cold zero must run one comparison' }
                    if ($script:messages.Count -ne 2 -or $script:messages[0] -cne "page Close UIA roots: $($case.Status)") {
                        throw "unexpected or unsafe comparison output: $($script:messages -join '; ')"
                    }
                } elseif ($script:comparisons -ne 0 -or $script:messages.Count -ne 1) {
                    throw 'a nonzero or inexact result invoked diagnostics'
                }
                if ($script:messages[-1] -cnotlike 'page Close UIA: matched=*; webview=10,20-110,120') {
                    throw 'lost the original refusal diagnostic'
                }
            }
        """)

    def test_root_comparison_samples_only_a_unique_live_widget_and_rejects_stale_counts(self) -> None:
        self.fixture("""
            class DiagnosticFocus {
                static [IntPtr] $Foreground
                static [IntPtr] $Focus
                static [bool] $QueueOk
                static [bool] $ChangeDuringRead
                static [IntPtr] GetForegroundWindow() { return [DiagnosticFocus]::Foreground }
                static [hashtable] InputQueue() {
                    if ([DiagnosticFocus]::ChangeDuringRead) { [DiagnosticFocus]::Foreground = [IntPtr]20 }
                    return @{Succeeded=[DiagnosticFocus]::QueueOk; hwndFocus=[DiagnosticFocus]::Focus}
                }
            }
            $frame = [IntPtr]10; $wrapper = [IntPtr]20; $widget = [IntPtr]30
            $start = [datetime]::UtcNow
            $owner = @{Id=100; HasExited=$false; StartTime=$start}
            function ReadPageCloseWindow($native, $window) { $windows[$window.ToInt64()] }
            function FindVisiblePageCloseWidgets($native, $scope) {
                if ($scope -ne $wrapper) { throw 'native enumeration escaped the wrapper' }
                $widgets
            }
            function ReadPageCloseCounts($root) {
                $reads.Add($root.ToInt64())
                if ($change -eq 'reparent' -and $root -eq $wrapper) { $windows[25L].Parent = [IntPtr]10 }
                if ($change -eq 'change-owner' -and $root -eq $widget) { $windows[30L].Owner = 201 }
                if ($change -eq 'hide' -and $root -eq $widget) { $windows[30L].Visible = $false }
                if ($change -eq 'exit' -and $root -eq $wrapper) { $owner.HasExited = $true }
                if ($change -eq 'foreground-lost-between' -and $root -eq $wrapper) { [DiagnosticFocus]::Foreground = $wrapper }
                if ($change -eq 'focus-lost-between' -and $root -eq $wrapper) { [DiagnosticFocus]::Focus = $wrapper }
                if ($change -eq 'foreground-lost-after' -and $root -eq $widget) { [DiagnosticFocus]::Foreground = [IntPtr]::Zero }
                if ($change -eq 'focus-lost-after' -and $root -eq $widget) { [DiagnosticFocus]::Focus = [IntPtr]::Zero }
                if ($change -eq 'focus-unverified-after' -and $root -eq $widget) { [DiagnosticFocus]::QueueOk = $false }
                if ($change -eq 'throw' -and $root -eq $widget) { throw 'private UIA provider exception' }
                if ($root -eq $wrapper) { NewPageCloseCounts 0 0 64 $true }
                elseif ($root -eq $widget) { NewPageCloseCounts 80 66 0 $true }
                else { throw 'UIA sample escaped the approved roots' }
            }
            if (-not (Get-Command ComparePageCloseRoots -ErrorAction SilentlyContinue)) {
                throw 'missing bounded root comparison'
            }
            foreach ($case in @(
                @{Mode='live'; Widgets=@($widget); Reads='20,30'; Report='ok wrapper.close_name_any_role=0 wrapper.close_name_button=0 wrapper.documents=65 widget.close_name_any_role=65 widget.close_name_button=65 widget.documents=1'},
                @{Mode='wrong-foreground-before'; Widgets=@($widget); Reads=''; Report='stale'},
                @{Mode='wrong-focus-before'; Widgets=@($widget); Reads=''; Report='stale'},
                @{Mode='focus-unverified-before'; Widgets=@($widget); Reads=''; Report='stale'},
                @{Mode='unstable-foreground-before'; Widgets=@($widget); Reads=''; Report='stale'},
                @{Mode='foreground-lost-between'; Widgets=@($widget); Reads='20'; Report='stale'},
                @{Mode='focus-lost-between'; Widgets=@($widget); Reads='20'; Report='stale'},
                @{Mode='foreground-lost-after'; Widgets=@($widget); Reads='20,30'; Report='stale'},
                @{Mode='focus-lost-after'; Widgets=@($widget); Reads='20,30'; Report='stale'},
                @{Mode='focus-unverified-after'; Widgets=@($widget); Reads='20,30'; Report='stale'},
                @{Mode='missing'; Widgets=@(); Reads=''; Report='unsupported'},
                @{Mode='multiple'; Widgets=@($widget,[IntPtr]31); Reads=''; Report='unsupported'},
                @{Mode='unrelated'; Widgets=@([IntPtr]31); Reads=''; Report='stale'},
                @{Mode='hidden'; Widgets=@($widget); Reads=''; Report='stale'},
                @{Mode='stale-wrapper'; Widgets=@($widget); Reads=''; Report='stale'},
                @{Mode='wrong-frame-owner'; Widgets=@($widget); Reads=''; Report='stale'},
                @{Mode='reparent'; Widgets=@($widget); Reads='20'; Report='stale'},
                @{Mode='change-owner'; Widgets=@($widget); Reads='20,30'; Report='stale'},
                @{Mode='hide'; Widgets=@($widget); Reads='20,30'; Report='stale'},
                @{Mode='exit'; Widgets=@($widget); Reads='20'; Report='stale'},
                @{Mode='throw'; Widgets=@($widget); Reads='20,30'; Report='error'}
            )) {
                $change = $case.Mode; $widgets = $case.Widgets
                $owner.HasExited = $false
                [DiagnosticFocus]::Foreground = if ($change -eq 'wrong-foreground-before') { $wrapper } else { $frame }
                [DiagnosticFocus]::Focus = if ($change -eq 'wrong-focus-before') { $wrapper } else { $frame }
                [DiagnosticFocus]::QueueOk = $change -ne 'focus-unverified-before'
                [DiagnosticFocus]::ChangeDuringRead = $change -eq 'unstable-foreground-before'
                $windows = @{
                    10L=@{Owner=100; Thread=1; Parent=[IntPtr]::Zero; Class='fixture frame'; Visible=$true}
                    20L=@{Owner=100; Thread=1; Parent=$frame; Class='WRY_WEBVIEW'; Visible=$true}
                    25L=@{Owner=200; Thread=2; Parent=$wrapper; Class='fixture intermediate'; Visible=$true}
                    30L=@{Owner=200; Thread=2; Parent=[IntPtr]25; Class='Chrome_WidgetWin_1'; Visible=$true}
                    31L=@{Owner=200; Thread=2; Parent=$frame; Class='Chrome_WidgetWin_1'; Visible=$true}
                }
                if ($change -eq 'hidden') { $windows[30L].Visible = $false }
                if ($change -eq 'stale-wrapper') { $windows[20L].Class = 'unrelated' }
                if ($change -eq 'wrong-frame-owner') { $windows[10L].Owner = 200 }
                $reads = [Collections.Generic.List[long]]::new()
                $report = ComparePageCloseRoots ([DiagnosticFocus]) $frame $wrapper $owner $start.ToUniversalTime().Ticks
                if ($report -cne $case.Report -or ($reads -join ',') -cne $case.Reads) {
                    throw "$change compared $($reads -join ',') and returned $report"
                }
            }
        """)

    def test_shared_reader_contains_compiler_and_checks_the_whole_job_before_accepting(self) -> None:
        self.fixture("""
            $env:SystemRoot = [IO.Path]::GetTempPath()
            $text = '# fixture only; process start is mocked'
            $clock = @{ElapsedMilliseconds=0L}
            $script:job = 'app job must not be touched'
            function NewPageCloseReaderJob { $events.Add('create'); $jobHandle }
            function AssignOwnedLauncher($job, $launcher) {
                if ($job -cne $jobHandle -or $launcher -ne $process -or $stdin.Text) { throw 'unsafe assignment' }
                $events.Add('assign')
                if ($mode -eq 'denied') { throw 'job assignment denied' }
                $process.InJob = $true
            }
            function TestPageCloseReaderJobMember($job, $reader) {
                if ($job -cne $jobHandle -or $reader -ne $process -or $stdin.Text) { throw 'unsafe membership check' }
                $events.Add('verify')
                if ($mode -eq 'late-admission') { $clock.ElapsedMilliseconds = 13000L }
                $mode -ne 'unverified'
            }
            function ReadPageCloseReaderJobActive($job) {
                if ($job -cne $jobHandle) { throw 'queried another job' }
                $events.Add("active=$($script:active)")
                $script:active
            }
            function StopOwnedJob($job) {
                if ($job -cne $jobHandle) { throw 'terminated another job' }
                $events.Add('stop')
                if ($mode -cne 'survivor') { $script:active = 0 }
            }
            function CloseOwnedJob($job) {
                if ($job -cne $jobHandle) { throw 'disposed another job' }
                $events.Add('close')
            }
            function Start-Sleep([int]$Milliseconds) { $clock.ElapsedMilliseconds += $Milliseconds }
            function StartPageCloseUiaReader($info) {
                if (-not $info.RedirectStandardOutput -or -not $info.RedirectStandardError -or $info.UseShellExecute) {
                    throw 'reader pipes are not private'
                }
                if ($mode -ne 'original' -and ($events -join ',') -cne 'create') {
                    throw 'diagnostic job must exist before reader start'
                }
                if ($info.RedirectStandardInput -ne ($mode -ne 'original')) { throw 'wrong admission pipe mode' }
                $events.Add('start')
                if ($mode -eq 'start-error') { throw 'reader failed to start' }
                $process
            }
            $jobHandle = 'independent diagnostic job'
            $stream = [pscustomobject]@{Text=''}
            $stream | Add-Member ScriptMethod ReadToEndAsync { [Threading.Tasks.Task]::FromResult($this.Text) }
            $output = $stream.PSObject.Copy(); $output.Text = '{"matched":0,"empty":0,"candidates":[]}'
            $stdin = [pscustomobject]@{Text=''}
            $stdin | Add-Member ScriptMethod WriteLine {
                param($line)
                if (-not $process.InJob -or $events[-1] -cne 'verify' -or $mode -eq 'unverified') {
                    throw 'authorized before verified membership'
                }
                $events.Add('authorize')
                $this.Text = $line
                $script:active = 1
            }
            $stdin | Add-Member ScriptMethod Dispose { $events.Add('eof') }
            $process = [pscustomobject]@{HasExited=$false; InJob=$false; ExitCode=0;
                StandardError=$stream; StandardOutput=$output; StandardInput=$stdin}
            $process | Add-Member ScriptMethod WaitForExit {
                param($milliseconds)
                $events.Add("wait=$milliseconds")
                if ($this.HasExited) { return $true }
                if ($mode -eq 'late') { $clock.ElapsedMilliseconds = 15000L }
                if ($mode -eq 'timeout') { $clock.ElapsedMilliseconds = 13000L; return $false }
                $this.HasExited = $true
                # The parent can exit with a compiler still active, even with both pipes closed.
                $script:active = if ($mode -cin @('leftover', 'survivor')) { 1 } else { 0 }
                $true
            }
            $process | Add-Member ScriptMethod Kill { $events.Add('kill'); $this.HasExited = $true }
            $process | Add-Member ScriptMethod Dispose { $events.Add('dispose') }
            foreach ($case in @(
                @{Mode='original'; Events='start,wait=15000,dispose'},
                @{Mode='clean'; Events='create,start,assign,verify,authorize,eof,wait=13000,active=0,active=0,dispose,close'},
                @{Mode='late'; Error='deadline'; Events='create,start,assign,verify,authorize,eof,wait=13000,active=0,dispose,close'},
                @{Mode='timeout'; Error='deadline'; Events='create,start,assign,verify,authorize,eof,wait=13000,active=1,stop,active=0,dispose,close'},
                @{Mode='leftover'; Error='did not empty'; Events='create,start,assign,verify,authorize,eof,wait=13000,active=1,active=1,stop,active=0,dispose,close'},
                @{Mode='survivor'; Error='cleanup'; Events='create,start,assign,verify,authorize,eof,wait=13000,active=1,active=1,stop,active=1*,dispose,close'},
                @{Mode='denied'; Error='denied'; Events='create,start,assign,eof,kill,wait=15000,active=0,dispose,close'},
                @{Mode='unverified'; Error='membership'; Events='create,start,assign,verify,eof,kill,wait=15000,active=0,dispose,close'},
                @{Mode='late-admission'; Error='deadline'; Events='create,start,assign,verify,eof,active=0,dispose,close'},
                @{Mode='expired'; Error='deadline'; Events=''},
                @{Mode='start-error'; Error='cannot start'; Events='create,start,active=0,close'}
            )) {
                $mode = $case.Mode
                $clock.ElapsedMilliseconds = if ($mode -eq 'expired') { 13000L } else { 0L }
                $process.HasExited = $process.InJob = $false
                $stdin.Text = ''
                $script:active = 0
                $events = [Collections.Generic.List[string]]::new()
                if ($mode -eq 'original') {
                    $query = InvokePageCloseUiaReader $text
                    if ($query.matched -ne 0) { throw 'original result changed' }
                } elseif ($case.Error) {
                    Reject { InvokePageCloseUiaReader $text $clock } $case.Error
                } else {
                    if ((InvokePageCloseUiaReader $text $clock) -cne $output.Text) { throw 'clean reader lost its output' }
                }
                if (($events -join ',') -cnotlike $case.Events) { throw "$mode had wrong lifecycle: $($events -join ',')" }
                if ($clock.ElapsedMilliseconds -gt 15000 -or ($mode -eq 'survivor' -and $clock.ElapsedMilliseconds -ne 15000)) {
                    throw 'cleanup failed to use the same bounded deadline'
                }
            }
        """)

    def test_both_queries_use_the_same_watchdog_without_launching_a_reader(self) -> None:
        self.fixture(r"""
            class AssistantWin {
                static [IntPtr] GetForegroundWindow() { return [IntPtr]10 }
                static [hashtable] InputQueue() { return @{Succeeded=$true; hwndFocus=[IntPtr]10} }
            }
            $h = [IntPtr]10
            $script:p = @{Id=100; StartTime=[datetime]::UtcNow}
            $view = @{Window=[IntPtr]20}
            $env:SystemRoot = [IO.Path]::GetTempPath()
            function IsOwnedFrame { $true }
            function NewPageCloseReaderJob { 'comparison job' }
            function AssignOwnedLauncher {}
            function TestPageCloseReaderJobMember { $true }
            function ReadPageCloseReaderJobActive { 0 }
            function CloseOwnedJob {}
            $script:launches = [Collections.Generic.List[string]]::new()
            function StartPageCloseUiaReader($info) {
                $encoded = $info.ArgumentList[-1]
                if ($encoded.Length -gt 32000) { throw 'reader command line is too long for Windows' }
                $text = [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($encoded))
                $payload = [scriptblock]::Create($text)
                if ($script:launches.Count -eq 1) {
                    $first = $payload.Ast.EndBlock.Statements[0]
                    if ($first -isnot [System.Management.Automation.Language.IfStatementAst]) {
                        throw 'secondary payload must gate compilation before any diagnostic work'
                    }
                    # Run the real gate with simulated stdin, but replace the Windows-only body.
                    $gate = [scriptblock]::Create($first.Extent.Text + "`n'fixture reached diagnostic body'")
                    foreach ($inputText in @('', "wrong`n", "compare`n")) {
                        $savedIn = [Console]::In
                        $inputReader = [IO.StringReader]::new($inputText)
                        try { [Console]::SetIn($inputReader); $admitted = & $gate }
                        finally { [Console]::SetIn($savedIn); $inputReader.Dispose() }
                        if (($admitted -ceq 'fixture reached diagnostic body') -ne ($inputText -ceq "compare`n")) {
                            throw 'incorrect diagnostic admission'
                        }
                    }
                }
                $script:launches.Add($text)
                $stream = [pscustomobject]@{Text=''}
                $stream | Add-Member ScriptMethod ReadToEndAsync { [Threading.Tasks.Task]::FromResult($this.Text) }
                $output = $stream.PSObject.Copy()
                $output.Text = if ($script:launches.Count -eq 1) {
                    '{"matched":0,"empty":0,"candidates":[]}'
                } else { "unsupported`r`n" }
                $reader = [pscustomobject]@{HasExited=$true; ExitCode=0; StandardOutput=$output; StandardError=$stream;
                    StandardInput=[IO.StringWriter]::new()}
                $reader | Add-Member ScriptMethod WaitForExit { $true }
                $reader | Add-Member ScriptMethod Dispose {}
                $reader
            }
            if (-not (Get-Command ReadPageCloseComparison -ErrorAction SilentlyContinue)) {
                throw 'missing secondary reader integration'
            }
            $query = FindPageClose $view
            if ($query.matched -ne 0 -or $query.candidates -isnot [array]) { throw 'lost cold JSON reply' }
            if ((ReadPageCloseComparison $view) -cne 'unsupported' -or $script:launches.Count -ne 2) {
                throw 'expected one cold reader and one comparison reader'
            }
        """)

    def test_parent_discards_the_receipt_if_cad_focus_cannot_be_verified(self) -> None:
        self.fixture("""
            class AssistantWin {
                static [IntPtr] $Foreground = [IntPtr]10
                static [IntPtr] $Focus = [IntPtr]10
                static [bool] $QueueOk = $true
                static [IntPtr] GetForegroundWindow() { return [AssistantWin]::Foreground }
                static [hashtable] InputQueue() {
                    return @{Succeeded=[AssistantWin]::QueueOk; hwndFocus=[AssistantWin]::Focus}
                }
            }
            $script:h = [IntPtr]10
            $script:p = @{Id=100; StartTime=[datetime]::UtcNow}
            $view = @{Window=[IntPtr]20}
            $counts = 'ok wrapper.close_name_any_role=0 wrapper.close_name_button=0 wrapper.documents=0 widget.close_name_any_role=1 widget.close_name_button=1 widget.documents=1'
            function IsOwnedFrame { $true }
            function InvokePageCloseUiaReader {
                $script:reads++
                if ($mode -eq 'foreground-lost') { [AssistantWin]::Foreground = [IntPtr]20 }
                if ($mode -eq 'focus-lost') { [AssistantWin]::Focus = [IntPtr]20 }
                if ($mode -eq 'focus-unknown') { [AssistantWin]::QueueOk = $false }
                $counts
            }
            foreach ($mode in @('before', 'foreground-lost', 'focus-lost', 'focus-unknown', 'live')) {
                $script:reads = 0
                [AssistantWin]::Foreground = [IntPtr]10
                [AssistantWin]::Focus = if ($mode -eq 'before') { [IntPtr]::Zero } else { [IntPtr]10 }
                [AssistantWin]::QueueOk = $true
                $report = ReadPageCloseComparison $view
                $wanted = if ($mode -eq 'live') { $counts } else { 'stale' }
                if ($report -cne $wanted -or $script:reads -ne [int]($mode -ne 'before')) {
                    throw "parent accepted or started a comparison with unverified CAD focus: $mode"
                }
            }
        """)

    def test_focus_api_failure_is_blocked_but_null_and_wrong_focus_are_failures(self) -> None:
        self.fixture("""
            function Start-Sleep {}
            function ReadInputQueue { @{ Succeeded=$false; hwndFocus=[IntPtr]::Zero } }
            Reject { WaitFocus { $true } 0 } 'BLOCKED: GetGUIThreadInfo'
            function ReadInputQueue { @{ Succeeded=$true; hwndFocus=[IntPtr]::Zero } }
            Reject { WaitFocus { $false } 0 } 'unexpected keyboard focus'
            function ReadInputQueue { @{ Succeeded=$true; hwndFocus=[IntPtr]44 } }
            Reject { WaitFocus { $false } 0 } 'unexpected keyboard focus'
            if ((WaitFocus { param($f) $f -eq [IntPtr]44 } 0) -ne [IntPtr]44) { throw 'lost valid focus' }
        """)

    def test_mouse_disallows_stale_frame_target_or_capture_and_releases_partial_press(self) -> None:
        self.fixture("""
            function Start-Sleep {}
            $script:sent = 0; $script:released = 0
            function SendMouseClick { $script:sent++; $script:count }
            function ReleaseMouseButton { $script:released++; $true }
            $script:sample = @{
                FrameOwned=$true; Foreground=$true; CursorAtPoint=$true; LeftDown=$false;
                TargetInFrame=$true; HitInTarget=$true;
                Queue=@{Succeeded=$true; hwndCapture=[IntPtr]::Zero}
            }
            function ReadClickRoute { $script:sample }
            function MovePointer {}
            $script:sample.FrameOwned = $false
            Reject { Click 10 20 ([IntPtr]1) } 'live owned frame'
            $script:sample.FrameOwned = $true; $script:sample.TargetInFrame = $false
            Reject { Click 10 20 ([IntPtr]1) } 'target is no longer in the owned frame'
            $script:sample.TargetInFrame = $true; $script:sample.Queue.hwndCapture = [IntPtr]7
            Reject { Click 10 20 ([IntPtr]1) } 'mouse capture'
            $script:sample.Queue.hwndCapture = [IntPtr]::Zero; $script:sample.LeftDown = $true
            Reject { Click 10 20 ([IntPtr]1) } 'left mouse button is already down'
            if ($script:sent -ne 0 -or $script:released -ne 0) { throw 'sent input to an unsafe route' }
            $script:sample.LeftDown = $false; $script:count = 1
            Reject { Click 10 20 ([IntPtr]1) } 'partial mouse click'
            if ($script:sent -ne 1 -or $script:released -ne 1) { throw 'partial press was not released' }
            $script:count = 2
            Click 10 20 ([IntPtr]1)
            if ($script:sent -ne 2 -or $script:released -ne 1) { throw 'complete click was not sent exactly once' }
        """)

    def test_cleanup_records_surviving_job_and_pipe_failures_without_losing_primary_failure(self) -> None:
        self.fixture("""
            $script:failures = [Collections.Generic.List[string]]::new()
            $script:failures.Add('original native failure')
            $Evidence = 'fixture'
            $script:launcher = [pscustomobject]@{ HasExited=$true }
            $script:launcher | Add-Member ScriptMethod Dispose {}
            $script:job = [IntPtr]7
            # Stub Wait to reproduce an open pipe without sleeping for the production timeout.
            $script:stderr = [pscustomobject]@{}
            $script:stderr | Add-Member ScriptMethod Wait { $false }
            $script:stdout = $script:stderr
            $script:terminated = 0; $script:disposed = 0
            function WaitJobEmpty { $false }
            function StopOwnedJob { $script:terminated++ }
            function CloseOwnedJob { $script:disposed++ }
            function Set-Content {}
            [void](Finish 'fixture')
            if ($script:failures -join ';' -notlike 'original native failure;*run-owned job did not empty after termination*stderr pipe did not close*stdout pipe did not close*') {
                throw "lost a failure: $($script:failures -join ';')"
            }
            if ($script:terminated -ne 1 -or $script:disposed -ne 1) { throw 'lost job cleanup with the launcher already exited' }
        """)

    def test_successful_forced_cleanup_only_fails_an_otherwise_clean_run(self) -> None:
        self.fixture("""
            $script:failures = [Collections.Generic.List[string]]::new()
            $Evidence = 'fixture'
            function WaitJobEmpty { $script:terminated }
            function StopOwnedJob { $script:terminated = $true }
            function CloseOwnedJob {}
            function Set-Content {}
            foreach ($primary in @('BLOCKED: desktop input unavailable', 'FAIL: wrong page', '')) {
                $script:failures.Clear()
                if ($primary) { $script:failures.Add($primary) }
                $script:job = [IntPtr]7; $script:terminated = $false
                [void](Finish 'fixture')
                if (-not $script:terminated) { throw 'job was not terminated' }
                if ($script:failures.Count -ne 1) { throw "unexpected failures: $($script:failures -join ';')" }
                if ($primary -and $script:failures[0] -cne $primary) { throw 'primary diagnosis changed' }
                if (-not $primary -and $script:failures[0] -notlike '*forcing cleanup*') {
                    throw 'a clean run requiring termination was not failed'
                }
            }
        """)

    def test_relay_nonzero_exit_fails_even_after_both_pipes_reach_eof(self) -> None:
        self.fixture("""
            $script:failures = [Collections.Generic.List[string]]::new()
            $Evidence = 'fixture'
            $script:launcher = [pscustomobject]@{ HasExited=$true; ExitCode=19 }
            $script:launcher | Add-Member ScriptMethod Dispose {}
            $script:stderr = [Threading.Tasks.Task]::FromResult('')
            $script:stdout = [Threading.Tasks.Task]::FromResult('')
            function Set-Content {}
            [void](Finish 'fixture')
            if ($script:failures.Count -ne 1 -or $script:failures[0] -notlike '*relay exited with code 19*') {
                throw 'ignored nonzero relay exit'
            }
        """)

    def test_hidden_runtime_blocked_preserves_label_and_prevents_pass(self) -> None:
        self.fixture("""
            $script:failures = [Collections.Generic.List[string]]::new()
            $made = $true; $root = 'fixture'
            function Launch { throw 'BLOCKED: desktop input unavailable' }
            function New-Item {}
            function Get-ChildItem {}
            function Finish {}
            $statements = @($ast.EndBlock.Statements)
            $hidden = $statements | Where-Object {
                $_ -is [System.Management.Automation.Language.IfStatementAst] -and $_.Extent.Text -like '*Launch 47952*'
            }
            . ([scriptblock]::Create($hidden.Extent.Text))
            if ($script:failures.Count -ne 1 -or $script:failures[0] -cnotlike 'BLOCKED:*desktop input unavailable') {
                throw "hidden run lost BLOCKED: $($script:failures -join ';')"
            }
            Reject { . ([scriptblock]::Create(($statements[-2..-1].Extent.Text -join "`n"))) } 'BLOCKED:'
        """)

    def test_launcher_gets_eof_without_permission_to_launch_when_job_assignment_fails(self) -> None:
        self.fixture("""
            $pipe = [IO.StringWriter]::new()
            $launcher = @{ Handle=[IntPtr]4; StandardInput=$pipe }
            function AssignOwnedLauncher { throw 'fixture nested job denial' }
            Reject { AuthorizeOwnedLaunch ([IntPtr]7) $launcher } 'BLOCKED: cannot nest a run-owned job'
            if ($pipe.ToString()) { throw 'launcher was authorized without job membership' }
            Reject { $pipe.WriteLine('unexpected write') } 'closed'
            function AssignOwnedLauncher {}
            $pipe = [IO.StringWriter]::new()
            $launcher.StandardInput = $pipe
            AuthorizeOwnedLaunch ([IntPtr]7) $launcher
            if ($pipe.ToString().Trim() -cne 'launch') { throw 'assigned launcher not released' }
            Reject { $pipe.WriteLine('unexpected write') } 'closed'
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
            $rect = ScreenRect $result 'titlebar:assistant' @{Left=44; Top=50; Right=444; Bottom=450}
            if ($rect.Left -ne 69 -or $rect.Top -ne 100 -or $rect.Right -ne 119 -or $rect.Bottom -ne 125) {
                throw 'wrong screen rectangle'
            }
        """)

    def test_rect_geometry_rejects_missing_or_non_numeric_values(self) -> None:
        self.fixture("""
            $client = @{Left=-40; Top=50; Right=360; Bottom=450}
            $default = @{name='agent_chat:web'; x=20; y=40; width=40; height=20}
            foreach ($field in @('x','y','width','height','pixelsPerPoint')) {
                foreach ($invalid in @($null, $true, '1', [double]::NaN, [double]::PositiveInfinity)) {
                    $entry = $default.Clone(); $reply = @{pixelsPerPoint=1.25; rects=@($entry)}
                    if ($field -ne 'pixelsPerPoint') { $entry[$field] = $invalid } else { $reply[$field] = $invalid }
                    Reject { ScreenRect $reply 'agent_chat:web' $client } 'invalid geometry for agent_chat:web'
                }
                $entry = $default.Clone(); $reply = @{pixelsPerPoint=1.25; rects=@($entry)}
                if ($field -ne 'pixelsPerPoint') { $entry.Remove($field) } else { $reply.Remove($field) }
                Reject { ScreenRect $reply 'agent_chat:web' $client } 'invalid geometry for agent_chat:web'
            }
        """)

    def test_rect_geometry_rejects_overflow_collapse_and_outside_client_edges(self) -> None:
        self.fixture("""
            $client = @{Left=-40; Top=50; Right=60; Bottom=150}
            foreach ($rect in @(
                @{x=20; y=20; width=0; height=20},
                @{x=20; y=20; width=20; height=-1},
                @{x=20; y=20; width=0.1; height=20},
                @{x=9999999999; y=20; width=20; height=20},
                @{x=20; y=20; width=[double]::MaxValue; height=20},
                @{x=-1; y=20; width=20; height=20},
                @{x=20; y=-1; width=20; height=20},
                @{x=70; y=20; width=20; height=20},
                @{x=20; y=70; width=20; height=20}
            )) {
                $rect.name = 'agent_chat:web'
                $reply = @{pixelsPerPoint=1.25; rects=@($rect)}
                Reject { ScreenRect $reply 'agent_chat:web' $client } 'invalid geometry for agent_chat:web'
            }
            $reply = @{pixelsPerPoint=1.25; rects=@(@{name='agent_chat:web';x=0;y=0;width=80;height=80})}
            $screen = ScreenRect $reply 'agent_chat:web' $client
            if ($screen.Left -ne -40 -or $screen.Top -ne 50 -or $screen.Right -ne 60 -or $screen.Bottom -ne 150) {
                throw 'wrong translated client bounds'
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

    def test_assistant_entry_click_prefers_rail_and_falls_back_using_one_rects_reply(self) -> None:
        self.fixture("""
            $h = [IntPtr]42
            $rail = @{name='rail:assistant'; x=280; y=64; width=20; height=16}
            $title = @{name='titlebar:assistant'; x=20; y=8; width=20; height=16}
            $other = @{name='rail:assistant_badge'; x=0; y=0; width=20; height=16}
            function ClientBounds { @{Left=40; Top=50; Right=440; Bottom=450} }
            function Call($method, $parameters, $until) {
                if ($method -cne 'ui.rects' -or $parameters.ContainsKey('prefix') -or $until -ne $deadline) {
                    throw 'expected one unfiltered observation with the existing deadline'
                }
                $script:reads++
                @{pixelsPerPoint=1.25; rects=$rects}
            }
            function Click($x, $y, $target) {
                if ($target -ne $h) { throw 'click is not scoped to the frame' }
                $script:click = "$x,$y"
            }
            function Start-Sleep {}
            foreach ($case in @(
                @{rects=@($other,$rail); point='402,140'},
                @{rects=@($title,$rail); point='402,140'},
                @{rects=@($other,$title); point='77,70'},
                @{rects=@(@{name='rail:assistant'; x=1; y=1; width=0.5; height=0.5}); point='41,51'}
            )) {
                $script:reads = 0; $script:click = $null
                $rects = $case.rects; $deadline = [datetime]::UtcNow.AddSeconds(10)
                ClickAssistantEntry $deadline
                if ($script:reads -ne 1 -or $script:click -cne $case.point) {
                    throw "wrong Assistant click: $script:click ($script:reads reads)"
                }
            }
            $rects = @($other); $deadline = [datetime]::UtcNow
            $script:reads = 0; $script:click = $null
            Reject { ClickAssistantEntry $deadline } 'rail:assistant or titlebar:assistant was not drawn'
            if ($script:reads -ne 1 -or $script:click) { throw 'clicked a target that was not drawn' }
            foreach ($badRail in @(
                @{name='rail:assistant'; y=64; width=20; height=16},
                @{name='rail:assistant'; x=315; y=64; width=20; height=16}
            )) {
                $rects = @($badRail, $title); $deadline = [datetime]::UtcNow.AddSeconds(10)
                $script:reads = 0
                Reject { ClickAssistantEntry $deadline } 'invalid geometry for rail:assistant'
                if ($script:reads -ne 1 -or $script:click) { throw 'clicked despite malformed preferred entry' }
            }
        """)


if __name__ == "__main__":
    unittest.main()
