# Open the Assistant in the Windows app and check its web view (WebView2), which no other check
# opens: the view is made as a child window of the app's, the page in it reports to the app, a
# click on the page gives it the keyboard and a click on the rest of the window takes the keyboard
# back, the view leaves the window's resize edges free with the panel docked to the window's edge,
# WebView2 keeps its files in the app's own data folder, and the app quits cleanly. Screenshots of
# the window are taken from the screen, so the native view is in them, and go to -Evidence with the
# app's error output.
#
# The Assistant is WebView2 and has no built-in fallback. The runtime is detected before the app
# starts (packaging/windows/webview2-runtime.ps1). When it is absent this check does not pass:
# -WebView2Recovery Auto installs it (online bootstrapper, or -WebView2OfflineInstaller when one
# is supplied); ReportOnly writes that the Assistant page was NOT checked and fails. With the
# runtime present the page must load and report in.
#
# The app is then started once more with the runtime hidden from it (the loader looks where
# WEBVIEW2_BROWSER_EXECUTABLE_FOLDER says, and an empty folder holds none). The app must report
# that the view could not be made and leave no blank window over the panel: a silent blank panel
# is a failure.
#
# What this can't check on a CI runner (one monitor at 100 %, no GPU): input methods, a scale other
# than 100 %, files dropped on the panel, and the view over a real graphics card.
#
# Usage: tools/windows-assistant-smoke.ps1 -Exe target\ci\intcad.exe -Evidence assistant-smoke
# Use a disposable runner account. The installed-package caller applies and verifies its own
# app/WebView2 firewall rules before calling this script; leave that isolation in place.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Evidence = 'assistant-smoke',
    [ValidateSet('Auto', 'Online', 'Offline', 'ReportOnly')][string]$WebView2Recovery = 'Auto',
    [string]$WebView2OfflineInstaller = $env:SOLIDCRAFT_WEBVIEW2_OFFLINE_INSTALLER,
    [string]$WebView2InstallerSha256 = $env:SOLIDCRAFT_WEBVIEW2_INSTALLER_SHA256
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or -not [Environment]::UserInteractive) {
    throw 'BLOCKED: Windows Assistant smoke needs an interactive Windows desktop'
}
if (-not (Get-Command Get-NetTCPConnection -ErrorAction SilentlyContinue)) {
    throw 'BLOCKED: cannot verify ownership of the local control listener'
}

$runtimeHelper = Join-Path (Split-Path $PSScriptRoot -Parent) 'packaging\windows\webview2-runtime.ps1'
if (-not (Test-Path -LiteralPath $runtimeHelper)) { throw "WebView2 helper not found: $runtimeHelper" }
# The helper declares its own `$Evidence` parameter; dot-sourcing would replace our evidence
# folder with its default (empty). Keep the caller's value.
$EvidenceBeforeHelper = $Evidence
. $runtimeHelper
$Evidence = $EvidenceBeforeHelper

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
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
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
            if (p == pid && IsWindowVisible(h) && (title.StartsWith("Intcad") || title.StartsWith("SolidCraft"))) { found = h; return false; }
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
if ($dpiBefore -eq [IntPtr]::Zero) { throw 'BLOCKED: cannot establish physical screen coordinates (per-monitor DPI)' }

try { Add-Type -AssemblyName System.Drawing }
catch { throw "BLOCKED: screen capture unavailable: $($_.Exception.Message)" }

$failures = New-Object System.Collections.Generic.List[string]
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "ok: $what" } else { Write-Host "FAIL: $what"; $failures.Add($what) }
}
function Rect($h) {
    $r = New-Object AssistantWin+RECT
    if (-not [AssistantWin]::GetWindowRect($h, [ref]$r)) { throw "cannot read bounds of $(Name $h)" }
    $r
}
function Name($h) {
    if ($h -eq [IntPtr]::Zero) { return 'no window' }
    "window 0x{0:x} of class '{1}'" -f $h.ToInt64(), [AssistantWin]::Class($h)
}

# WebView2 detection and recovery live in packaging/windows/webview2-runtime.ps1, dot-sourced
# above. It checks Microsoft's per-machine (native and WOW6432Node) and per-user keys and the
# known version folders, and never treats pv 0.0.0.0 as an install.

# The app's window as the screen shows it, native child windows included (the app's own
# `ui.screenshot` holds only what the app draws).
function Shot($h, [string]$name) {
    $bitmap = $graphics = $null
    try {
        $r = Rect $h
        $bitmap = New-Object System.Drawing.Bitmap(($r.Right - $r.Left), ($r.Bottom - $r.Top))
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen($r.Left, $r.Top, 0, 0, $bitmap.Size)
        $path = Join-Path $Evidence "$name.png"
        $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
        Write-Host "screenshot: $path"
    } catch {
        throw "BLOCKED: cannot capture the desktop for $name ($($_.Exception.Message))"
    } finally {
        if ($graphics) { $graphics.Dispose() }
        if ($bitmap) { $bitmap.Dispose() }
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
        if ($visible) {
            $found = @{ Window = $child; Rect = $r; Area = $area }
            if (-not $largest -or $area -gt $largest.Area) { $largest = $found }
            if ([AssistantWin]::Class($child) -eq 'WRY_WEBVIEW' -and (-not $view -or $area -gt $view.Area)) { $view = $found }
        }
    }
    if ($Any) { $largest } else { $view }
}

# A real press and release of the left button at a point of the screen.
function Click([int]$x, [int]$y, [IntPtr]$target) {
    $pt = New-Object AssistantWin+POINT
    if (-not [AssistantWin]::SetCursorPos($x, $y)) { throw 'BLOCKED: cannot move the desktop pointer' }
    Start-Sleep -Milliseconds 250
    if (-not [AssistantWin]::GetCursorPos([ref]$pt) -or $pt.X -ne $x -or $pt.Y -ne $y -or
        [AssistantWin]::GetForegroundWindow() -ne $h) {
        throw 'BLOCKED: cannot direct desktop input to the smoke window'
    }
    $over = [AssistantWin]::At($x, $y)
    if ($over -ne $target -and -not [AssistantWin]::IsChild($target, $over)) {
        throw "click at $x,$y would hit $(Name $over), not $(Name $target)"
    }
    [AssistantWin]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero) # LEFTDOWN
    Start-Sleep -Milliseconds 60
    [AssistantWin]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) # LEFTUP
    Start-Sleep -Milliseconds 700
}

# Windows moves the keyboard in its own messages, and a busy frame can delay that past the click's
# pause. Poll for the window the check expects; a real failure still ends on the wrong window or
# none at all within the limit.
function WaitFocus([scriptblock]$ok, [int]$seconds = 5) {
    $until = (Get-Date).AddSeconds($seconds)
    do {
        $focus = [AssistantWin]::Focus()
        if (& $ok $focus) { return $focus }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $until)
    $focus
}

# Lower-case letters and spaces, as key presses.
function Keys([string]$text) {
    foreach ($c in $text.ToCharArray()) {
        if ([AssistantWin]::GetForegroundWindow() -ne $h) { throw 'BLOCKED: the smoke window lost foreground before typing' }
        $key = if ($c -eq ' ') { 0x20 } else { [byte][char]::ToUpperInvariant($c) }
        [AssistantWin]::keybd_event([byte]$key, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 20
        [AssistantWin]::keybd_event([byte]$key, 0, 0x0002, [UIntPtr]::Zero) # KEYUP
        Start-Sleep -Milliseconds 40
    }
}

$Evidence = (New-Item -ItemType Directory -Force $Evidence).FullName
$temp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$root = Join-Path $temp ('assistant-smoke-' + [guid]::NewGuid().ToString('N'))
$crashes = Join-Path $root 'crashes'
$exePath = (Resolve-Path $Exe).Path
# Where WebView2 puts its files when the app does not say where: beside the exe.
$beside = "$exePath.WebView2"
$besideBefore = Test-Path $beside

$runtime = Get-WebView2Runtime
if ($runtime -and -not $runtime.Healthy) {
    throw "the WebView2 registry names version $($runtime.Version), but msedgewebview2.exe is missing; the runtime is damaged, not present"
}
if (-not $runtime) {
    if ($WebView2Recovery -eq 'ReportOnly') {
        $gap = "Windows Assistant check: the WebView2 Runtime is NOT installed. The Assistant page was NOT checked, so this is not a pass. Install it with packaging/windows/webview2-runtime.ps1 -Action Install (online, or offline with -OfflineInstaller and -ExpectedSha256)."
        Write-Host "BLOCKED: $gap"
        if ($env:GITHUB_STEP_SUMMARY) { Add-Content -Path $env:GITHUB_STEP_SUMMARY -Value $gap }
        throw "BLOCKED: $gap"
    }
    $installMode = switch ($WebView2Recovery) {
        'Online' { 'Online' }
        'Offline' { 'Offline' }
        default { 'Auto' }
    }
    Write-Host "==> WebView2 runtime absent: $WebView2Recovery recovery"
    $runtime = Install-WebView2Runtime -InstallMode $installMode -InstallScope 'Auto' `
        -InstallerPath $WebView2OfflineInstaller -Sha256 $WebView2InstallerSha256 -EvidenceDirectory $Evidence
    if (-not $runtime) { throw 'the WebView2 recovery path returned no runtime' }
    Write-Host "WebView2 runtime installed by the recovery path: $($runtime.Version) ($($runtime.Scope), $($runtime.Source))"
}
Write-Host "WebView2 runtime: $($runtime.Version) ($($runtime.Scope), $($runtime.Source))"

# Start the app with settings of its own and wait for its window and its control channel. The
# window is put at a known place and size that fits the runner's screen (physical px; the minimum
# is 960 x 600) and brought to the front.
function Launch([int]$port, [hashtable]$extra) {
    $profile = Join-Path $root "$port"
    $script:settings = Join-Path $profile 'settings'
    $local = Join-Path $profile 'local'
    $roaming = Join-Path $profile 'roaming'
    $control = Join-Path $profile 'control'
    New-Item -ItemType Directory -Force $settings, $local, $roaming, $control, $crashes | Out-Null
    $script:token = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant()
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exePath
    foreach ($a in @('--sample', '--control', "$port")) { [void]$psi.ArgumentList.Add($a) }
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    foreach ($key in @($psi.Environment.Keys)) {
        if ($key -like 'WEBVIEW2_*') { [void]$psi.Environment.Remove($key) }
    }
    $psi.Environment['APPDATA'] = $roaming
    $psi.Environment['LOCALAPPDATA'] = $local
    $psi.Environment['SOLIDCRAFT_SETTINGS_DIR'] = $settings
    $psi.Environment['SOLIDCRAFT_RECOVERY_DIR'] = Join-Path $profile 'recovery'
    $psi.Environment['SOLIDCRAFT_CONTROL_DIR'] = $control
    $psi.Environment['SOLIDCRAFT_VALIDATION_DIR'] = Join-Path $profile 'validation'
    $psi.Environment['SOLIDCRAFT_CRASH_DIR'] = $crashes
    $psi.Environment['SOLIDCRAFT_CONTROL_TOKEN'] = $script:token
    $psi.Environment['SOLIDCRAFT_UPDATE_CHECK'] = '0'
    foreach ($k in $extra.Keys) { $psi.Environment[$k] = $extra[$k] }
    $script:calls = 0
    $script:client = $script:stderr = $script:stdout = $null
    $script:p = [System.Diagnostics.Process]::Start($psi)
    $script:stderr = $script:p.StandardError.ReadToEndAsync()
    $script:stdout = $script:p.StandardOutput.ReadToEndAsync()
    $script:h = [IntPtr]::Zero
    $deadline = [datetime]::UtcNow.AddSeconds(180)
    while ($script:h -eq [IntPtr]::Zero -and [datetime]::UtcNow -lt $deadline -and -not $script:p.HasExited) {
        $script:h = [AssistantWin]::MainWindow([uint32]$script:p.Id)
        if ($script:h -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100 }
    }
    if ($script:h -eq [IntPtr]::Zero) { throw "no window (exited: $($script:p.HasExited))" }
    Write-Host "window: '$([AssistantWin]::Text($script:h))' class '$([AssistantWin]::Class($script:h))'"
    # Only connect to a listener owned by this process, not a stale/other occupant of the port.
    while (-not $script:client -and [datetime]::UtcNow -lt $deadline -and -not $script:p.HasExited) {
        $listener = @(Get-NetTCPConnection -LocalAddress '127.0.0.1' -LocalPort $port -State Listen -ErrorAction SilentlyContinue)
        if ($listener.Count -gt 0) {
            if ($listener.Count -ne 1 -or $listener[0].OwningProcess -ne $script:p.Id) {
                throw "control port $port does not belong to the launched process"
            }
            $client = [Net.Sockets.TcpClient]::new()
            try {
                [void]$client.ConnectAsync('127.0.0.1', $port).Wait((RemainingMilliseconds $deadline))
                if ($client.Connected) { $script:client = $client }
            } finally {
                if (-not $script:client) { $client.Dispose() }
            }
        } else { Start-Sleep -Milliseconds 200 }
    }
    if (-not $script:client) { throw "no control channel (exited: $($script:p.HasExited))" }
    $stream = $script:client.GetStream()
    $script:writer = New-Object System.IO.StreamWriter($stream)
    $script:writer.AutoFlush = $true
    $script:reader = New-Object System.IO.StreamReader($stream)
    $sw = [AssistantWin]::GetSystemMetrics(0); $sh = [AssistantWin]::GetSystemMetrics(1)
    Write-Host "screen: $sw x $sh"
    if ($sw -lt 1000 -or $sh -lt 740) { throw 'BLOCKED: the desktop is too small for the 960 x 600 smoke window and margins' }
    if (-not [AssistantWin]::SetWindowPos($script:h, [IntPtr]::Zero, 20, 20, [Math]::Min(1000, $sw - 40), [Math]::Min(620, $sh - 140), 0x0014)) {
        throw 'BLOCKED: cannot place the smoke window'
    }
    [void][AssistantWin]::SetForegroundWindow($script:h)
    Start-Sleep -Milliseconds 500
}

function RemainingMilliseconds([datetime]$until) {
    $ms = [Math]::Floor(($until - [datetime]::UtcNow).TotalMilliseconds)
    if ($ms -lt 1) { throw 'operation deadline expired' }
    [int]$ms
}

function Call([string]$method, $params = @{}, [datetime]$until = [datetime]::UtcNow.AddSeconds(15)) {
    if ($method -cnotin @('ui.inspect', 'ui.rects', 'ui.screenshot')) { throw 'smoke requires read-only observations' }
    $script:calls += 1
    $request = @{ id = $script:calls; method = $method; params = $params }
    if ($script:calls -eq 1) { $request.token = $script:token }
    $script:client.SendTimeout = RemainingMilliseconds $until
    $script:writer.WriteLine(($request | ConvertTo-Json -Compress -Depth 6))
    $read = $script:reader.ReadLineAsync()
    if (-not $read.Wait((RemainingMilliseconds $until))) { throw "control read timed out during $method" }
    $line = $read.Result
    if (-not $line) { throw "the control channel closed during $method" }
    $reply = $line | ConvertFrom-Json -AsHashtable
    if ($reply.id -ne $script:calls -or $reply.ok -isnot [bool] -or -not $reply.ok) { throw "$method did not return a successful matching reply" }
    $reply.result
}

function ReadAssistant($inspection) {
    $state = $inspection.assistant
    if ($state -isnot [Collections.IDictionary] -or $state.open -isnot [bool] -or
        $state.surface_ready -isnot [bool] -or $state.host -isnot [string] -or
        $state.host -cnotin @('available', 'unavailable', 'native_wayland', 'failed')) {
        throw 'missing or invalid ui.inspect.assistant marker'
    }
    $state
}

function ScreenRect($result, [string]$name, $origin) {
    $entry = $result.rects | Where-Object { $_.name -ceq $name } | Select-Object -Last 1
    if (-not $entry) { return }
    $scale = $result.pixelsPerPoint
    if ($scale -le 0 -or -not [double]::IsFinite($scale) -or $entry.width -le 0 -or $entry.height -le 0) {
        throw "invalid geometry for $name"
    }
    @{
        Left   = $origin.X + [int]($entry.x * $scale)
        Top    = $origin.Y + [int]($entry.y * $scale)
        Right  = $origin.X + [int](($entry.x + $entry.width) * $scale)
        Bottom = $origin.Y + [int](($entry.y + $entry.height) * $scale)
    }
}

function TargetRect([string]$name, [datetime]$until) {
    $result = Call 'ui.rects' @{ prefix = $name } $until
    $origin = New-Object AssistantWin+POINT
    if (-not [AssistantWin]::ClientToScreen($h, [ref]$origin)) { throw 'cannot locate the smoke window client origin' }
    ScreenRect $result $name $origin
}

function ClickTarget([string]$name, [datetime]$until) {
    do {
        $r = TargetRect $name $until
        if ($r) {
            Click ([int](($r.Left + $r.Right) / 2)) ([int](($r.Top + $r.Bottom) / 2)) $h
            return
        }
        Start-Sleep -Milliseconds 150
    } while ([datetime]::UtcNow -lt $until)
    throw "native UI target $name was not drawn"
}

function WaitPage([datetime]$until, [string]$layout = '') {
    do {
        $inspection = Call 'ui.inspect' @{} $until
        $state = ReadAssistant $inspection
        if ($state.host -cin @('failed', 'unavailable', 'native_wayland')) { throw "installed WebView2 host: $($state.host)" }
        if ($state.open -and $state.surface_ready -and (-not $layout -or $inspection.ui.layout -ceq $layout)) {
            $expected = TargetRect 'agent_chat:web' $until
            $view = Views $h 'Assistant ready'
            if ($view -and $expected) {
                $v = $view.Rect; $w = Rect $h
                # Rust and Win32 can round edges independently by a pixel.
                $differences = @('Left', 'Top', 'Right', 'Bottom' | ForEach-Object { [Math]::Abs($v.$_ - $expected.$_) })
                if (($differences | Measure-Object -Maximum).Maximum -le 2 -and
                    $v.Left -ge $w.Left -and $v.Top -ge $w.Top -and $v.Right -le $w.Right -and $v.Bottom -le $w.Bottom -and
                    $v.Right -gt $v.Left -and $v.Bottom -gt $v.Top) {
                    return $view
                }
                Write-Host "native page edges differ from agent_chat:web by $($differences -join ', ') px"
            }
        }
        Start-Sleep -Milliseconds 200
    } while ([datetime]::UtcNow -lt $until)
    throw "did not see an open, surface-ready Assistant with matching native bounds (layout: '$layout')"
}

function PageClosePoint($page, $candidates) {
    $valid = @($candidates | Where-Object {
        $_.Enabled -eq $true -and $_.Offscreen -eq $false -and
        $_.Right -gt $_.Left -and $_.Bottom -gt $_.Top -and
        $_.Left -ge $page.Left -and $_.Top -ge $page.Top -and $_.Right -le $page.Right -and $_.Bottom -le $page.Bottom
    })
    if ($valid.Count -ne 1) { throw 'no unique clickable page Close inside the live WebView' }
    @{
        X = [int][Math]::Floor(($valid[0].Left + $valid[0].Right) / 2)
        Y = [int][Math]::Floor(($valid[0].Top + $valid[0].Bottom) / 2)
    }
}

function FindPageClose($view) {
    # Use Windows' inbox UIA assemblies via Windows PowerShell (.NET Framework).
    # This reader sends no input. A separate process bounds even a stalled UIA provider.
    $scriptText = '$child = [IntPtr]' + $view.Window.ToInt64() + "`n" + {
        $ErrorActionPreference = 'Stop'
        try { Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase }
        catch { [Console]::Error.WriteLine($_.Exception.Message); exit 2 }
        $name = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Close')
        $button = [Windows.Automation.PropertyCondition]::new(
            [Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Button)
        $condition = [Windows.Automation.AndCondition]::new($name, $button)
        $element = [Windows.Automation.AutomationElement]::FromHandle($child)
        if ($null -eq $element) { throw 'the live WebView is not exposed to UI Automation' }
        $found = @($element.FindAll([Windows.Automation.TreeScope]::Descendants, $condition) | ForEach-Object {
            $current = $_.Current; $rect = $current.BoundingRectangle
            if (-not $rect.IsEmpty) {
                @{ Left = $rect.Left; Top = $rect.Top; Right = $rect.Right; Bottom = $rect.Bottom
                   Enabled = $current.IsEnabled; Offscreen = $current.IsOffscreen }
            }
        })
        ConvertTo-Json -InputObject $found -Compress
    }.ToString()
    $info = [Diagnostics.ProcessStartInfo]::new((Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'))
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($arg in @('-NoProfile', '-NonInteractive', '-Mta', '-EncodedCommand',
            [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($scriptText)))) {
        [void]$info.ArgumentList.Add($arg)
    }
    try { $uia = [Diagnostics.Process]::Start($info) }
    catch { throw "BLOCKED: Windows UI Automation reader cannot start: $($_.Exception.Message)" }
    try {
        $output = $uia.StandardOutput.ReadToEndAsync()
        $errorOutput = $uia.StandardError.ReadToEndAsync()
        if (-not $uia.WaitForExit(15000)) { throw 'page Close UI Automation query timed out' }
        if (-not $errorOutput.Wait(1000) -or -not $output.Wait(1000)) { throw 'page Close UI Automation output did not end' }
        if ($uia.ExitCode -eq 2) { throw "BLOCKED: Windows UI Automation assemblies unavailable: $($errorOutput.Result)" }
        if ($uia.ExitCode -ne 0) { throw "page Close UI Automation query failed: $($errorOutput.Result)" }
        $output.Result | ConvertFrom-Json -AsHashtable
    } finally {
        if (-not $uia.HasExited) { $uia.Kill() }
        $uia.Dispose()
    }
}

function ClosePage($view) {
    $point = PageClosePoint $view.Rect (FindPageClose $view)
    Click $point.X $point.Y $view.Window
    $until = [datetime]::UtcNow.AddSeconds(15)
    do {
        if ($script:p.HasExited) { throw 'app exited during page Close' }
        $state = ReadAssistant (Call 'ui.inspect' @{} $until)
        if (PageClosed $state (Views $h 'after page Close') (-not $script:p.HasExited)) { return }
        Start-Sleep -Milliseconds 200
    } while ([datetime]::UtcNow -lt $until)
    throw 'page Close did not close the panel and hide its native child'
}

function PageClosed($state, $visiblePage, [bool]$appAlive) {
    if (-not $appAlive) { throw 'app exited during page Close' }
    $state.open -eq $false -and $null -eq $visiblePage
}

function RequireCreationDiagnostic([string]$stderr) {
    if ($stderr -cnotmatch '(?m)^solidcraft: assistant web view: could not be created(?:: [^\r\n]+| in this window)\r?$') {
        throw 'hidden runtime: missing the native creation diagnostic in stderr'
    }
}

function WaitCleanExit($process, [scriptblock]$close) {
    if ($process.HasExited) { throw 'app exited before main window close' }
    & $close
    if (-not $process.WaitForExit(60000)) { throw 'app did not exit within 60 s of main window close' }
    if ($process.ExitCode -ne 0) { throw ('exit code 0x{0:x} after main window close' -f $process.ExitCode) }
}

function Quit {
    $script:client.Dispose()
    $script:client = $null
    WaitCleanExit $script:p {
        if ([AssistantWin]::MainWindow([uint32]$script:p.Id) -ne $h -or [AssistantWin]::Owner($h) -ne $script:p.Id -or
            -not [AssistantWin]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { # WM_CLOSE
            throw 'cannot close the owned main window'
        }
    }
    Write-Host 'ok: the owned main window closed with exit code zero'
}

# The app's error output of a run, printed and kept; the run is ended if it has not ended.
# Nothing here waits without a limit: an app that started and never showed a window (the 32-bit
# build on a CI runner, 2026-11-11) kept this script waiting for the end of its error output for
# as long as the job ran. The app is ended with every process it started, and the output is
# taken as far as it has come if it does not end.
function Finish([string]$name) {
    if ($script:client) { $script:client.Dispose(); $script:client = $null }
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
    $initial = Call 'ui.inspect'
    if ((ReadAssistant $initial).open -or $initial.ui.layout -cne 'floating') {
        throw 'the isolated app did not start with the Assistant closed and Floating layout'
    }
    [void](Call 'ui.screenshot' @{ path = (Join-Path $Evidence '00-app-frame-before.png') })
    $until = [datetime]::UtcNow.AddSeconds(90)
    ClickTarget 'titlebar:assistant' $until
    $view = WaitPage $until 'floating'
    Write-Host 'ok: the open Assistant reports current-view readiness and matches the visible native page'
    Shot $h '01-assistant-open'
    [void](Call 'ui.screenshot' @{ path = (Join-Path $Evidence '01-app-frame-assistant-open.png') })

    # Do not submit a chat. A click in the message box lets the page take the keyboard;
    # a click in the 3D view must give the keyboard back.
    $w = Rect $h; $v = $view.Rect
    $scale = [AssistantWin]::GetDpiForWindow($h) / 96.0
    if ($scale -le 0) { throw 'BLOCKED: cannot read the desktop DPI' }
    Click ($v.Left + [int](90 * $scale)) ($v.Bottom - [int](150 * $scale)) $view.Window
    $focus = WaitFocus { param($f) $f -eq $view.Window -or [AssistantWin]::IsChild($view.Window, $f) }
    if ($focus -eq [IntPtr]::Zero) { throw 'BLOCKED: Windows reported no keyboard focus after clicking the page' }
    $inPage = $focus -eq $view.Window -or [AssistantWin]::IsChild($view.Window, $focus)
    Check $inPage "a click on the message box gives the page the keyboard ($(Name $focus))"
    if ($inPage) { Keys 'hello windows' }
    Shot $h '02-after-click-and-typing-on-page'
    $free = if (($v.Left - $w.Left) -ge ($w.Right - $v.Right)) { [int](($w.Left + $v.Left) / 2) } else { [int](($v.Right + $w.Right) / 2) }
    Click $free ([int](($w.Top + $w.Bottom) / 2)) $h
    $focus = WaitFocus { param($f) $f -eq $h }
    if ($focus -eq [IntPtr]::Zero) { throw 'BLOCKED: Windows reported no keyboard focus after clicking the frame' }
    Check ($focus -eq $h) "a click on the 3D view gives the frame the keyboard back ($(Name $focus))"
    Shot $h '03-after-click-on-3d-view'

    ClosePage $view
    Write-Host 'ok: page Close hid the native child and the app is still running'
    Shot $h '03a-page-closed'
    $until = [datetime]::UtcNow.AddSeconds(30)
    ClickTarget 'titlebar:assistant' $until
    $reopened = WaitPage $until 'floating'
    if ($reopened.Window -ne $view.Window) { throw 'reopening did not reuse the retained native child' }
    Shot $h '03b-page-reopened'

    # Switch layout through the real search result while the Assistant is open.
    $until = [datetime]::UtcNow.AddSeconds(30)
    ClickTarget 'ribbon:search' $until
    Keys 'workspace layout'
    ClickTarget 'search:command:view.layout' $until
    $docked = WaitPage $until 'compact'
    Shot $h '04-compact-layout'
    $w = Rect $h; $v = $docked.Rect
    $cx = [int](($v.Left + $v.Right) / 2); $cy = [int](($v.Top + $v.Bottom) / 2)
    $side = if (($w.Right - $v.Right) -le ($v.Left - $w.Left)) { @{ Name = 'right'; X = $w.Right - 3 } } else { @{ Name = 'left'; X = $w.Left + 2 } }
    $at = [AssistantWin]::At($side.X, $cy)
    Check ($at -eq $h) "compact layout: the window's $($side.Name) edge beside the panel is the app window's ($(Name $at))"
    $at = [AssistantWin]::At($cx, $w.Bottom - 3)
    Check ($at -eq $h) "compact layout: the window's bottom edge under the panel is the app window's ($(Name $at))"

    $data = Join-Path $settings 'assistant-web-view'
    Check (Test-Path -LiteralPath $data -PathType Container) "WebView2 keeps its files in the isolated settings folder ($data)"
    Check ($besideBefore -or -not (Test-Path -LiteralPath $beside)) "WebView2 did not create a new profile beside the exe ($beside)"
    Quit
    $made = $true
} catch {
    if ($_.Exception.Message.StartsWith('BLOCKED:')) { Write-Host $_.Exception.Message } else { Write-Host "FAIL: $($_.Exception.Message)" }
    $failures.Add($_.Exception.Message)
} finally {
    [void](Finish 'first-run')
}

# The same app with no runtime to be found: the app must report that the view could not be made
# and leave no blank window over the panel. It must not quietly pass as a working Assistant.
if ($made) {
    Write-Host '=== the Assistant with the runtime hidden'
    $none = Join-Path $root 'no-runtime'
    New-Item -ItemType Directory -Force $none | Out-Null
    $hiddenPassed = $false
    try {
        Launch 47952 @{ WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $none }
        $until = [datetime]::UtcNow.AddSeconds(60)
        ClickTarget 'titlebar:assistant' $until
        do {
            $state = ReadAssistant (Call 'ui.inspect' @{} $until)
            if ($state.surface_ready) { throw 'hidden runtime still produced a ready page' }
            if ($state.open -and $state.host -ceq 'failed') { break }
            Start-Sleep -Milliseconds 200
        } while ([datetime]::UtcNow -lt $until)
        if (-not $state.open -or $state.host -cne 'failed') { throw 'hidden runtime did not report an open panel with a failed host' }
        Shot $h '05-runtime-hidden'
        [void](Call 'ui.screenshot' @{ path = (Join-Path $Evidence '05-app-frame-runtime-hidden.png') })
        $left = Views $h 'runtime hidden' -Any
        if ($left) { throw "hidden runtime left a visible child over the frame: $(Name $left.Window)" }
        Quit
        $hiddenPassed = $true
    } catch {
        Write-Host "FAIL: with the runtime hidden: $($_.Exception.Message)"
        $failures.Add("with the runtime hidden: " + $_.Exception.Message)
    } finally {
        $said = Finish 'runtime-hidden'
    }
    if ($hiddenPassed) {
        RequireCreationDiagnostic $said
        Write-Host 'ok: the hidden runtime produced a native creation diagnostic and no blank child'
    }
} else {
    Write-Host 'NOT RUN: hidden runtime (the installed-runtime checks did not reach native exit)'
}

$reports = @(Get-ChildItem -Recurse -Filter report.json $crashes -ErrorAction SilentlyContinue)
foreach ($r in $reports) { Get-Content $r.FullName | Write-Host }
if ($reports.Count -gt 0) { $failures.Add('a crash report was written') }
if ($failures.Count -gt 0) { Write-Error ("assistant web view: " + ($failures -join '; ')) }
Write-Host 'PASS: assistant web view'
