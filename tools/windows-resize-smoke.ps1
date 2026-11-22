# Start the Windows app and check its resize border (#1381): at first sight, before requesting
# a screenshot, Windows' hit test reports the sides, corners and client area. The resize border
# disappears on maximize and returns on restore. A real bottom-edge drag resizes the window
# when the desktop delivers mouse input. The app must exit cleanly on a native close request.
#
# What this can't check on a CI runner (one monitor at 100 %): moving the window between monitors
# with different scales, and the resize border's width at other scales (unit tests cover the sizes).
#
# Usage: tools/windows-resize-smoke.ps1 -Exe target\ci\intcad.exe
param([Parameter(Mandatory = $true)][string]$Exe)
$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Win {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll", ExactSpelling = true, SetLastError = true)]
    private static extern IntPtr SendMessageTimeoutW(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll", ExactSpelling = true)] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
    [DllImport("user32.dll")] public static extern int GetSystemMetricsForDpi(int i, uint dpi);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
    public static string Text(IntPtr h) { var s = new System.Text.StringBuilder(256); GetWindowTextW(h, s, 256); return s.ToString(); }
    public static string Class(IntPtr h) { var s = new System.Text.StringBuilder(256); GetClassNameW(h, s, 256); return s.ToString(); }
    public static bool IsOwnedWindow(IntPtr h, uint pid) {
        uint owner;
        return h != IntPtr.Zero && GetWindowThreadProcessId(h, out owner) != 0 && owner == pid && IsWindowVisible(h);
    }
    // The process's visible top-level Intcad window, not its other windows (such as the IME's).
    public static IntPtr MainWindow(uint pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            if (IsOwnedWindow(h, pid)) {
                var title = Text(h);
                if (title.StartsWith("Intcad") || title.StartsWith("SolidCraft")) { found = h; return false; }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static int HitTest(IntPtr h, int x, int y) {
        long l = ((long)(ushort)(short)y << 16) | (long)(ushort)(short)x;
        UIntPtr hit;
        // SMTO_BLOCK | SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT. The API status is not the hit code.
        if (SendMessageTimeoutW(h, 0x0084 /* WM_NCHITTEST */, IntPtr.Zero, new IntPtr(l), 0x23, 2000, out hit) == IntPtr.Zero) {
            int error = Marshal.GetLastWin32Error();
            throw new InvalidOperationException($"WM_NCHITTEST failed (Win32 error {error}) at ({x}, {y}) within the 2 s limit");
        }
        return unchecked((int)hit.ToUInt64());
    }
}
'@

$HT = @{ 1 = 'client'; 10 = 'left'; 11 = 'right'; 12 = 'top'; 13 = 'top-left'; 14 = 'top-right'; 15 = 'bottom'; 16 = 'bottom-left'; 17 = 'bottom-right' }
$failures = New-Object System.Collections.Generic.List[string]
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "ok: $what" } else { Write-Host "FAIL: $what"; $failures.Add($what) }
}
function Rect($h) {
    $r = New-Object Win+RECT
    if (-not [Win]::GetWindowRect($h, [ref]$r)) { throw 'could not read the window rectangle' }
    $r
}

function Assert-OwnedWindow($p, $h, [string]$when) {
    if ($p.HasExited -or -not [Win]::IsOwnedWindow($h, [uint32]$p.Id)) {
        throw "$when : no owned window for process $($p.Id) (exited: $($p.HasExited))"
    }
}

function Get-OwnedHit($p, $h, [int]$x, [int]$y, [string]$when) {
    Assert-OwnedWindow $p $h $when
    $hit = [Win]::HitTest($h, $x, $y)
    Assert-OwnedWindow $p $h $when
    $hit
}

function Wait-WindowState($p, $h, [bool]$zoomed, [string]$state) {
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    do {
        Assert-OwnedWindow $p $h "waiting for $state"
        if ([Win]::IsZoomed($h) -eq $zoomed -and -not [Win]::IsIconic($h)) {
            Write-Host "ok: window became $state"
            return
        }
        Start-Sleep -Milliseconds 100
    } while ($timer.ElapsedMilliseconds -lt 10000)
    throw "window did not become $state within 10 s"
}

# Every side and corner, `in` px inside the window's outer pixel, and points just past the border.
function CheckBorder($p, $h, [string]$when) {
    $r = Rect $h
    if ($when -eq 'first sight') {
        $settled = $false
        for ($i = 0; $i -lt 25; $i++) {
            Start-Sleep -Milliseconds 200
            $next = Rect $h
            if ($r.Equals($next)) { $settled = $true; break }
            $r = $next
        }
        if (-not $settled) { throw "$when : window placement did not settle within 5 s" }
    }
    for ($try = 1; $try -le 3; $try++) {
        $r = Rect $h
        $dpi = [Win]::GetDpiForWindow($h)
        # Windows' own resize border at this DPI (SM_CXSIZEFRAME + SM_CXPADDEDBORDER), as the app uses it.
        $b = [Win]::GetSystemMetricsForDpi(32, $dpi) + [Win]::GetSystemMetricsForDpi(92, $dpi)
        $cx = [int](($r.Left + $r.Right) / 2); $cy = [int](($r.Top + $r.Bottom) / 2)
        $l = $r.Left; $t = $r.Top; $rt = $r.Right - 1; $bt = $r.Bottom - 1
        Write-Host "$when : window $($r.Left),$($r.Top) - $($r.Right),$($r.Bottom) at $dpi DPI, border $b px"
        $cases = @(
            @($l, $cy, 10), @(($l + $b - 1), $cy, 10), @(($l + $b), $cy, 1),
            @($rt, $cy, 11), @(($rt - $b + 1), $cy, 11), @(($rt - $b), $cy, 1),
            @($cx, $t, 12), @($cx, ($t + $b - 1), 12), @($cx, ($t + $b), 1),
            @($cx, $bt, 15), @($cx, ($bt - $b + 1), 15), @($cx, ($bt - $b), 1),
            @($l, $t, 13), @($rt, $t, 14), @($l, $bt, 16), @($rt, $bt, 17),
            @(($l + 2 * $b - 1), $t, 13), @($rt, ($bt - 2 * $b + 1), 17),
            @($cx, $cy, 1)
        )
        # Record failures only after confirming these points used a stable window rect.
        $results = foreach ($c in $cases) {
            $got = Get-OwnedHit $p $h $c[0] $c[1] $when
            @{ ok = ($got -eq $c[2]); what = "$when hit test at ($($c[0]), $($c[1])): $($HT[$got]) ($got), want $($HT[$c[2]])" }
        }
        $after = Rect $h
        if (-not $r.Equals($after)) {
            if ($try -lt 3) {
                Write-Host "$when : window rect changed during hit tests; retry $try of 2"
                continue
            }
            Check $false "$when : window rect changed during all 3 hit-test passes"
            return
        }
        Check ($b -ge 1) "$when : a resize border of $b px"
        foreach ($result in $results) { Check $result.ok $result.what }
        return
    }
}

$port = 47901
$root = Join-Path $env:RUNNER_TEMP 'resize-smoke'
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
$settings = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'Intcad\data'
Remove-Item -Recurse -Force $settings -ErrorAction SilentlyContinue
$local = Join-Path $root 'local'
$crashes = Join-Path $root 'crashes'
New-Item -ItemType Directory -Force $settings, $local, $crashes | Out-Null
$token = -join ((1..64) | ForEach-Object { '{0:x}' -f (Get-Random -Maximum 16) })

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = (Resolve-Path $Exe).Path
foreach ($a in @('--sample', '--control', "$port")) { [void]$psi.ArgumentList.Add($a) }
$psi.UseShellExecute = $false
$psi.RedirectStandardError = $true
$psi.RedirectStandardOutput = $true
$psi.Environment['LOCALAPPDATA'] = $local
$psi.Environment['SOLIDCRAFT_CRASH_DIR'] = $crashes
$psi.Environment['SOLIDCRAFT_CONTROL_TOKEN'] = $token
$p = $null
$stderr = $null
$client = $null
$dragSkipped = $false

try {
    $p = [System.Diagnostics.Process]::Start($psi)
    $stderr = $p.StandardError.ReadToEndAsync()
    $stdout = $p.StandardOutput.ReadToEndAsync()
    # The window as soon as it shows (eframe creates it hidden and shows it after the first frame).
    $h = [IntPtr]::Zero
    $deadline = (Get-Date).AddSeconds(180)
    while ($h -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline -and -not $p.HasExited) {
        $h = [Win]::MainWindow([uint32]$p.Id)
        if ($h -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 20 }
    }
    if ($h -eq [IntPtr]::Zero) { throw "no window (exited: $($p.HasExited))" }
    Assert-OwnedWindow $p $h 'first sight'
    Write-Host "window: '$([Win]::Text($h))' class '$([Win]::Class($h))'"
    CheckBorder $p $h 'first sight'

    while (-not $client -and (Get-Date) -lt $deadline -and -not $p.HasExited) {
        try { $client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', $port) } catch { Start-Sleep -Milliseconds 500 }
    }
    if (-not $client) { throw "no control channel (exited: $($p.HasExited))" }
    $client.ReceiveTimeout = 120000
    $stream = $client.GetStream()
    $writer = New-Object System.IO.StreamWriter($stream)
    $writer.AutoFlush = $true
    $reader = New-Object System.IO.StreamReader($stream)
    $writer.WriteLine((@{ id = 1; token = $token; method = 'ui.screenshot'; params = @{ path = (Join-Path $root 'first.png') } } | ConvertTo-Json -Compress))
    Write-Host "first frame: $($reader.ReadLine())"
    $client.Close()
    $client = $null

    # A known place and size that fits the runner's screen (physical px; the minimum is 960 x 600).
    Assert-OwnedWindow $p $h 'placing the window'
    $sw = [Win]::GetSystemMetrics(0); $sh = [Win]::GetSystemMetrics(1)
    Write-Host "screen: $sw x $sh"
    if (-not [Win]::SetWindowPos($h, [IntPtr]::Zero, 20, 20, [Math]::Min(1000, $sw - 40), [Math]::Min(620, $sh - 140), 0x0014)) { # NOZORDER | NOACTIVATE
        throw 'could not place the window (SetWindowPos failed)'
    }
    Start-Sleep -Milliseconds 500
    CheckBorder $p $h 'restored'

    # A real drag on the bottom edge, if this session takes mouse input.
    Assert-OwnedWindow $p $h 'preparing the drag'
    $before = Rect $h
    $x = [int](($before.Left + $before.Right) / 2); $y = $before.Bottom - 2
    $tolerance = 4
    $grow = [Math]::Min(80, $sh - $y - 10)
    if ($grow -le $tolerance) { throw "insufficient bottom-edge drag travel: $grow px (need more than $tolerance px)" }
    [void][Win]::SetForegroundWindow($h)
    if (-not [Win]::SetCursorPos($x, $y)) {
        $code = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        if ($code -ne 5) { throw "could not position the cursor (Win32 error $code)" }
        # ERROR_ACCESS_DENIED independently proves cursor control is unavailable, before any drag.
        $dragSkipped = $true
        Write-Host 'skipped: the drag (SetCursorPos denied access before mouse input)'
    } else {
        $pt = New-Object Win+POINT
        if (-not [Win]::GetCursorPos([ref]$pt)) { throw 'could not read the drag start cursor position' }
        if ($pt.X -ne $x -or $pt.Y -ne $y) {
            throw "cursor did not reach the drag start: at $($pt.X),$($pt.Y), expected $x,$y"
        }
        # The cursor position alone does not prove the window receives mouse input.
        Start-Sleep -Milliseconds 300
        Assert-OwnedWindow $p $h 'starting the drag'
        [Win]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero) # LEFTDOWN
        try {
            for ($i = 1; $i -le 8; $i++) {
                Start-Sleep -Milliseconds 40
                if (-not [Win]::SetCursorPos($x, $y + [int]($grow * $i / 8))) { throw "could not move the cursor during drag (step $i)" }
                [Win]::mouse_event(0x0001, 0, 0, 0, [UIntPtr]::Zero) # MOVE (relative 0: the position above)
            }
            Start-Sleep -Milliseconds 100
        } finally {
            [Win]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) # LEFTUP
        }
        Start-Sleep -Milliseconds 500
        Assert-OwnedWindow $p $h 'checking the drag'
        $after = Rect $h
        $dh = ($after.Bottom - $after.Top) - ($before.Bottom - $before.Top)
        $ok = [Math]::Abs($dh - $grow) -le $tolerance -and $after.Top -eq $before.Top -and
            $after.Left -eq $before.Left -and $after.Right -eq $before.Right
        Check $ok "dragging the bottom edge down $grow px resizes the window (height change $dh; fixed edges $($before.Left),$($before.Top),$($before.Right) -> $($after.Left),$($after.Top),$($after.Right))"
    }

    # Post native system commands to the app's window thread. They don't need mouse input,
    # and posting keeps the state wait bounded if the thread doesn't respond.
    Assert-OwnedWindow $p $h 'maximizing'
    if ([Win]::IsZoomed($h) -or [Win]::IsIconic($h)) { throw 'window was not restored before maximize' }
    if (-not [Win]::PostMessageW($h, 0x0112, [IntPtr]0xF030, [IntPtr]::Zero)) { # WM_SYSCOMMAND / SC_MAXIMIZE
        throw 'could not post SC_MAXIMIZE'
    }
    Wait-WindowState $p $h $true 'maximized'
    # The maximized window's rect overhangs the screen by the hidden frame (Windows' own
    # frame answers there); the screen's left edge is where the pointer can be.
    $r = Rect $h
    $x = [Math]::Max($r.Left, 0) + 1
    $got = Get-OwnedHit $p $h $x ([int](($r.Top + $r.Bottom) / 2)) 'maximized'
    Check ($got -eq 1) "maximized ($($r.Left),$($r.Top) - $($r.Right),$($r.Bottom)), the screen's left edge is client area ($($HT[$got]))"

    Assert-OwnedWindow $p $h 'restoring'
    if (-not [Win]::IsZoomed($h)) { throw 'window was not maximized before restore' }
    if (-not [Win]::PostMessageW($h, 0x0112, [IntPtr]0xF120, [IntPtr]::Zero)) { # WM_SYSCOMMAND / SC_RESTORE
        throw 'could not post SC_RESTORE'
    }
    Wait-WindowState $p $h $false 'restored'
    CheckBorder $p $h 'restored again'

    Assert-OwnedWindow $p $h 'closing'
    if (-not [Win]::PostMessageW($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'could not post WM_CLOSE' }
    if (-not $p.WaitForExit(60000)) { throw 'did not exit after WM_CLOSE within 60 s' }
    if ($p.ExitCode -ne 0) { throw ('exit code 0x{0:x}' -f $p.ExitCode) }
    Write-Host 'ok: WM_CLOSE exited with code 0'
} catch {
    $failures.Add($_.Exception.Message)
} finally {
    try {
        if ($client) { $client.Close() }
    } catch {
        $failures.Add("control connection cleanup: $($_.Exception.Message)")
    }
    try {
        if ($p -and -not $p.HasExited) {
            $failures.Add('process was still running at cleanup')
            $p.Kill()
            if (-not $p.WaitForExit(5000)) { $failures.Add('process did not exit after cleanup kill within 5 s') }
        }
    } catch {
        $failures.Add("process cleanup: $($_.Exception.Message)")
    }
}
if ($stderr -and $stderr.IsCompletedSuccessfully) { Write-Host $stderr.Result }
if ($failures.Count -gt 0) { Write-Error ("resize border: " + ($failures -join '; ')) }
if ($dragSkipped) { Write-Host 'PASS: resize border (real-input drag skipped)' }
else { Write-Host 'PASS: resize border' }
