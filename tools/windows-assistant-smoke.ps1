# Open the Assistant in the Windows app and check its web view (WebView2), which no other check
# opens: the view is made as a child window of the app's, the page in it reports to the app, a
# click on the page gives it the keyboard and a click on the rest of the window takes the keyboard
# back, the view leaves the window's resize edges free with the panel docked to the window's edge,
# WebView2 keeps its files in the app's own data folder, and the app quits cleanly. Screenshots of
# the window are taken from the screen, so the native view is in them, and go to -Evidence with the
# app's error output.
#
# On a machine without the WebView2 runtime the view cannot be made. The check then only makes sure
# that the panel went back to the built-in conversation and that the app still quits cleanly. Where
# the runtime is there, the app is started a second time with the runtime hidden from it (the
# loader looks where WEBVIEW2_BROWSER_EXECUTABLE_FOLDER says, and an empty folder holds none), to
# see the same going back.
#
# What this can't check on a CI runner (one monitor at 100 %, no GPU): input methods, a scale other
# than 100 %, files dropped on the panel, and the view over a real graphics card.
#
# Usage: tools/windows-assistant-smoke.ps1 -Exe target\ci\solidcraft.exe -Evidence assistant-smoke
param([Parameter(Mandatory = $true)][string]$Exe, [string]$Evidence = 'assistant-smoke')
$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class AssistantWin {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct GUITHREADINFO {
        public int cbSize; public int flags;
        public IntPtr hwndActive, hwndFocus, hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret;
        public RECT rcCaret;
    }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsChild(IntPtr parent, IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint thread, ref GUITHREADINFO info);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumProc f, IntPtr l);
    public static string Text(IntPtr h) { var s = new System.Text.StringBuilder(256); GetWindowTextW(h, s, 256); return s.ToString(); }
    public static string Class(IntPtr h) { var s = new System.Text.StringBuilder(256); GetClassNameW(h, s, 256); return s.ToString(); }
    // (Not named Process: PowerShell cannot call a static method of that name.)
    public static uint Owner(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return p; }
    // The process's visible top-level window with the app's title, as
    // tools/windows-resize-smoke.ps1 finds it (the process has other top-level windows, such as
    // the IME's).
    public static IntPtr MainWindow(uint pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            var title = Text(h);
            if (p == pid && IsWindowVisible(h) && (title.StartsWith("open-skyfall") || title.StartsWith("SolidCraft"))) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    // Every window inside `parent`, at any depth, whichever process made it.
    public static IntPtr[] Descendants(IntPtr parent) {
        var found = new List<IntPtr>();
        EnumChildWindows(parent, (h, l) => { found.Add(h); return true; }, IntPtr.Zero);
        return found.ToArray();
    }
    // The window that has the keyboard in the foreground thread's input queue. A child window of
    // another process shares that queue with its parent's thread.
    public static IntPtr Focus() {
        var info = new GUITHREADINFO();
        info.cbSize = Marshal.SizeOf(typeof(GUITHREADINFO));
        return GetGUIThreadInfo(0, ref info) ? info.hwndFocus : IntPtr.Zero;
    }
    public static IntPtr At(int x, int y) { POINT p; p.X = x; p.Y = y; return WindowFromPoint(p); }
}
'@

# pwsh is not DPI-aware. At a display scale other than 100 % Windows would give this script a
# smaller, scaled picture of the screen than the one the app's window lives on: its points
# would miss and its screenshots would hold a corner of the window (found on a person's machine
# at 125 %, 2026-11-11). Per-monitor awareness (context -4) for this thread, on which every
# call below is made.
$dpiBefore = [AssistantWin]::SetThreadDpiAwarenessContext([IntPtr](-4))
Write-Host ("DPI awareness: per monitor" + $(if ($dpiBefore -eq [IntPtr]::Zero) { ' was NOT taken (an older Windows): positions are right at 100 % only' } else { '' }))

$drawing = $true
try { Add-Type -AssemblyName System.Drawing } catch { $drawing = $false; Write-Host "no System.Drawing here: $($_.Exception.Message)" }

$failures = New-Object System.Collections.Generic.List[string]
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "ok: $what" } else { Write-Host "FAIL: $what"; $failures.Add($what) }
}
function Rect($h) { $r = New-Object AssistantWin+RECT; [void][AssistantWin]::GetWindowRect($h, [ref]$r); $r }
function Name($h) {
    if ($h -eq [IntPtr]::Zero) { return 'no window' }
    "window 0x{0:x} of class '{1}'" -f $h.ToInt64(), [AssistantWin]::Class($h)
}

# The version of the WebView2 runtime on this machine, or nothing: the registry values Microsoft
# documents for finding it (per machine, 64-bit and 32-bit, and per user).
function WebView2Runtime {
    $client = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    $keys = @(
        "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$client",
        "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$client",
        "HKCU:\Software\Microsoft\EdgeUpdate\Clients\$client"
    )
    foreach ($key in $keys) {
        $found = Get-ItemProperty -Path $key -Name pv -ErrorAction SilentlyContinue
        if ($found -and $found.pv -and $found.pv -ne '0.0.0.0') { return $found.pv }
    }
    return $null
}

# The app's window as the screen shows it, native child windows included (the app's own
# `ui.screenshot` holds only what the app draws).
function Shot($h, [string]$name) {
    if (-not $drawing) { Write-Host "skipped: screenshot $name (no System.Drawing)"; return }
    try {
        $r = Rect $h
        $bitmap = New-Object System.Drawing.Bitmap(($r.Right - $r.Left), ($r.Bottom - $r.Top))
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen($r.Left, $r.Top, 0, 0, $bitmap.Size)
        $path = Join-Path $Evidence "$name.png"
        $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
        $graphics.Dispose(); $bitmap.Dispose()
        Write-Host "screenshot: $path"
    } catch {
        Write-Host "skipped: screenshot $name ($($_.Exception.Message))"
    }
}

# The visible windows inside the app's window, and the web view's among them: the one wry makes
# for it, of class WRY_WEBVIEW. (Not the largest: with a real graphics card the largest is
# Chromium's "Intermediate D3D Window", which runs from the panel's corner to the monitor's size
# and is clipped by its parent; a check of a person's machine found that, 2026-11-11.) With -Any,
# the largest visible one whatever its class, for the run in which none may be there.
function Views($h, [string]$when, [switch]$Any) {
    $largest = $null
    $view = $null
    foreach ($child in [AssistantWin]::Descendants($h)) {
        $r = Rect $child
        $visible = [AssistantWin]::IsWindowVisible($child)
        Write-Host ("$when : child 0x{0:x} class '{1}' at {2},{3} - {4},{5} visible {6} process {7}" -f $child.ToInt64(), [AssistantWin]::Class($child), $r.Left, $r.Top, $r.Right, $r.Bottom, $visible, [AssistantWin]::Owner($child))
        $area = ($r.Right - $r.Left) * ($r.Bottom - $r.Top)
        if ($visible -and $area -gt 0) {
            $found = @{ Window = $child; Rect = $r; Area = $area }
            if (-not $largest -or $area -gt $largest.Area) { $largest = $found }
            if ([AssistantWin]::Class($child) -eq 'WRY_WEBVIEW' -and (-not $view -or $area -gt $view.Area)) { $view = $found }
        }
    }
    if ($Any) { $largest } else { $view }
}

# A real press and release of the left button at a point of the screen.
function Click([int]$x, [int]$y) {
    [void][AssistantWin]::SetCursorPos($x, $y)
    Start-Sleep -Milliseconds 250
    [AssistantWin]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero) # LEFTDOWN
    Start-Sleep -Milliseconds 60
    [AssistantWin]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) # LEFTUP
    Start-Sleep -Milliseconds 700
}

# Lower-case letters and spaces, as key presses.
function Keys([string]$text) {
    foreach ($c in $text.ToCharArray()) {
        $key = if ($c -eq ' ') { 0x20 } else { [byte][char]::ToUpperInvariant($c) }
        [AssistantWin]::keybd_event([byte]$key, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 20
        [AssistantWin]::keybd_event([byte]$key, 0, 0x0002, [UIntPtr]::Zero) # KEYUP
        Start-Sleep -Milliseconds 40
    }
}

$Evidence = (New-Item -ItemType Directory -Force $Evidence).FullName
$root = Join-Path $env:RUNNER_TEMP 'assistant-smoke'
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
$local = Join-Path $root 'local'
$crashes = Join-Path $root 'crashes'
$exePath = (Resolve-Path $Exe).Path
# Where WebView2 puts its files when the app does not say where: beside the exe.
$beside = "$exePath.WebView2"
$besideBefore = Test-Path $beside

$runtime = WebView2Runtime
if ($runtime) { Write-Host "WebView2 runtime: $runtime" } else { Write-Host 'WebView2 runtime: none found in the registry' }

# Start the app with settings of its own and wait for its window and its control channel. The
# window is put at a known place and size that fits the runner's screen (physical px; the minimum
# is 960 x 600) and brought to the front.
function Launch([int]$port, [hashtable]$extra) {
    $settings = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'SolidCraft\data'
    Remove-Item -Recurse -Force $settings -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $settings, $local, $crashes | Out-Null
    $script:token = -join ((1..64) | ForEach-Object { '{0:x}' -f (Get-Random -Maximum 16) })
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exePath
    foreach ($a in @('--sample', '--control', "$port")) { [void]$psi.ArgumentList.Add($a) }
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    $psi.Environment['LOCALAPPDATA'] = $local
    $psi.Environment['SOLIDCRAFT_CRASH_DIR'] = $crashes
    $psi.Environment['SOLIDCRAFT_CONTROL_TOKEN'] = $script:token
    foreach ($k in $extra.Keys) { $psi.Environment[$k] = $extra[$k] }
    $script:calls = 0
    $script:p = [System.Diagnostics.Process]::Start($psi)
    $script:stderr = $script:p.StandardError.ReadToEndAsync()
    $script:stdout = $script:p.StandardOutput.ReadToEndAsync()
    $script:h = [IntPtr]::Zero
    $deadline = (Get-Date).AddSeconds(180)
    while ($script:h -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline -and -not $script:p.HasExited) {
        $script:h = [AssistantWin]::MainWindow([uint32]$script:p.Id)
        if ($script:h -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100 }
    }
    if ($script:h -eq [IntPtr]::Zero) { throw "no window (exited: $($script:p.HasExited))" }
    Write-Host "window: '$([AssistantWin]::Text($script:h))' class '$([AssistantWin]::Class($script:h))'"
    $script:client = $null
    while (-not $script:client -and (Get-Date) -lt $deadline -and -not $script:p.HasExited) {
        try { $script:client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', $port) } catch { Start-Sleep -Milliseconds 500 }
    }
    if (-not $script:client) { throw "no control channel (exited: $($script:p.HasExited))" }
    $script:client.ReceiveTimeout = 120000
    $stream = $script:client.GetStream()
    $script:writer = New-Object System.IO.StreamWriter($stream)
    $script:writer.AutoFlush = $true
    $script:reader = New-Object System.IO.StreamReader($stream)
    $sw = [AssistantWin]::GetSystemMetrics(0); $sh = [AssistantWin]::GetSystemMetrics(1)
    Write-Host "screen: $sw x $sh"
    [void][AssistantWin]::SetWindowPos($script:h, [IntPtr]::Zero, 20, 20, [Math]::Min(1000, $sw - 40), [Math]::Min(620, $sh - 140), 0x0014) # NOZORDER | NOACTIVATE
    [void][AssistantWin]::SetForegroundWindow($script:h)
    Start-Sleep -Milliseconds 500
}

function Call([string]$method, $params = @{}) {
    $script:calls += 1
    $request = @{ id = $script:calls; method = $method; params = $params }
    if ($script:calls -eq 1) { $request.token = $script:token }
    $script:writer.WriteLine(($request | ConvertTo-Json -Compress -Depth 6))
    $line = $script:reader.ReadLine()
    if (-not $line) { throw "the control channel closed during $method" }
    $reply = $line | ConvertFrom-Json -AsHashtable
    if (-not $reply.ok) { throw "$method was refused: $line" }
    $reply.result
}

# Open the Assistant, and wait for the page to report or for the host to give the view up.
function OpenAssistant([int]$seconds) {
    $state = Call 'ui.agent_chat' @{ features = $true; open = $true }
    $until = (Get-Date).AddSeconds($seconds)
    while ($state.webView -and $state.webView.host -and -not $state.webView.page -and (Get-Date) -lt $until) {
        Start-Sleep -Milliseconds 500
        $state = Call 'ui.agent_chat'
    }
    Start-Sleep -Milliseconds 1500
    $state
}

function Quit {
    $script:writer.WriteLine((@{ id = 9999; method = 'app.quit'; params = @{ force = $true } } | ConvertTo-Json -Compress))
    Write-Host "quit: $($script:reader.ReadLine())"
    $script:client.Close()
    if (-not $script:p.WaitForExit(60000)) { throw 'did not exit after app.quit' }
    if ($script:p.ExitCode -ne 0) { throw ('exit code 0x{0:x}' -f $script:p.ExitCode) }
}

# The app's error output of a run, printed and kept; the run is ended if it has not ended.
# Nothing here waits without a limit: an app that started and never showed a window (the 32-bit
# build on a CI runner, 2026-11-11) kept this script waiting for the end of its error output for
# as long as the job ran. The app is ended with every process it started, and the output is
# taken as far as it has come if it does not end.
function Finish([string]$name) {
    if ($script:p -and -not $script:p.HasExited) {
        Write-Host 'the app is still running: ending it and every process it started'
        try { $script:p.Kill($true) } catch { Write-Host "could not end it: $($_.Exception.Message)" }
        [void]$script:p.WaitForExit(15000)
    }
    $err = ''
    if ($script:stderr) {
        if ($script:stderr.Wait(15000)) { $err = $script:stderr.Result } else { $err = "(the app's error output did not end within 15 s: a process it started still holds it open)" }
    }
    Set-Content -Path (Join-Path $Evidence "$name-app-error-output.txt") -Value $err
    Write-Host $err
    $err
}

Write-Host '=== the Assistant with the runtime this machine has'
$made = $false
try {
    Launch 47951 @{}
    [void](Call 'ui.screenshot' @{ path = (Join-Path $Evidence '00-app-frame-before.png') })
    $state = OpenAssistant 90
    Check ($null -ne $state.webView) "the Assistant's status says how the conversation is shown (webView: $($state.webView | ConvertTo-Json -Compress))"
    Shot $h '01-assistant-open'
    [void](Call 'ui.screenshot' @{ path = (Join-Path $Evidence '01-app-frame-assistant-open.png') })
    $view = Views $h 'assistant open'

    if ($state.webView -and $state.webView.host -and $state.webView.page) {
        $made = $true
        Check $true 'the page in the web view reported to the app'
        Check ($null -ne $view) 'the web view is a visible window inside the app window'
        $w = Rect $h
        if ($view) {
            $v = $view.Rect
            Write-Host "window $($w.Left),$($w.Top) - $($w.Right),$($w.Bottom); view $($v.Left),$($v.Top) - $($v.Right),$($v.Bottom)"
            Check ($v.Left -ge $w.Left -and $v.Top -ge $w.Top -and $v.Right -le $w.Right -and $v.Bottom -le $w.Bottom) 'the view lies inside the app window'

            # The keyboard: to the page on a click in it, back to the window on a click elsewhere.
            # The message box: the text is above the row of buttons at the foot of the page (a
            # click lower down, at 85 px from the foot, opened the menu of one of those buttons).
            # (Measured at 100 %; the page is drawn larger at a larger display scale.)
            $scale = [AssistantWin]::GetDpiForWindow($h) / 96.0
            if ($scale -le 0) { $scale = 1.0 }
            Write-Host "display scale of the app's window: $([int]($scale * 100)) %"
            $x = $v.Left + [int](90 * $scale); $y = $v.Bottom - [int](150 * $scale)
            [void][AssistantWin]::SetCursorPos($x, $y)
            $pt = New-Object AssistantWin+POINT; [void][AssistantWin]::GetCursorPos([ref]$pt)
            if ($pt.X -ne $x -or $pt.Y -ne $y) {
                Write-Host "skipped: the keyboard checks (this session doesn't move the pointer: at $($pt.X),$($pt.Y))"
            } else {
                Click $x $y
                $focus = [AssistantWin]::Focus()
                $inPage = ($focus -ne [IntPtr]::Zero -and $focus -ne $h -and [AssistantWin]::IsChild($h, $focus))
                if ($focus -eq [IntPtr]::Zero) {
                    Write-Host 'unknown: which window has the keyboard after a click on the page (Windows named none)'
                } else {
                    Check $inPage "a click on the message box gives the page the keyboard ($(Name $focus))"
                }
                if ($inPage) {
                    Keys 'hello windows'
                    Start-Sleep -Milliseconds 800
                }
                Shot $h '02-after-click-and-typing-on-page'
                # The 3D view, in the part of the window the panel leaves free.
                $free = if (($v.Left - $w.Left) -ge ($w.Right - $v.Right)) { [int](($w.Left + $v.Left) / 2) } else { [int](($v.Right + $w.Right) / 2) }
                Click $free ([int](($w.Top + $w.Bottom) / 2))
                # The app moves the keyboard on the click's own message; give that message a
                # moment to arrive before asking which window has it (a runner read the page's
                # window on the next line, 2026-11-13).
                Start-Sleep -Milliseconds 500
                $focus = [AssistantWin]::Focus()
                if ($focus -eq [IntPtr]::Zero) {
                    Write-Host 'unknown: which window has the keyboard after a click on the rest of the window (Windows named none)'
                } else {
                    Check ($focus -eq $h) "a click on the rest of the window gives the window the keyboard back ($(Name $focus))"
                    if (-not $inPage) { Write-Host 'note: the page was not seen to have the keyboard before that click, so the line above says little' }
                }
                Shot $h '03-after-click-on-3d-view'
            }
        }

        # Docked to the window's edge (the compact layout), the view leaves the resize edges free:
        # at 2 px inside the window's edge the pointer is over the app's window, not over the view.
        [void](Call 'engine.execute' @{ command = 'view.layout'; params = @{ mode = 'compact' } })
        Start-Sleep -Milliseconds 2000
        [void](Call 'ui.agent_chat')
        Start-Sleep -Milliseconds 500
        $docked = Views $h 'compact layout'
        Shot $h '04-compact-layout'
        $w = Rect $h
        if ($docked) {
            $v = $docked.Rect
            Write-Host "compact layout: window $($w.Left),$($w.Top) - $($w.Right),$($w.Bottom); view $($v.Left),$($v.Top) - $($v.Right),$($v.Bottom)"
            $cx = [int](($v.Left + $v.Right) / 2); $cy = [int](($v.Top + $v.Bottom) / 2)
            $side = if (($w.Right - $v.Right) -le ($v.Left - $w.Left)) { @{ Name = 'right'; X = $w.Right - 3 } } else { @{ Name = 'left'; X = $w.Left + 2 } }
            $at = [AssistantWin]::At($side.X, $cy)
            Check ($at -eq $h) "compact layout: the window's $($side.Name) edge beside the panel is the app window's ($(Name $at))"
            $at = [AssistantWin]::At($cx, $w.Bottom - 3)
            Check ($at -eq $h) "compact layout: the window's bottom edge under the panel is the app window's ($(Name $at))"
        } else {
            Check $false 'compact layout: the web view is a visible window inside the app window'
        }

        $data = Join-Path $local 'SolidCraft\assistant-web-view'
        Check (Test-Path $data) "WebView2 keeps its files in the app's data folder ($data)"
        Check ($besideBefore -or -not (Test-Path $beside)) "WebView2 wrote nothing beside the exe ($beside)"
    } elseif ($state.webView -and -not $state.webView.host) {
        if ($runtime) {
            Check $false "the WebView2 runtime $runtime is installed, but the web view could not be made (the app's error output below says why)"
        } else {
            Write-Host 'skipped: the web view checks (no WebView2 runtime on this machine; the panel draws the built-in conversation)'
        }
    } else {
        Check $false 'the page in the web view reported to the app within 90 s'
    }
    Quit
} catch {
    Write-Host "FAIL: $($_.Exception.Message)"
    $failures.Add($_.Exception.Message)
} finally {
    [void](Finish 'first-run')
}

# The same app with no runtime to be found: the panel goes back to the built-in conversation, the
# app says why on its error output, and it still quits cleanly.
if ($made) {
    Write-Host '=== the Assistant with the runtime hidden'
    $none = Join-Path $root 'no-runtime'
    New-Item -ItemType Directory -Force $none | Out-Null
    $fellBack = $false
    try {
        Launch 47952 @{ WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $none }
        $state = OpenAssistant 60
        Shot $h '05-runtime-hidden'
        [void](Call 'ui.screenshot' @{ path = (Join-Path $Evidence '05-app-frame-runtime-hidden.png') })
        $left = Views $h 'runtime hidden' -Any
        if ($state.webView -and -not $state.webView.host) {
            $fellBack = $true
            Check $true 'with the runtime hidden, the app gives the view up and draws the built-in conversation'
            # The view's toolkit begins with a window of its own in the app's; one left shown
            # there lies over the built-in conversation as a blank box.
            $what = if ($left) { Name $left.Window } else { 'none' }
            Check ($null -eq $left) "with the runtime hidden, no window is left shown over the panel ($what)"
        } elseif ($state.webView -and $state.webView.page) {
            Write-Host 'unknown: the app without a runtime (an empty WEBVIEW2_BROWSER_EXECUTABLE_FOLDER did not keep the view from being made)'
        } else {
            Check $false 'with the runtime hidden, the app gives the view up within 60 s'
        }
        Quit
    } catch {
        Write-Host "FAIL: with the runtime hidden: $($_.Exception.Message)"
        $failures.Add("with the runtime hidden: " + $_.Exception.Message)
    } finally {
        $said = Finish 'runtime-hidden'
    }
    if ($fellBack) { Check ($said -match 'assistant web view: could not be created') "with the runtime hidden, the app's error output says the view could not be created" }
}

$reports = @(Get-ChildItem -Recurse -Filter report.json $crashes -ErrorAction SilentlyContinue)
foreach ($r in $reports) { Get-Content $r.FullName | Write-Host }
if ($reports.Count -gt 0) { $failures.Add('a crash report was written') }
if ($failures.Count -gt 0) { Write-Error ("assistant web view: " + ($failures -join '; ')) }
Write-Host 'PASS: assistant web view'
