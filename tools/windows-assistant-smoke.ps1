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
    public class QueueState {
        public bool Succeeded { get; set; }
        public IntPtr hwndFocus { get; set; }
        public IntPtr hwndCapture { get; set; }
    }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT {
        public int X, Y; public uint Data, Flags, Time; public UIntPtr Extra;
    }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT {
        public ushort Key, Scan; public uint Flags, Time; public UIntPtr Extra;
    }
    [StructLayout(LayoutKind.Explicit)] struct INPUTDATA {
        [FieldOffset(0)] public MOUSEINPUT Mouse;
        [FieldOffset(0)] public KEYBDINPUT Key;
    }
    [StructLayout(LayoutKind.Sequential)] struct INPUT {
        public uint Type; public INPUTDATA Data;
    }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] static extern uint SendInput(uint count, INPUT[] input, int size);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
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
    public static QueueState InputQueue() {
        var info = new GUITHREADINFO();
        info.cbSize = Marshal.SizeOf(typeof(GUITHREADINFO));
        bool ok = GetGUIThreadInfo(0, ref info);
        return new QueueState { Succeeded = ok, hwndFocus = info.hwndFocus, hwndCapture = info.hwndCapture };
    }
    static INPUT Mouse(uint flags) {
        return new INPUT { Type = 0, Data = new INPUTDATA { Mouse = new MOUSEINPUT { Flags = flags } } };
    }
    static INPUT Key(ushort key, uint flags) {
        return new INPUT { Type = 1, Data = new INPUTDATA { Key = new KEYBDINPUT { Key = key, Flags = flags } } };
    }
    public static uint Click() {
        return SendInput(2, new[] { Mouse(0x0002), Mouse(0x0004) }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static bool ReleaseMouse() {
        return SendInput(1, new[] { Mouse(0x0004) }, Marshal.SizeOf(typeof(INPUT))) == 1;
    }
    public static bool TypeKey(ushort key) {
        uint count = SendInput(2, new[] { Key(key, 0), Key(key, 0x0002) }, Marshal.SizeOf(typeof(INPUT)));
        if (count == 1 && SendInput(1, new[] { Key(key, 0x0002) }, Marshal.SizeOf(typeof(INPUT))) != 1)
            throw new InvalidOperationException("partial key press; key release could not be queued");
        return count == 2;
    }
    public static IntPtr At(int x, int y) { POINT p; p.X = x; p.Y = y; return WindowFromPoint(p); }
}

// An unnamed nested job, with no breakaway flags and kill-on-last-handle-close.
public static class AssistantJob {
    [StructLayout(LayoutKind.Sequential)] struct Limits {
        public long ProcessTime, JobTime;
        public uint Flags;
        public UIntPtr MinimumWorkingSet, MaximumWorkingSet;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint Priority, Scheduling;
    }
    [StructLayout(LayoutKind.Sequential)] struct ExtendedLimits {
        public Limits Basic;
        public ulong ReadOps, WriteOps, OtherOps, ReadBytes, WriteBytes, OtherBytes;
        public UIntPtr ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory;
    }
    [StructLayout(LayoutKind.Sequential)] struct Accounting {
        public long UserTime, KernelTime, PeriodUserTime, PeriodKernelTime;
        public uint Faults, Total, Active, Terminated;
    }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern Microsoft.Win32.SafeHandles.SafeFileHandle CreateJobObject(IntPtr attrs, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetInformationJobObject(Microsoft.Win32.SafeHandles.SafeFileHandle job, int kind, ref ExtendedLimits info, int size);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool QueryInformationJobObject(Microsoft.Win32.SafeHandles.SafeFileHandle job, int kind, out Accounting info, int size, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AssignProcessToJobObject(Microsoft.Win32.SafeHandles.SafeFileHandle job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool IsProcessInJob(IntPtr process, Microsoft.Win32.SafeHandles.SafeFileHandle job, out bool member);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateJobObject(Microsoft.Win32.SafeHandles.SafeFileHandle job, uint code);
    static Exception Error(string call) {
        return new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), call + " failed");
    }
    public static Microsoft.Win32.SafeHandles.SafeFileHandle Create() {
        var job = CreateJobObject(IntPtr.Zero, null);
        if (job.IsInvalid) throw Error("CreateJobObject");
        var limits = new ExtendedLimits { Basic = new Limits { Flags = 0x2000 } }; // KILL_ON_JOB_CLOSE
        if (SetInformationJobObject(job, 9, ref limits, Marshal.SizeOf(typeof(ExtendedLimits)))) return job;
        var error = Error("SetInformationJobObject");
        job.Dispose();
        throw error;
    }
    public static void Assign(Microsoft.Win32.SafeHandles.SafeFileHandle job, IntPtr process) {
        if (!AssignProcessToJobObject(job, process)) throw Error("AssignProcessToJobObject");
    }
    public static bool Contains(Microsoft.Win32.SafeHandles.SafeFileHandle job, IntPtr process) {
        bool member;
        if (!IsProcessInJob(process, job, out member)) throw Error("IsProcessInJob");
        return member;
    }
    public static uint Active(Microsoft.Win32.SafeHandles.SafeFileHandle job) {
        Accounting info;
        if (!QueryInformationJobObject(job, 1, out info, Marshal.SizeOf(typeof(Accounting)), IntPtr.Zero))
            throw Error("QueryInformationJobObject");
        return info.Active;
    }
    public static void Terminate(Microsoft.Win32.SafeHandles.SafeFileHandle job) {
        if (!TerminateJobObject(job, 1)) throw Error("TerminateJobObject");
    }
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

function IsOwnedFrame {
    $script:p -and -not $script:p.HasExited -and [AssistantWin]::IsWindow($h) -and
        [AssistantWin]::Owner($h) -eq $script:p.Id
}

function WithinWindow([IntPtr]$parent, [IntPtr]$candidate) {
    $candidate -ne [IntPtr]::Zero -and [AssistantWin]::IsWindow($candidate) -and
        ($candidate -eq $parent -or [AssistantWin]::IsChild($parent, $candidate))
}

function ReadInputQueue { [AssistantWin]::InputQueue() }
function SendMouseClick { [AssistantWin]::Click() }
function ReleaseMouseButton { [AssistantWin]::ReleaseMouse() }
function MovePointer([int]$x, [int]$y) {
    if (-not [AssistantWin]::SetCursorPos($x, $y)) { throw 'BLOCKED: cannot move the desktop pointer' }
}

function ReadClickRoute([int]$x, [int]$y, [IntPtr]$target) {
    $pt = New-Object AssistantWin+POINT
    @{
        CursorAtPoint = [AssistantWin]::GetCursorPos([ref]$pt) -and $pt.X -eq $x -and $pt.Y -eq $y
        HitInTarget = WithinWindow $target ([AssistantWin]::At($x, $y))
        Queue = ReadInputQueue
        LeftDown = ([AssistantWin]::GetAsyncKeyState(1) -band 0x8000) -ne 0
        TargetInFrame = WithinWindow $h $target
        Foreground = [AssistantWin]::GetForegroundWindow() -eq $h
        FrameOwned = IsOwnedFrame
    }
}

# Validate after pointer movement. Queue down/up together so there is no sleep with a button held.
function Click([int]$x, [int]$y, [IntPtr]$target) {
    MovePointer $x $y
    Start-Sleep -Milliseconds 250
    $route = ReadClickRoute $x $y $target
    if (-not $route.FrameOwned) { throw 'cannot direct input to a live owned frame' }
    if (-not $route.TargetInFrame) { throw 'click target is no longer in the owned frame' }
    if (-not $route.CursorAtPoint -or -not $route.Foreground) {
        throw 'BLOCKED: cannot direct desktop input to the smoke window'
    }
    if (-not $route.Queue.Succeeded) { throw 'BLOCKED: GetGUIThreadInfo could not check mouse capture' }
    if ($route.Queue.hwndCapture -ne [IntPtr]::Zero) { throw 'BLOCKED: mouse capture can redirect the click' }
    if ($route.LeftDown) { throw 'BLOCKED: the left mouse button is already down' }
    if (-not $route.HitInTarget) { throw "click at $x,$y would not hit the target or its descendant" }
    $sent = SendMouseClick
    if ($sent -eq 1) {
        if (-not (ReleaseMouseButton)) { throw 'partial mouse click; left mouse button release could not be queued' }
        throw 'partial mouse click; left mouse button was released'
    }
    if ($sent -ne 2) { throw 'BLOCKED: SendInput did not accept the mouse click' }
    Start-Sleep -Milliseconds 700
}

# Windows moves the keyboard in its own messages, and a busy frame can delay that past the click's
# pause. Poll for the window the check expects; a real failure still ends on the wrong window or
# none at all within the limit.
function WaitFocus([scriptblock]$ok, [int]$seconds = 5) {
    $until = (Get-Date).AddSeconds($seconds)
    do {
        $queue = ReadInputQueue
        if (-not $queue.Succeeded) { throw 'BLOCKED: GetGUIThreadInfo could not read keyboard focus' }
        $focus = $queue.hwndFocus
        if (& $ok $focus) { return $focus }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $until)
    throw "unexpected keyboard focus (HWND: $focus)"
}

# Lower-case letters and spaces, as key presses.
function Keys([string]$text, [IntPtr]$target) {
    foreach ($c in $text.ToCharArray()) {
        $key = if ($c -eq ' ') { 0x20 } else { [byte][char]::ToUpperInvariant($c) }
        $queue = ReadInputQueue
        if (-not $queue.Succeeded) { throw 'BLOCKED: GetGUIThreadInfo could not verify typing focus' }
        if (-not (IsOwnedFrame) -or [AssistantWin]::GetForegroundWindow() -ne $h) {
            throw 'BLOCKED: the live smoke frame lost foreground before typing'
        }
        if (-not (WithinWindow $h $target) -or -not (WithinWindow $target $queue.hwndFocus) -or
            ($target -eq $h -and $queue.hwndFocus -ne $h)) { throw 'unexpected keyboard focus before typing' }
        if (-not [AssistantWin]::TypeKey($key)) { throw 'BLOCKED: SendInput did not accept a complete key press' }
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
function AssignOwnedLauncher($job, $launcher) { [AssistantJob]::Assign($job, $launcher.Handle) }

function AuthorizeOwnedLaunch($job, $launcher) {
    try {
        try { AssignOwnedLauncher $job $launcher }
        catch { throw "BLOCKED: cannot nest a run-owned job under this runner's job policy: $($_.Exception.Message)" }
        $launcher.StandardInput.WriteLine('launch')
    } finally { $launcher.StandardInput.Dispose() } # On assignment failure, EOF means do not launch.
}

function StartOwnedApp($psi, [int]$port) {
    # The launcher waits on a private pipe. It cannot start the app until assigned to the job.
    # This avoids the race between Process.Start and job assignment without breaking away from CI.
    $launcherCode = {
        $ErrorActionPreference = 'Stop'
        if ([Console]::ReadLine() -cne 'launch') { exit 0 }
        $start = [Diagnostics.ProcessStartInfo]::new($env:ASSISTANT_SMOKE_EXE)
        foreach ($arg in @('--sample', '--control', $env:ASSISTANT_SMOKE_PORT)) { [void]$start.ArgumentList.Add($arg) }
        $start.UseShellExecute = $false
        $start.RedirectStandardOutput = $start.RedirectStandardError = $true
        $app = [Diagnostics.Process]::Start($start)
        [Console]::WriteLine($app.Id)
        [Console]::Out.Flush()
        $outCopy = $app.StandardOutput.BaseStream.CopyToAsync([Console]::OpenStandardOutput())
        $errCopy = $app.StandardError.BaseStream.CopyToAsync([Console]::OpenStandardError())
        $app.WaitForExit()
        [Threading.Tasks.Task]::WaitAll(@($outCopy, $errCopy))
        exit $app.ExitCode
    }
    $psi.FileName = Join-Path $PSHOME 'pwsh.exe'
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardInput = $true
    $psi.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
    $psi.Environment['ASSISTANT_SMOKE_EXE'] = $exePath
    $psi.Environment['ASSISTANT_SMOKE_PORT'] = "$port"
    foreach ($arg in @('-NoProfile', '-NonInteractive', '-EncodedCommand',
            [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($launcherCode.ToString())))) {
        [void]$psi.ArgumentList.Add($arg)
    }
    try { $script:job = [AssistantJob]::Create() }
    catch { throw "BLOCKED: cannot create the run-owned Windows job: $($_.Exception.Message)" }
    $script:launcher = [Diagnostics.Process]::Start($psi)
    $script:stderr = $script:launcher.StandardError.ReadToEndAsync()
    AuthorizeOwnedLaunch $script:job $script:launcher
    $pidLine = $script:launcher.StandardOutput.ReadLineAsync()
    if (-not $pidLine.Wait(15000)) { throw 'owned launcher did not report the app process within 15 s' }
    $script:stdout = $script:launcher.StandardOutput.ReadToEndAsync()
    $appId = 0
    if (-not [int]::TryParse($pidLine.Result, [ref]$appId)) { throw 'owned launcher did not start the app' }
    $appProcess = [Diagnostics.Process]::GetProcessById($appId)
    try {
        if (-not [AssistantJob]::Contains($script:job, $appProcess.Handle)) { throw 'app process is not in the run-owned job' }
    } catch {
        $appProcess.Dispose()
        throw
    }
    $script:p = $appProcess
}

function Launch([int]$port, [hashtable]$extra) {
    $profile = Join-Path $root "$port"
    $script:settings = Join-Path $profile 'settings'
    $local = Join-Path $profile 'local'
    $roaming = Join-Path $profile 'roaming'
    $control = Join-Path $profile 'control'
    New-Item -ItemType Directory -Force $settings, $local, $roaming, $control, $crashes | Out-Null
    $script:token = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant()
    $psi = New-Object System.Diagnostics.ProcessStartInfo
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
    $script:client = $script:stderr = $script:stdout = $script:p = $script:launcher = $script:job = $null
    StartOwnedApp $psi $port
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

function ScreenRect($result, [string]$name, $client) {
    $entry = $result.rects | Where-Object { $_.name -ceq $name } | Select-Object -Last 1
    if (-not $entry) { return }
    foreach ($value in @($entry.x, $entry.y, $entry.width, $entry.height, $result.pixelsPerPoint)) {
        # JSON numbers are integers or floating-point values, never coerced strings or booleans.
        if (($value -isnot [int] -and $value -isnot [long] -and $value -isnot [double]) -or
            -not [double]::IsFinite($value)) {
            throw "invalid geometry for ${name}: expected finite numeric coordinates, size and scale"
        }
    }
    $x = [double]$entry.x; $y = [double]$entry.y
    $width = [double]$entry.width; $height = [double]$entry.height
    $scale = [double]$result.pixelsPerPoint
    if ($x -lt 0 -or $y -lt 0 -or $width -le 0 -or $height -le 0 -or $scale -le 0) {
        throw "invalid geometry for ${name}: negative origin or nonpositive size or scale"
    }
    # Keep arithmetic in double until all rounded edges are proven representable and in bounds.
    $left = [Math]::Round($x * $scale)
    $top = [Math]::Round($y * $scale)
    $right = [Math]::Round(($x + $width) * $scale)
    $bottom = [Math]::Round(($y + $height) * $scale)
    $clientWidth = [double]$client.Right - $client.Left
    $clientHeight = [double]$client.Bottom - $client.Top
    if (-not [double]::IsFinite($right) -or -not [double]::IsFinite($bottom) -or
        $right -le $left -or $bottom -le $top -or
        $right -gt [Math]::Min($clientWidth, [int]::MaxValue) -or
        $bottom -gt [Math]::Min($clientHeight, [int]::MaxValue)) {
        throw "invalid geometry for ${name}: rounded rectangle is empty, overflows or is outside the client"
    }
    @{
        Left   = [int]($client.Left + $left)
        Top    = [int]($client.Top + $top)
        Right  = [int]($client.Left + $right)
        Bottom = [int]($client.Top + $bottom)
    }
}

function ClientBounds {
    $bounds = New-Object AssistantWin+RECT
    if (-not (IsOwnedFrame) -or -not [AssistantWin]::GetClientRect($h, [ref]$bounds)) {
        throw 'cannot read the live owned frame client bounds'
    }
    $origin = New-Object AssistantWin+POINT; $far = New-Object AssistantWin+POINT
    $origin.X = $bounds.Left; $origin.Y = $bounds.Top
    $far.X = $bounds.Right; $far.Y = $bounds.Bottom
    if (-not [AssistantWin]::ClientToScreen($h, [ref]$origin) -or
        -not [AssistantWin]::ClientToScreen($h, [ref]$far) -or -not (IsOwnedFrame) -or
        $far.X -le $origin.X -or $far.Y -le $origin.Y) {
        throw 'cannot translate positive client bounds for the live owned frame'
    }
    @{ Left = $origin.X; Top = $origin.Y; Right = $far.X; Bottom = $far.Y }
}

function TargetRect([string[]]$names, [datetime]$until) {
    $params = if ($names.Count -eq 1) { @{ prefix = $names[0] } } else { @{} }
    $result = Call 'ui.rects' $params $until
    $client = ClientBounds
    foreach ($name in $names) {
        $rect = ScreenRect $result $name $client
        if ($rect) { return $rect }
    }
}

function ClickTarget([string[]]$names, [datetime]$until) {
    do {
        $r = TargetRect $names $until
        if ($r) {
            # Right and bottom are exclusive, including for a one-pixel target.
            Click ([int][Math]::Floor(($r.Left + $r.Right) / 2)) ([int][Math]::Floor(($r.Top + $r.Bottom) / 2)) $h
            return
        }
        Start-Sleep -Milliseconds 150
    } while ([datetime]::UtcNow -lt $until)
    throw "native UI target $($names -join ' or ') was not drawn"
}

function ClickAssistantEntry([datetime]$until) {
    # The design workspace uses the rail. If it has no rail, use the title bar from the same frame.
    ClickTarget @('rail:assistant', 'titlebar:assistant') $until
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

function PageClosePoint($page, $candidates, $query = $null) {
    # Count each nonempty candidate by its first failing check, in the original filter order.
    $counts = [ordered]@{ disabled = 0; offscreen = 0; nonpositive = 0; out_of_child = 0; valid = 0 }
    $valid = @(
        foreach ($candidate in $candidates) {
            if ($candidate.Enabled -ne $true) { $counts.disabled++ }
            elseif ($candidate.Offscreen -ne $false) { $counts.offscreen++ }
            elseif (-not ($candidate.Right -gt $candidate.Left -and $candidate.Bottom -gt $candidate.Top)) { $counts.nonpositive++ }
            elseif (-not ($candidate.Left -ge $page.Left -and $candidate.Top -ge $page.Top -and
                    $candidate.Right -le $page.Right -and $candidate.Bottom -le $page.Bottom)) { $counts.out_of_child++ }
            else { $counts.valid++; $candidate }
        }
    )
    if ($valid.Count -ne 1) {
        if ($null -ne $query) {
            # Saturate only the printed counters. Check every candidate for uniqueness.
            $summary = [ordered]@{ matched = [int]$query.matched; empty = [int]$query.empty }
            foreach ($key in $counts.Keys) { $summary[$key] = $counts[$key] }
            $fields = foreach ($key in $summary.Keys) {
                $count = if ($summary[$key] -gt 64) { '>64' } else { $summary[$key] }
                "$key=$count"
            }
            Write-Host "page Close UIA: $($fields -join ' '); webview=$($page.Left),$($page.Top)-$($page.Right),$($page.Bottom)"
        }
        throw 'no unique clickable page Close inside the live WebView'
    }
    @{
        X = [int][Math]::Floor(($valid[0].Left + $valid[0].Right) / 2)
        Y = [int][Math]::Floor(($valid[0].Top + $valid[0].Bottom) / 2)
    }
}

function PageCloseUiaPredicate {
    {
        $name = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Close')
        $button = [Windows.Automation.PropertyCondition]::new(
            [Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Button)
        $condition = [Windows.Automation.AndCondition]::new($name, $button)
    }.ToString()
}

function FindPageClose($view) {
    # Use Windows' inbox UIA assemblies via Windows PowerShell (.NET Framework).
    # This reader sends no input. A separate process bounds even a stalled UIA provider.
    $scriptText = '$child = [IntPtr]' + $view.Window.ToInt64() + "`n" + {
        $ErrorActionPreference = 'Stop'
        try { Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase }
        catch { [Console]::Error.WriteLine($_.Exception.Message); exit 2 }
    }.ToString() + (PageCloseUiaPredicate) + {
        $element = [Windows.Automation.AutomationElement]::FromHandle($child)
        if ($null -eq $element) { throw 'the live WebView is not exposed to UI Automation' }
        # The handle the caller passes is the WebView's inner render child, whose
        # descendants hold no page controls; the page's Close lives on an ancestor
        # root (the diagnostic sees exactly one button named Close there). Walk up
        # until a root answers, then require exactly one valid candidate as before.
        $matches = $element.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)
        if ($matches.Count -eq 0) {
            $walker = [Windows.Automation.TreeWalker]::ControlViewWalker
            $root = $walker.GetParent($element)
            for ($depth = 0; $depth -lt 5 -and $null -ne $root -and $matches.Count -eq 0; $depth++) {
                $matches = $root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)
                if ($matches.Count -eq 0) { $root = $walker.GetParent($root) }
            }
        }
        $found = @($matches | ForEach-Object {
            $current = $_.Current; $rect = $current.BoundingRectangle
            if (-not $rect.IsEmpty) {
                @{ Left = $rect.Left; Top = $rect.Top; Right = $rect.Right; Bottom = $rect.Bottom
                   Enabled = $current.IsEnabled; Offscreen = $current.IsOffscreen }
            }
        })
        # Extra reader data is counts only; the candidate payload and selection stay untruncated.
        ConvertTo-Json -InputObject @{
            matched = [Math]::Min(65, $matches.Count)
            empty = [Math]::Min(65, ($matches.Count - $found.Count))
            candidates = $found
        } -Compress
    }.ToString()
    InvokePageCloseUiaReader $scriptText
}

function StartPageCloseUiaReader($info) { [Diagnostics.Process]::Start($info) }

function PageCloseReaderRemaining($clock, [int]$limit) {
    [int][Math]::Max(0, ($limit - $clock.ElapsedMilliseconds))
}

function NewPageCloseReaderJob { [AssistantJob]::Create() }
function TestPageCloseReaderJobMember($job, $reader) { [AssistantJob]::Contains($job, $reader.Handle) }
function ReadPageCloseReaderJobActive($job) { [AssistantJob]::Active($job) }

function InvokeContainedPageCloseUiaReader($info, $clock) {
    # Use a separate no-breakaway, kill-on-close job. Do not touch the app's job.
    if ((PageCloseReaderRemaining $clock 13000) -eq 0) { throw 'comparison deadline expired' }
    $readerJob = NewPageCloseReaderJob
    $reader = $null
    $verified = $false
    try {
        if ((PageCloseReaderRemaining $clock 13000) -eq 0) { throw 'comparison deadline expired' }
        try { $reader = StartPageCloseUiaReader $info }
        catch { throw 'comparison reader cannot start' }
        try {
            AssignOwnedLauncher $readerJob $reader
            $verified = TestPageCloseReaderJobMember $readerJob $reader
            if (-not $verified) { throw 'comparison reader job membership not verified' }
            if ((PageCloseReaderRemaining $clock 13000) -eq 0) { throw 'comparison deadline expired' }
            $reader.StandardInput.WriteLine('compare')
        } finally { $reader.StandardInput.Dispose() } # EOF instead of admission on failure.

        $output = $reader.StandardOutput.ReadToEndAsync()
        $errorOutput = $reader.StandardError.ReadToEndAsync()
        if (-not $reader.WaitForExit((PageCloseReaderRemaining $clock 13000)) -or
            -not $errorOutput.Wait((PageCloseReaderRemaining $clock 13000)) -or
            -not $output.Wait((PageCloseReaderRemaining $clock 13000)) -or
            (PageCloseReaderRemaining $clock 13000) -eq 0) {
            throw 'comparison deadline expired'
        }
        if ($reader.ExitCode -ne 0) { throw 'comparison reader failed' }
        if ((ReadPageCloseReaderJobActive $readerJob) -ne 0) { throw 'comparison job did not empty' }
        return $output.Result -creplace '\r?\n\z', ''
    } finally {
        try {
            # Only a reader that never got admission could be outside the job.
            if ($reader -and -not $verified -and -not $reader.HasExited) {
                $reader.Kill()
                if (-not $reader.WaitForExit((PageCloseReaderRemaining $clock 15000))) { throw 'unadmitted reader still active' }
            }
            if ((ReadPageCloseReaderJobActive $readerJob) -ne 0) {
                StopOwnedJob $readerJob
                while ((ReadPageCloseReaderJobActive $readerJob) -ne 0) {
                    $remaining = PageCloseReaderRemaining $clock 15000
                    if ($remaining -eq 0) { throw 'comparison job still active' }
                    Start-Sleep -Milliseconds ([Math]::Min(50, $remaining))
                }
            }
        } catch { throw 'comparison reader cleanup failed' }
        finally {
            try { if ($reader) { $reader.Dispose() } }
            finally { CloseOwnedJob $readerJob }
        }
    }
}

function InvokePageCloseUiaReader([string]$scriptText, $clock = $null) {
    $info = [Diagnostics.ProcessStartInfo]::new((Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'))
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    if ($clock) {
        $info.RedirectStandardInput = $true
        $info.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
        $scriptText = 'if ([Console]::ReadLine() -cne ''compare'') { exit 0 }' + "`n" + $scriptText
    }
    foreach ($arg in @('-NoProfile', '-NonInteractive', '-Mta', '-EncodedCommand',
            [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($scriptText)))) {
        [void]$info.ArgumentList.Add($arg)
    }
    # The diagnostic alone uses a single 15 s budget, with the last 2 s reserved for termination.
    if ($clock) { return (InvokeContainedPageCloseUiaReader $info $clock) }
    try { $uia = StartPageCloseUiaReader $info }
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
        try {
            if (-not $uia.HasExited) {
                $uia.Kill()
                if (-not $uia.WaitForExit(3000)) { throw 'owned UI Automation query did not exit after termination' }
            }
        } catch { RecordCleanupFailure 'page Close UI Automation' $_.Exception.Message }
        $uia.Dispose()
    }
}

function ReadPageCloseWindow($native, [IntPtr]$window) {
    if ($window -eq [IntPtr]::Zero -or -not $native::IsWindow($window)) { return }
    $owner = [uint32]0
    $thread = $native::GetWindowThreadProcessId($window, [ref]$owner)
    $class = New-Object Text.StringBuilder 256
    if (-not $owner -or -not $thread -or -not $native::GetClassNameW($window, $class, $class.Capacity)) { return }
    @{
        Owner = $owner; Thread = $thread; Class = $class.ToString()
        Parent = $native::GetAncestor($window, 1) # GA_PARENT, not an owned top-level window's owner
        Visible = $native::IsWindowVisible($window)
    }
}

function FindVisiblePageCloseWidgets($native, [IntPtr]$wrapper) {
    $native::VisibleWidgets($wrapper)
}

function GetPageCloseNativeRoute($native, [IntPtr]$from, [IntPtr]$to, $first) {
    # Return a comparable HWND/PID/TID chain, not UIA data. Refuse a broken or cyclic chain.
    $route = @()
    $seen = [Collections.Generic.HashSet[IntPtr]]::new()
    for ($at = $from; $at -ne $to; $at = $entry.Parent) {
        if (-not $seen.Add($at)) { return }
        $entry = if ($at -eq $from) { $first } else { ReadPageCloseWindow $native $at }
        if (-not $entry -or -not $entry.Visible) { return }
        $route += "$($at.ToInt64()):$($entry.Owner):$($entry.Thread)"
    }
    if ($route.Count) { $route -join '/' }
}

function TestPageCloseFrameFocus($native, [IntPtr]$frame) {
    if ($frame -eq [IntPtr]::Zero -or $native::GetForegroundWindow() -ne $frame) { return $false }
    # InputQueue reads the foreground thread's queue. Reject a foreground change across that read.
    $queue = $native::InputQueue()
    $queue.Succeeded -eq $true -and $queue.hwndFocus -eq $frame -and $native::GetForegroundWindow() -eq $frame
}

function GetPageCloseRootIdentity($native, [IntPtr]$frame, [IntPtr]$wrapper, $owner, [long]$startTicks) {
    if ($owner.HasExited -or $owner.StartTime.ToUniversalTime().Ticks -ne $startTicks) { return 'stale' }
    $f = ReadPageCloseWindow $native $frame
    $w = ReadPageCloseWindow $native $wrapper
    if (-not $f.Visible -or $f.Owner -ne $owner.Id -or -not $w.Visible -or
        $w.Owner -ne $owner.Id -or $w.Class -cne 'WRY_WEBVIEW') { return 'stale' }

    $wrapperRoute = GetPageCloseNativeRoute $native $wrapper $frame $w
    if (-not $wrapperRoute) { return 'stale' }
    $widgets = @(FindVisiblePageCloseWidgets $native $wrapper)
    if ($widgets.Count -ne 1) { return 'unsupported' }
    $widget = $widgets[0]
    $c = ReadPageCloseWindow $native $widget
    if (-not $c.Visible -or $c.Class -cne 'Chrome_WidgetWin_1') { return 'stale' }
    $widgetRoute = GetPageCloseNativeRoute $native $widget $wrapper $c
    if (-not $widgetRoute -or $owner.HasExited -or -not (TestPageCloseFrameFocus $native $frame)) { return 'stale' }
    @{
        Widget = $widget
        Route = "$($frame.ToInt64()):$($f.Owner):$($f.Thread):$($f.Parent.ToInt64())/$wrapperRoute/$widgetRoute"
    }
}

function NewPageCloseCounts([int]$anyRole, [int]$button, [int]$documents, [bool]$rootIsDocument) {
    [ordered]@{
        close_name_any_role = [Math]::Min(65, $anyRole)
        close_name_button = [Math]::Min(65, $button)
        documents = [Math]::Min(65L, ([long]$documents + [int]$rootIsDocument))
    }
}

function ReadPageCloseCounts([IntPtr]$root) {
    . ([scriptblock]::Create((PageCloseUiaPredicate)))
    $element = [Windows.Automation.AutomationElement]::FromHandle($root)
    if ($null -eq $element) { throw 'UIA root unavailable' }
    $scope = [Windows.Automation.TreeScope]::Descendants
    $document = [Windows.Automation.PropertyCondition]::new(
        [Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Document)
    NewPageCloseCounts ($element.FindAll($scope, $name).Count) ($element.FindAll($scope, $condition).Count) `
        ($element.FindAll($scope, $document).Count) ($element.Current.ControlType -eq [Windows.Automation.ControlType]::Document)
}

function ComparePageCloseRoots($native, [IntPtr]$frame, [IntPtr]$wrapper, $owner, [long]$startTicks) {
    try {
        $before = GetPageCloseRootIdentity $native $frame $wrapper $owner $startTicks
        if ($before -is [string]) { return $before }
        $wrapperCounts = ReadPageCloseCounts $wrapper
        # The middle validation is both after the wrapper and before the widget.
        $middle = GetPageCloseRootIdentity $native $frame $wrapper $owner $startTicks
        if ($middle.Route -cne $before.Route) { return 'stale' }
        $widgetCounts = ReadPageCloseCounts $before.Widget
        $after = GetPageCloseRootIdentity $native $frame $wrapper $owner $startTicks
        if ($after.Route -cne $before.Route) { return 'stale' }
        $fields = foreach ($label in @('wrapper', 'widget')) {
            $counts = if ($label -ceq 'wrapper') { $wrapperCounts } else { $widgetCounts }
            foreach ($key in $counts.Keys) { "$label.$key=$($counts[$key])" }
        }
        "ok $($fields -join ' ')"
    } catch { 'error' }
}

function ReadPageCloseComparison($view) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    if (-not (IsOwnedFrame) -or -not (TestPageCloseFrameFocus ([AssistantWin]) $h)) { return 'stale' }
    $scriptText = "`$frame = [IntPtr]$($h.ToInt64()); `$wrapper = [IntPtr]$($view.Window.ToInt64()); " +
        "`$framePid = $($script:p.Id); `$startTicks = $($script:p.StartTime.ToUniversalTime().Ticks)L`n"
    foreach ($functionName in @('PageCloseUiaPredicate', 'ReadPageCloseWindow', 'FindVisiblePageCloseWidgets',
            'GetPageCloseNativeRoute', 'TestPageCloseFrameFocus', 'GetPageCloseRootIdentity',
            'NewPageCloseCounts', 'ReadPageCloseCounts', 'ComparePageCloseRoots')) {
        $scriptText += "function $functionName { $((Get-Command $functionName -CommandType Function).Definition) }`n"
    }
    $scriptText += {
        $ErrorActionPreference = 'Stop'
        $owner = $null
        try {
            try { $owner = [Diagnostics.Process]::GetProcessById($framePid) }
            catch [ArgumentException] { [Console]::WriteLine('stale'); exit }
            try { Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase }
            catch { [Console]::WriteLine('unsupported'); exit }
            Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class PageCloseDiagnosticWindows {
    [StructLayout(LayoutKind.Sequential)] struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct GUITHREADINFO {
        public int cbSize; public int flags;
        public IntPtr hwndActive, hwndFocus, hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret;
        public RECT rcCaret;
    }
    public struct QueueState { public bool Succeeded; public IntPtr hwndFocus; }
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread, ref GUITHREADINFO info);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h, EnumProc f, IntPtr l);
    public static QueueState InputQueue() {
        var info = new GUITHREADINFO { cbSize = Marshal.SizeOf(typeof(GUITHREADINFO)) };
        bool ok = GetGUIThreadInfo(0, ref info);
        return new QueueState { Succeeded = ok, hwndFocus = info.hwndFocus };
    }
    public static IntPtr[] VisibleWidgets(IntPtr wrapper) {
        var found = new List<IntPtr>();
        var name = new StringBuilder(256);
        EnumChildWindows(wrapper, (h, l) => {
            if (IsWindowVisible(h) && GetClassNameW(h, name, name.Capacity) != 0 &&
                    name.ToString() == "Chrome_WidgetWin_1") {
                found.Add(h);
            }
            return found.Count < 2;
        }, IntPtr.Zero);
        return found.ToArray();
    }
}
'@
            [Console]::WriteLine((ComparePageCloseRoots ([PageCloseDiagnosticWindows]) $frame $wrapper $owner $startTicks))
        } catch { [Console]::WriteLine('error') }
        finally { if ($owner) { $owner.Dispose() } }
    }.ToString()
    $receipt = InvokePageCloseUiaReader $scriptText $clock
    if (-not (IsOwnedFrame) -or -not (TestPageCloseFrameFocus ([AssistantWin]) $h)) { return 'stale' }
    if ((PageCloseReaderRemaining $clock 13000) -eq 0) { return 'error' }
    $receipt
}

function PageCloseComparisonReceipt($receipt) {
    if ($receipt -isnot [string] -or $receipt.Length -gt 256) { return 'error' }
    if ($receipt -cin @('unsupported', 'stale', 'error')) { return $receipt }
    $counter = '([1-5]?[0-9]|6[0-5])'
    $fields = foreach ($root in @('wrapper', 'widget')) {
        foreach ($key in @('close_name_any_role', 'close_name_button', 'documents')) {
            "$root\.$key=$counter"
        }
    }
    if ($receipt -cmatch ('\Aok ' + ($fields -join ' ') + '\z') -and
        [int]$Matches[1] -ge [int]$Matches[2] -and [int]$Matches[4] -ge [int]$Matches[5]) {
        return $receipt
    }
    'error'
}

function ClosePage($view) {
    $query = FindPageClose $view
    if ($query -is [Collections.IDictionary] -and $query.psbase.Keys.Count -eq 3 -and
        'matched' -cin $query.psbase.Keys -and 'empty' -cin $query.psbase.Keys -and 'candidates' -cin $query.psbase.Keys -and
        ($query.matched -is [long] -or $query.matched -is [int]) -and $query.matched -eq 0 -and
        ($query.empty -is [long] -or $query.empty -is [int]) -and $query.empty -eq 0 -and
        $query.candidates -is [array] -and $query.candidates.Count -eq 0) {
        # Diagnostic only: neither an alternate match nor a failed reader can affect the cold result.
        try { $receipt = PageCloseComparisonReceipt (ReadPageCloseComparison $view) }
        catch { $receipt = 'error' }
        try { Write-Host "page Close UIA roots: $receipt" } catch { } # Preserve the primary refusal even if logging fails.
    }
    if ($query.candidates -is [array] -and $query.candidates.Count -eq 0) {
        # At the bottom centre (#221) the page shows Collapse instead of Close and the rail's
        # Assistant entry is what closes it: follow the affordance the layout really offers.
        ClickAssistantEntry ([datetime]::UtcNow.AddSeconds(20))
    } else {
        $point = PageClosePoint $view.Rect $query.candidates $query
        Click $point.X $point.Y $view.Window
    }
    $until = [datetime]::UtcNow.AddSeconds(15)
    do {
        if ($script:p.HasExited) { throw 'app exited during page Close' }
        $state = ReadAssistant (Call 'ui.inspect' @{} $until)
        $visible = Views $h 'after page Close'
        $retained = (WithinWindow $h $view.Window) -and [AssistantWin]::Class($view.Window) -ceq 'WRY_WEBVIEW' -and
            -not [AssistantWin]::IsWindowVisible($view.Window)
        if (PageClosed $state $visible (IsOwnedFrame) $retained) { return }
        Start-Sleep -Milliseconds 200
    } while ([datetime]::UtcNow -lt $until)
    throw 'page Close did not leave a closed panel with the original hidden, surface-ready child'
}

function PageClosed($state, $visiblePage, [bool]$appAlive, [bool]$originalHiddenChild) {
    if (-not $appAlive) { throw 'app exited during page Close' }
    $state.open -eq $false -and $state.surface_ready -eq $true -and $state.host -ceq 'available' -and
        $null -eq $visiblePage -and $originalHiddenChild
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

function WaitJobEmpty($job, [int]$milliseconds) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        if ([AssistantJob]::Active($job) -eq 0) { return $true }
        if ($timer.ElapsedMilliseconds -ge $milliseconds) { return $false }
        Start-Sleep -Milliseconds 100
    } while ($true)
}

function StopOwnedJob($job) { [AssistantJob]::Terminate($job) }
function CloseOwnedJob($job) { $job.Dispose() }

function DrainRunPipe($task, [string]$which) {
    if ($task) {
        if (-not $task.Wait(3000)) { throw "$which pipe did not close after the run; a surviving process cannot be ruled out" }
        $task.Result
    }
}

function RecordCleanupFailure([string]$name, [string]$message) {
    $what = "${name} cleanup: $message"
    Write-Host "FAIL: $what"
    $script:failures.Add($what)
}

function RecordRunFailure([string]$message, [string]$context = '') {
    if ($message.StartsWith('BLOCKED:')) {
        $what = "BLOCKED: $context$($message.Substring(8).TrimStart())"
    } else {
        $what = "FAIL: $context$message"
    }
    Write-Host $what
    $script:failures.Add($what)
}

# Keep checking the job and both pipes even after the app or launcher exits.
# Do not let a cleanup error overwrite the original smoke diagnosis.
function Finish([string]$name) {
    $hadPrimaryFailure = $script:failures.Count -gt 0
    $forced = $false
    if ($script:client) {
        try { $script:client.Dispose() } catch { RecordCleanupFailure $name $_.Exception.Message }
        $script:client = $null
    }
    if ($script:job) {
        $empty = $false
        try {
            $grace = if ($script:p -and -not $script:p.HasExited) { 0 } else { 5000 }
            $empty = WaitJobEmpty $script:job $grace
        } catch { RecordCleanupFailure $name $_.Exception.Message }
        if (-not $empty) {
            $forced = $true
            $why = 'run-owned job did not empty on its own; forcing cleanup'
            if ($hadPrimaryFailure) { Write-Host "${name}: $why" } else { RecordCleanupFailure $name $why }
            try {
                StopOwnedJob $script:job
                if (-not (WaitJobEmpty $script:job 15000)) { throw 'run-owned job did not empty after termination' }
            } catch { RecordCleanupFailure $name $_.Exception.Message }
        }
        try { CloseOwnedJob $script:job } catch { RecordCleanupFailure $name $_.Exception.Message }
        $script:job = $null
    }
    if ($script:launcher) {
        try {
            # If nested assignment failed, the unassigned launcher was sent EOF, not "launch".
            if (-not $script:launcher.HasExited -and -not $script:launcher.WaitForExit(3000)) {
                $forced = $true
                $why = 'owned launcher did not exit on its own; forcing cleanup'
                if ($hadPrimaryFailure) { Write-Host "${name}: $why" } else { RecordCleanupFailure $name $why }
                $script:launcher.Kill()
                if (-not $script:launcher.WaitForExit(3000)) { throw 'owned launcher did not exit after termination' }
            }
        } catch { RecordCleanupFailure $name $_.Exception.Message }
    }
    $err = ''
    try { $err = DrainRunPipe $script:stderr 'stderr' } catch { RecordCleanupFailure $name $_.Exception.Message }
    try { [void](DrainRunPipe $script:stdout 'stdout') } catch { RecordCleanupFailure $name $_.Exception.Message }
    if ($script:launcher -and -not $hadPrimaryFailure -and -not $forced) {
        try {
            if ($script:launcher.ExitCode -ne 0) { throw "relay exited with code $($script:launcher.ExitCode)" }
        } catch { RecordCleanupFailure $name $_.Exception.Message }
    }
    try {
        Set-Content -Path (Join-Path $Evidence "$name-app-error-output.txt") -Value $err
    } catch { RecordCleanupFailure $name $_.Exception.Message }
    foreach ($process in @($script:p, $script:launcher)) {
        if ($process) {
            try { $process.Dispose() } catch { RecordCleanupFailure $name $_.Exception.Message }
        }
    }
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
    ClickAssistantEntry $until
    $view = WaitPage $until 'floating'
    Write-Host 'ok: the open Assistant reports current-view readiness and matches the visible native page'
    Shot $h '01-assistant-open'
    [void](Call 'ui.screenshot' @{ path = (Join-Path $Evidence '01-app-frame-assistant-open.png') })

    # Do not submit a chat. A click in the message box lets the page take the keyboard;
    # a click in the 3D view must give the keyboard back.
    $w = Rect $h; $v = $view.Rect
    $scale = [AssistantWin]::GetDpiForWindow($h) / 96.0
    if ($scale -le 0) { throw 'BLOCKED: cannot read the desktop DPI' }
    # The Assistant sits at the bottom centre of the window by default (agent_chat_centre): its
    # text lies 47 to 87 px above the view's foot, above the 41 px button row, so 67 px lands in
    # the message box. (Measured at 100 %; the page is drawn larger at a larger display scale.)
    Click ($v.Left + [int](90 * $scale)) ($v.Bottom - [int](67 * $scale)) $view.Window
    $focus = WaitFocus { param($f) WithinWindow $view.Window $f }
    Write-Host "ok: a click on the message box gives the page the keyboard ($(Name $focus))"
    Keys 'hello windows' $view.Window
    Shot $h '02-after-click-and-typing-on-page'
    $free = if (($v.Left - $w.Left) -ge ($w.Right - $v.Right)) { [int](($w.Left + $v.Left) / 2) } else { [int](($v.Right + $w.Right) / 2) }
    Click $free ([int](($w.Top + $w.Bottom) / 2)) $h
    $focus = WaitFocus { param($f) $f -eq $h }
    Write-Host "ok: a click on the 3D view gives the frame the keyboard back ($(Name $focus))"
    Shot $h '03-after-click-on-3d-view'

    ClosePage $view
    Write-Host 'ok: the page Close (or the Assistant entry at the bottom centre) hid the native child and the app is still running'
    Shot $h '03a-page-closed'
    $until = [datetime]::UtcNow.AddSeconds(30)
    ClickAssistantEntry $until
    $reopened = WaitPage $until 'floating'
    if ($reopened.Window -ne $view.Window) { throw 'reopening did not reuse the retained native child' }
    Shot $h '03b-page-reopened'

    # Switch layout through the real search result while the Assistant is open.
    $until = [datetime]::UtcNow.AddSeconds(30)
    ClickTarget 'ribbon:search' $until
    Keys 'workspace layout' $h
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
    RecordRunFailure $_.Exception.Message
} finally {
    [void](Finish 'first-run')
}

# The same app with no runtime to be found: the app must report that the view could not be made
# and leave no blank window over the panel. It must not quietly pass as a working Assistant.
if ($made -and $failures.Count -eq 0) {
    Write-Host '=== the Assistant with the runtime hidden'
    $none = Join-Path $root 'no-runtime'
    New-Item -ItemType Directory -Force $none | Out-Null
    $hiddenPassed = $false
    try {
        Launch 47952 @{ WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $none }
        $until = [datetime]::UtcNow.AddSeconds(60)
        ClickAssistantEntry $until
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
        RecordRunFailure $_.Exception.Message 'with the runtime hidden: '
    } finally {
        $said = Finish 'runtime-hidden'
    }
    if ($hiddenPassed -and $failures.Count -eq 0) {
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
