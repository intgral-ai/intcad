<#
.SYNOPSIS
  Detect, require or install the Microsoft Edge WebView2 Evergreen Runtime.

.DESCRIPTION
  The Assistant on Windows is a WebView2 page. There is no built-in fallback: a machine without
  the runtime cannot show the Assistant at all. This script makes that state explicit and gives
  the recovery path, instead of letting a package check or a user run fall through to a blank
  panel.

  Detection follows Microsoft's documented registry keys for the Evergreen Runtime: the pv value
  under HKEY_LOCAL_MACHINE (native and WOW6432Node) and HKEY_CURRENT_USER for
  {F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}. A pv of null, empty or 0.0.0.0 is not an install. The
  version folder is also looked for under Program Files, Program Files (x86) and LocalAppData so
  a present runtime whose executable is missing is reported rather than silently accepted.

  Installation has two deliberately different paths:

    Online   Microsoft's Evergreen Bootstrapper, downloaded from the documented link
             https://go.microsoft.com/fwlink/p/?LinkId=2124703. That link is stable; the file
             behind it is not, so there is no stable checksum to pin. Its Authenticode signature
             must be Valid and name Microsoft before it is run.

    Offline  The Evergreen Standalone Installer supplied by the caller, never downloaded from a
             URL made up here. Download it from
             https://developer.microsoft.com/microsoft-edge/webview2/ for the target
             architecture, pass its path and its SHA-256, and the file is checked before it runs.
             The latest standalone installer supports per-machine and per-user installation just
             like the bootstrapper: elevation decides which one happens.

  Both installers are run as `/silent /install`. An elevated process installs for the machine; an
  unelevated process installs for the current user. -Scope only checks that intent: PerMachine
  refuses to run unelevated, and PerUser refuses to run elevated because the installer would
  silently do a per-machine install instead.

  Every failure throws with an actionable message. There is no silent success and no swallowed
  error.

.PARAMETER Action
  Detect (default): report the runtime, exit 0 even when absent.
  Require: throw when absent.
  Install: detect, install when absent, then require it.
  Report: write the detection result and recovery instructions, then exit 0 even when absent.

.PARAMETER Mode
  For Install: Auto (offline when -OfflineInstaller is given, otherwise online), Online or Offline.

.PARAMETER Scope
  Auto (the elevation decides), PerMachine (requires elevation) or PerUser (requires no elevation).

.PARAMETER OfflineInstaller
  Path to the Evergreen Standalone Installer. Defaults to
  SOLIDCRAFT_WEBVIEW2_OFFLINE_INSTALLER. Download it from the official page above; this script
  never guesses a delivery URL.

.PARAMETER ExpectedSha256
  SHA-256 of the offline installer, or of a bootstrapper you supply yourself. Required for an
  offline install unless -AllowUnverifiedInstaller is given. Defaults to
  SOLIDCRAFT_WEBVIEW2_INSTALLER_SHA256.

.PARAMETER Evidence
  Optional directory. A webview2-runtime.json record is written there for CI artifacts.

.EXAMPLE
  pwsh packaging/windows/webview2-runtime.ps1 -Action Detect

.EXAMPLE
  pwsh packaging/windows/webview2-runtime.ps1 -Action Install -Mode Online -Evidence evidence

.EXAMPLE
  pwsh packaging/windows/webview2-runtime.ps1 -Action Install -Mode Offline `
    -OfflineInstaller C:\downloads\MicrosoftEdgeWebView2RuntimeInstallerX64.exe `
    -ExpectedSha256 <sha256 from the release owner>
#>
[CmdletBinding()]
param(
  [ValidateSet('Detect', 'Require', 'Install', 'Report')]
  [string] $Action = 'Detect',
  [ValidateSet('Auto', 'Online', 'Offline')]
  [string] $Mode = 'Auto',
  [ValidateSet('Auto', 'PerMachine', 'PerUser')]
  [string] $Scope = 'Auto',
  [string] $OfflineInstaller = $env:SOLIDCRAFT_WEBVIEW2_OFFLINE_INSTALLER,
  [string] $ExpectedSha256 = $env:SOLIDCRAFT_WEBVIEW2_INSTALLER_SHA256,
  [string] $Evidence = '',
  [int] $WaitSeconds = 120,
  [switch] $AllowUnverifiedInstaller,
  [switch] $Json
)
$ErrorActionPreference = 'Stop'

# Microsoft's documented Evergreen clients key. The pv value is the Runtime version.
$script:WebView2Client = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
# The documented Evergreen Bootstrapper link. It redirects to a current, changing file: there is
# no stable SHA-256 to pin, so its Microsoft Authenticode signature is checked instead.
$script:WebView2OnlineBootstrapper = 'https://go.microsoft.com/fwlink/p/?LinkId=2124703'
$script:WebView2DownloadPage = 'https://developer.microsoft.com/microsoft-edge/webview2/'

function Test-WebView2Version {
  param([string] $Version)
  if ([string]::IsNullOrWhiteSpace($Version)) { return $false }
  $parsed = $null
  if (-not [version]::TryParse($Version, [ref] $parsed)) { return $false }
  return $parsed -gt [version] '0.0.0.0'
}

function Find-WebView2Executable {
  param([string] $Version)
  $roots = @()
  if (${env:ProgramFiles(x86)}) { $roots += (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\EdgeWebView\Application') }
  if ($env:ProgramFiles) { $roots += (Join-Path $env:ProgramFiles 'Microsoft\EdgeWebView\Application') }
  if ($env:LOCALAPPDATA) { $roots += (Join-Path $env:LOCALAPPDATA 'Microsoft\EdgeWebView\Application') }
  $candidates = @()
  foreach ($runtimeRoot in $roots) {
    if (-not (Test-Path -LiteralPath $runtimeRoot)) { continue }
    if ($Version) {
      $candidates += (Join-Path $runtimeRoot $Version)
    } else {
      $candidates += @(Get-ChildItem -LiteralPath $runtimeRoot -Directory -ErrorAction SilentlyContinue | ForEach-Object FullName)
    }
  }
  $best = $null
  $bestVersion = $null
  foreach ($candidate in $candidates) {
    $exe = Join-Path $candidate 'msedgewebview2.exe'
    if (-not (Test-Path -LiteralPath $exe)) { continue }
    $name = Split-Path $candidate -Leaf
    $parsed = $null
    if (-not [version]::TryParse($name, [ref] $parsed)) { $parsed = [version] '0.0.0.0' }
    if (-not $best -or $parsed -gt $bestVersion) {
      $best = $exe
      $bestVersion = $parsed
    }
  }
  return $best
}

function Get-WebView2Runtime {
  <#
    The installed Evergreen Runtime, or $null. Registry first (the documented check), then the
    known version folders when a registry entry exists but has no usable location.
  #>
  if (($PSVersionTable.PSEdition -eq 'Core') -and -not $IsWindows) { return $null }
  $keys = @(
    @{ Path = "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$script:WebView2Client"; Scope = 'PerMachine'; View = '64-bit WOW6432Node' }
    @{ Path = "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$script:WebView2Client"; Scope = 'PerMachine'; View = 'native' }
    @{ Path = "HKCU:\Software\Microsoft\EdgeUpdate\Clients\$script:WebView2Client"; Scope = 'PerUser'; View = 'current user' }
  )
  $found = @()
  foreach ($key in $keys) {
    $item = Get-ItemProperty -LiteralPath $key.Path -ErrorAction SilentlyContinue
    if (-not $item) { continue }
    if (-not (Test-WebView2Version $item.pv)) { continue }
    $exe = $null
    if ($item.location) {
      $candidate = Join-Path $item.location 'msedgewebview2.exe'
      if (Test-Path -LiteralPath $candidate) { $exe = $candidate }
    }
    if (-not $exe) { $exe = Find-WebView2Executable -Version $item.pv }
    if (-not $exe) { $exe = Find-WebView2Executable }
    $found += [pscustomobject]@{
      Version = [string] $item.pv
      Scope = $key.Scope
      Source = $key.View
      RegistryKey = $key.Path
      InstallPath = if ($exe) { Split-Path $exe -Parent } else { $null }
      Executable = $exe
      Healthy = [bool] $exe
    }
  }
  # A registry entry with pv 0.0.0.0 is the documented "not installed" state. Do not let a folder
  # left behind by an uninstall turn that into a false pass.
  if ($found.Count -eq 0) { return $null }
  $best = $null
  $bestVersion = [version] '0.0.0.0'
  foreach ($item in $found) {
    $parsed = [version] $item.Version
    if (-not $best -or $parsed -gt $bestVersion) {
      $best = $item
      $bestVersion = $parsed
    }
  }
  return $best
}

function Get-WebView2RecoveryText {
  param([string] $Reason = '')
  $lines = @(
    'Microsoft Edge WebView2 Evergreen Runtime is not installed.',
    $(if ($Reason) { $Reason }),
    '',
    'The Assistant on Windows is a WebView2 page and has no built-in fallback.',
    '',
    'Online recovery:',
    "  pwsh packaging/windows/webview2-runtime.ps1 -Action Install -Mode Online",
    "  Official bootstrapper: $script:WebView2OnlineBootstrapper",
    '',
    'Offline recovery:',
    "  Download the Evergreen Standalone Installer from $script:WebView2DownloadPage",
    '  (choose the architecture of this Windows installation), then:',
    '  pwsh packaging/windows/webview2-runtime.ps1 -Action Install -Mode Offline -OfflineInstaller <path> -ExpectedSha256 <sha256>',
    '',
    'A per-machine install needs an elevated shell; an unelevated shell installs for this user.'
  )
  return ($lines -join [Environment]::NewLine)
}

function Assert-WebView2Runtime {
  param([string] $Reason = '')
  $runtime = Get-WebView2Runtime
  if (-not $runtime) { throw (Get-WebView2RecoveryText -Reason $Reason) }
  if (-not $runtime.Healthy) {
    throw ("The WebView2 Runtime registry entry says version $($runtime.Version), but " +
      "msedgewebview2.exe was not found under its install path or the known version folders. " +
      "The runtime is damaged; reinstall it. It is not treated as present." +
      [Environment]::NewLine + (Get-WebView2RecoveryText))
  }
  return $runtime
}

function Test-Elevated {
  if (($PSVersionTable.PSEdition -eq 'Core') -and -not $IsWindows) { return $false }
  $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-Sha256 {
  param([string] $Path)
  return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-InstallerSignature {
  param([string] $Path)
  $signature = Get-AuthenticodeSignature -LiteralPath $Path
  if ($signature.Status -ne 'Valid') {
    throw "Refusing to run ${Path}: its Authenticode signature is $($signature.Status), not Valid."
  }
  $subject = if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { '' }
  if ($subject -notmatch 'Microsoft') {
    throw "Refusing to run ${Path}: it is signed by '$subject', which does not name Microsoft."
  }
}

function Write-WebView2Evidence {
  param(
    [string] $Directory,
    [string] $Action,
    [string] $Mode,
    $Before,
    $After,
    [string] $Installer,
    [string] $Sha256,
    [string] $Detail
  )
  if (-not $Directory) { return }
  New-Item -ItemType Directory -Force -Path $Directory | Out-Null
  $record = [ordered]@{
    action = $Action
    mode = $Mode
    scope = $Scope
    timestampUtc = (Get-Date).ToUniversalTime().ToString('o')
    before = $Before
    after = $After
    installer = $Installer
    installerSha256 = $Sha256
    detail = $Detail
    onlineBootstrapper = $script:WebView2OnlineBootstrapper
    downloadPage = $script:WebView2DownloadPage
  }
  $path = Join-Path $Directory 'webview2-runtime.json'
  $record | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $path -Encoding utf8
  Write-Output "WebView2 record: $path"
}

function Install-WebView2Runtime {
  <#
    Install the Evergreen Runtime when it is absent and return the detected runtime. Every path
    that cannot install throws: no caller can mistake this for success.
  #>
  param(
    [ValidateSet('Auto', 'Online', 'Offline')] [string] $InstallMode = $Mode,
    [ValidateSet('Auto', 'PerMachine', 'PerUser')] [string] $InstallScope = $Scope,
    [string] $InstallerPath = $OfflineInstaller,
    [string] $Sha256 = $ExpectedSha256,
    [string] $EvidenceDirectory = $Evidence
  )
  $before = Get-WebView2Runtime
  if ($before -and $before.Healthy) {
    Write-Output "WebView2 runtime already installed: $($before.Version) ($($before.Scope), $($before.Source))"
    Write-WebView2Evidence -Directory $EvidenceDirectory -Action 'Install' -Mode $InstallMode `
      -Before $before -After $before -Installer $null -Sha256 $null -Detail 'already installed'
    return $before
  }
  if ($before -and -not $before.Healthy) {
    throw ("A WebView2 registry entry exists for version $($before.Version), but its executable is " +
      "missing. Reinstall the runtime rather than trusting the stale entry." +
      [Environment]::NewLine + (Get-WebView2RecoveryText))
  }

  $elevated = Test-Elevated
  if ($InstallScope -eq 'PerMachine' -and -not $elevated) {
    throw 'A per-machine WebView2 install needs an elevated shell; this process is not elevated. Run it from an elevated prompt or choose -Scope PerUser.'
  }
  if ($InstallScope -eq 'PerUser' -and $elevated) {
    throw 'A per-user WebView2 install needs an unelevated shell. This process is elevated, so the installer would install for the machine. Run it without elevation or choose -Scope PerMachine.'
  }

  $useOffline = ($InstallMode -eq 'Offline') -or (($InstallMode -eq 'Auto') -and $InstallerPath)
  if ($useOffline) {
    if (-not $InstallerPath) {
      throw ("Offline WebView2 install needs -OfflineInstaller. Download the Evergreen Standalone " +
        "Installer from $script:WebView2DownloadPage (the release owner supplies its SHA-256); " +
        'this script does not invent a fixed-version URL.')
    }
    if (-not (Test-Path -LiteralPath $InstallerPath)) {
      throw "Offline WebView2 installer not found: $InstallerPath"
    }
    if (-not $Sha256 -and -not $AllowUnverifiedInstaller) {
      throw ("Offline WebView2 install needs -ExpectedSha256 (or SOLIDCRAFT_WEBVIEW2_INSTALLER_SHA256). " +
        'A standalone installer is not downloaded from a stable URL here, so the release owner must ' +
        'supply the hash. Pass -AllowUnverifiedInstaller only with a file you obtained and checked yourself.')
    }
  } else {
    if ($InstallMode -eq 'Offline') {
      throw 'Offline WebView2 install needs -OfflineInstaller.'
    }
  }

  $temporary = Join-Path ([IO.Path]::GetTempPath()) ("solidcraft-webview2-" + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Force -Path $temporary | Out-Null
  $installer = $null
  try {
    if ($useOffline) {
      $installer = (Resolve-Path -LiteralPath $InstallerPath).Path
      if ($Sha256) {
        $actual = Get-Sha256 $installer
        if ($actual -ne $Sha256.ToLowerInvariant()) {
          throw "WebView2 installer SHA-256 mismatch: expected $($Sha256.ToLowerInvariant()), got $actual ($installer)."
        }
        Write-Output "WebView2 installer SHA-256 verified: $actual"
      } else {
        Write-Warning "the offline WebView2 installer is not checksum-verified (only its Authenticode signature is checked): $installer"
      }
      Write-Output "using offline WebView2 installer: $installer"
    } else {
      $installer = Join-Path $temporary 'MicrosoftEdgeWebview2Setup.exe'
      Write-Output "downloading the WebView2 Evergreen Bootstrapper from $script:WebView2OnlineBootstrapper"
      try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
      } catch {
        Write-Verbose "could not set TLS 1.2 explicitly: $($_.Exception.Message)"
      }
      try {
        Invoke-WebRequest -Uri $script:WebView2OnlineBootstrapper -OutFile $installer -UseBasicParsing
      } catch {
        throw ("Could not download the WebView2 Bootstrapper from $script:WebView2OnlineBootstrapper. " +
          "This machine may be offline. Use the offline path instead: download the Evergreen Standalone " +
          "Installer from $script:WebView2DownloadPage on a connected machine, copy it here, and run " +
          'this script with -Mode Offline -OfflineInstaller <path> -ExpectedSha256 <sha256>. ' +
          "Original error: $($_.Exception.Message)")
      }
      if ($Sha256) {
        $actual = Get-Sha256 $installer
        if ($actual -ne $Sha256.ToLowerInvariant()) {
          throw "WebView2 bootstrapper SHA-256 mismatch: expected $($Sha256.ToLowerInvariant()), got $actual ($installer)."
        }
        Write-Output "WebView2 bootstrapper SHA-256 verified: $actual"
      } else {
        Write-Output 'no stable SHA-256 is published for the changing Evergreen Bootstrapper; its Microsoft Authenticode signature is checked instead'
      }
    }

    Assert-InstallerSignature $installer
    # The file is now signature-checked (and checksum-checked on the offline path). Clear the
    # Mark-of-the-Web so an unattended install cannot sit behind a SmartScreen prompt.
    try { Unblock-File -LiteralPath $installer -ErrorAction SilentlyContinue } catch {
      Write-Verbose "could not clear the installer's Mark-of-the-Web: $($_.Exception.Message)"
    }
    Write-Output '==> installing the WebView2 Evergreen Runtime (/silent /install)'
    $process = Start-Process -FilePath $installer -ArgumentList @('/silent', '/install') -Wait -PassThru
    if ($process.ExitCode -ne 0) {
      $logs = @(
        (Join-Path $env:ALLUSERSPROFILE 'Microsoft\EdgeUpdate\Log\MicrosoftEdgeUpdate.log'),
        (Join-Path $env:LOCALAPPDATA 'Temp\msedge_installer.log'),
        (Join-Path $env:TEMP 'msedge_installer.log')
      ) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }
      $tail = if ($logs) { ($logs | ForEach-Object { "--- $_"; Get-Content -LiteralPath $_ -Tail 40 -ErrorAction SilentlyContinue }) -join [Environment]::NewLine } else { '(no installer log was found)' }
      throw ("The WebView2 installer exited $($process.ExitCode); the runtime is not known to be installed. " +
        "Logs: $($logs -join ', ')" + [Environment]::NewLine + $tail)
    }

    $deadline = (Get-Date).AddSeconds($WaitSeconds)
    $after = $null
    do {
      Start-Sleep -Seconds 2
      $after = Get-WebView2Runtime
      if ($after -and $after.Healthy) { break }
    } while ((Get-Date) -lt $deadline)
    if (-not $after) {
      throw ("The WebView2 installer exited 0, but the runtime was not detected within $WaitSeconds s. " +
        'Check the installer logs and the machine policy that controls WebView2 installation. ' +
        [Environment]::NewLine + (Get-WebView2RecoveryText))
    }
    if (-not $after.Healthy) {
      throw ("The WebView2 installer exited 0 and the registry names version $($after.Version), but " +
        'msedgewebview2.exe was not found. The runtime is damaged, not installed.')
    }
    $sha = if ($Sha256) { $Sha256.ToLowerInvariant() } else { $null }
    Write-WebView2Evidence -Directory $EvidenceDirectory -Action 'Install' -Mode $InstallMode `
      -Before $before -After $after -Installer $installer -Sha256 $sha `
      -Detail $(if ($useOffline) { 'offline installer' } else { 'online bootstrapper' })
    Write-Output "WebView2 runtime installed: $($after.Version) ($($after.Scope), $($after.Source))"
    return $after
  } finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
  }
}

function Write-WebView2Report {
  param([string] $EvidenceDirectory = $Evidence)
  $runtime = Get-WebView2Runtime
  if ($runtime) {
    $state = "WebView2 runtime $($runtime.Version) ($($runtime.Scope), $($runtime.Source)); executable: $($runtime.Executable)"
    Write-Output $state
  } else {
    $state = Get-WebView2RecoveryText
    Write-Output $state
  }
  Write-WebView2Evidence -Directory $EvidenceDirectory -Action 'Report' -Mode $Mode -Before $runtime -After $runtime `
    -Installer $null -Sha256 $null -Detail $state
  return $state
}

# Dot-sourcing this file (installed-check.ps1 and tools/windows-assistant-smoke.ps1 do) defines the
# functions without running an action.
if ($MyInvocation.InvocationName -eq '.') { return }

switch ($Action) {
  'Detect' {
    $runtime = Get-WebView2Runtime
    Write-WebView2Evidence -Directory $Evidence -Action 'Detect' -Mode $Mode -Before $runtime -After $runtime `
      -Installer $null -Sha256 $null -Detail 'detection'
    if ($Json) {
      $runtime | ConvertTo-Json -Depth 5
    } elseif ($runtime) {
      Write-Output "WebView2 runtime $($runtime.Version) ($($runtime.Scope), $($runtime.Source)); executable: $($runtime.Executable)"
    } else {
      Write-Output (Get-WebView2RecoveryText)
    }
  }
  'Require' {
    $runtime = Assert-WebView2Runtime
    Write-WebView2Evidence -Directory $Evidence -Action 'Require' -Mode $Mode -Before $runtime -After $runtime `
      -Installer $null -Sha256 $null -Detail 'required'
    Write-Output "WebView2 runtime $($runtime.Version) ($($runtime.Scope), $($runtime.Source))"
  }
  'Install' {
    $runtime = Install-WebView2Runtime
    Write-Output "WebView2 runtime $($runtime.Version) ($($runtime.Scope), $($runtime.Source))"
  }
  'Report' {
    [void](Write-WebView2Report)
  }
}
