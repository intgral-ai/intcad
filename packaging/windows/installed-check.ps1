<#
.SYNOPSIS
  Install the MSI as a user would, open the Assistant in the installed app, upgrade, and uninstall.

.DESCRIPTION
  For the Windows package job (.github/workflows/release.yml), after package.ps1. A build tree
  proves nothing about an install: the installed app lives under Program Files, where it cannot
  write beside itself, and it is whatever the MSI carries.

    0. The MSI's identity is read with Windows Installer itself: ProductCode, UpgradeCode and the
       Upgrade table. A real major upgrade is exercised only when a previous MSI is supplied with
       -PreviousMsi (or SOLIDCRAFT_PREVIOUS_MSI); without one the check says so instead of claiming
       upgrade coverage.
    1. The machine has no install yet (one already there would be what gets checked).
    2. A supplied previous MSI is installed; then msiexec installs this MSI silently, for the
       machine, into its default folder. The previous product's ARP entry must be gone.
    3. The installed solidcraft.exe is byte for byte the one this job built.
    4. The installed solidcraft-cli.exe runs.
    5. The WebView2 Evergreen Runtime is detected. It is the Assistant's only renderer: there is
       no built-in fallback, so absence fails the check unless the recovery path is exercised.
       -WebView2Recovery Auto (default) installs it from the official online bootstrapper, or from
       -WebView2OfflineInstaller when one is given. ReportOnly refuses to pass and writes that the
       Assistant page was NOT checked.
    6. tools/windows-assistant-smoke.ps1 opens the Assistant in the installed app. With the
       runtime present the page must load and report in. The smoke check also runs once with the
       runtime hidden: the app must say the view could not be made and leave no blank window over
       the panel. (x64 only, see below.)
    7. msiexec removes it again. The program files, the ARP entry, the InstallDir registry value
       and the PATH entry are gone.

  The 32-bit build (-Arch x86) is installed, compared, asked for its version and removed, and
  not started: on a GitHub runner (2026-11-11, run 43112420636) the installed 32-bit app showed
  no window within three minutes, before the Assistant was ever opened. No 32-bit build had
  been started in CI before, so whether that is the runner (no graphics card) or the build is
  not known. The step says so in the job's summary instead of passing in silence.

  A package check never starts a release build of the desktop app with the network open
  (docs/releasing.md): for step 6 the firewall blocks the installed app and the WebView2
  runtime's processes from reaching out, a copy of curl.exe under the same kind of rule shows
  that the block holds on this machine, and the app's update check is off as well
  (SOLIDCRAFT_UPDATE_CHECK=0). The rules are removed afterwards.

  It takes the mouse and the keyboard, and it installs and removes the app for the whole
  machine: for a CI runner, never a person's machine. Needs RUNNER_TEMP (the smoke check's).

.EXAMPLE
  pwsh packaging/windows/installed-check.ps1 -Arch x64 -Evidence $env:RUNNER_TEMP/installed-check

.EXAMPLE
  pwsh packaging/windows/installed-check.ps1 -Arch x64 -PreviousMsi C:\artifacts\previous.msi

.EXAMPLE
  pwsh packaging/windows/installed-check.ps1 -Arch x64 -WebView2Recovery Offline `
    -WebView2OfflineInstaller C:\downloads\MicrosoftEdgeWebView2RuntimeInstallerX64.exe `
    -WebView2InstallerSha256 <sha256 from the release owner>
#>
param(
  [ValidateSet('x64', 'x86')] [string] $Arch = 'x64',
  [string] $Evidence = 'installed-check',
  [ValidateSet('Auto', 'Online', 'Offline', 'ReportOnly')] [string] $WebView2Recovery = 'Auto',
  [string] $WebView2OfflineInstaller = $env:SOLIDCRAFT_WEBVIEW2_OFFLINE_INSTALLER,
  [string] $WebView2InstallerSha256 = $env:SOLIDCRAFT_WEBVIEW2_INSTALLER_SHA256,
  [string] $PreviousMsi = $env:SOLIDCRAFT_PREVIOUS_MSI
)
$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $RepoRoot 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $RepoRoot 'target' }

# webview2-runtime.ps1 has its own `$Evidence` parameter; dot-sourcing it in this scope would
# replace ours with that default (empty) before line 86 uses it. Keep the caller's value.
$EvidenceRoot = $Evidence
. (Join-Path $PSScriptRoot 'webview2-runtime.ps1')
$Evidence = $EvidenceRoot

# Never change this: it is what lets a new MSI replace every earlier install, including those from
# before the SolveCraft -> SolidCraft rename (assets/platforms/windows/solidcraft.wxs).
$ExpectedUpgradeCode = '6A145FD6-D285-428D-AE84-A69570CDF4DB'

$found = @(Get-ChildItem $Dist -Filter "solidcraft-*-windows-$Arch.msi")
if ($found.Count -ne 1) { throw "expected one $Arch MSI in $Dist, found $($found.Count)" }
$Msi = $found[0].FullName
$Built = Join-Path $TargetDir "windows-package\$Arch\solidcraft.exe"
if (-not (Test-Path $Built)) { throw "no built solidcraft.exe at $Built (run package.ps1 first)" }
# The MSI installs for the machine, a 32-bit build under "Program Files (x86)" on 64-bit Windows.
$ProgramFiles = if ($Arch -eq 'x86' -and ${env:ProgramFiles(x86)}) { ${env:ProgramFiles(x86)} } else { $env:ProgramFiles }
$Dir = Join-Path $ProgramFiles 'SolidCraft'
if (Test-Path $Dir) { throw "there is an install at $Dir already: this machine is not a clean one, so the check would prove nothing" }

New-Item -ItemType Directory -Force -Path $Evidence | Out-Null
$Evidence = (Resolve-Path $Evidence).Path
$RuleGroup = 'Intcad installed check'
$firewallBefore = @()

function Add-Summary([string] $Text) {
  Write-Output $Text
  if ($env:GITHUB_STEP_SUMMARY) { Add-Content -Path $env:GITHUB_STEP_SUMMARY -Value $Text }
}

function Invoke-MsiFile([string] $File, [string] $Verb, [string] $Log) {
  $run = Start-Process msiexec.exe -ArgumentList @($Verb, "`"$File`"", '/qn', '/norestart', '/L*v', "`"$Log`"") -Wait -PassThru
  if ($run.ExitCode -ne 0) {
    Get-Content $Log -Tail 60 -ErrorAction SilentlyContinue
    throw "msiexec $Verb $File exited $($run.ExitCode)"
  }
}

function Get-MsiMetadata([string] $File) {
  <#
    ProductCode, ProductVersion and the Upgrade table, read from the MSI database itself. If the
    Windows Installer COM object is unavailable this fails: a package check must not silently skip
    the upgrade identity it claims to check.
  #>
  try {
    $installer = New-Object -ComObject WindowsInstaller.Installer
  } catch {
    throw "cannot read the MSI's identity: the WindowsInstaller.Installer COM object is unavailable ($($_.Exception.Message))"
  }
  $database = $installer.OpenDatabase($File, 0)
  $properties = @{}
  $view = $database.OpenView('SELECT Property, Value FROM Property')
  $view.Execute()
  while ($record = $view.Fetch()) { $properties[$record.StringData(1)] = $record.StringData(2) }
  $view.Close()

  $upgrades = @()
  try {
    $view = $database.OpenView('SELECT UpgradeCode, VersionMin, VersionMax, Attributes, ActionProperty FROM Upgrade')
    $view.Execute()
    while ($record = $view.Fetch()) {
      $upgrades += [pscustomobject]@{
        # Windows Installer returns GUIDs in braces; compare the same form as $ExpectedUpgradeCode.
        UpgradeCode = ($record.StringData(1) -replace '[{}]', '')
        VersionMin = $record.StringData(2)
        VersionMax = $record.StringData(3)
        Attributes = [int] $record.IntegerData(4)
        ActionProperty = $record.StringData(5)
      }
    }
    $view.Close()
  } catch {
    throw "cannot read the MSI's Upgrade table: $($_.Exception.Message)"
  }

  $upgradeCode = $properties['UpgradeCode']
  if (-not $upgradeCode -and $upgrades.Count -gt 0) { $upgradeCode = $upgrades[0].UpgradeCode }
  $upgradeCode = $upgradeCode -replace '[{}]', ''
  return [pscustomobject]@{
    ProductCode = $properties['ProductCode']
    ProductVersion = $properties['ProductVersion']
    ProductName = $properties['ProductName']
    UpgradeCode = $upgradeCode
    Upgrades = $upgrades
  }
}

function Get-ArpEntry([string] $ProductCode) {
  $paths = @(
    "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode",
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode",
    "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode"
  )
  foreach ($path in $paths) {
    $entry = Get-ItemProperty -LiteralPath $path -ErrorAction SilentlyContinue
    if ($entry) {
      return [pscustomobject]@{ Path = $path; DisplayName = $entry.DisplayName; DisplayVersion = $entry.DisplayVersion }
    }
  }
  return $null
}

$meta = Get-MsiMetadata $Msi
foreach ($field in 'ProductCode', 'ProductVersion', 'UpgradeCode') {
  if (-not $meta.$field) { throw "the MSI has no ${field}: it cannot be installed or upgraded safely" }
}
if ($meta.UpgradeCode -ne $ExpectedUpgradeCode) {
  throw "the MSI's UpgradeCode is $($meta.UpgradeCode), expected $ExpectedUpgradeCode; a new MSI would not replace earlier installs"
}
if ($meta.Upgrades.Count -eq 0) {
  throw "the MSI has no Upgrade table: a new MSI would not replace this one"
}
# MajorUpgrade AllowSameVersionUpgrades writes VersionMaxInclusive (0x200) in the Upgrade table.
$sameVersion = @($meta.Upgrades | Where-Object { ($_.Attributes -band 0x200) -ne 0 }).Count -gt 0
if (-not $sameVersion) {
  Add-Summary "Windows upgrade check: the MSI's Upgrade table does not allow same-version upgrades; successive builds of one version would not replace each other."
}
if ($env:SOLIDCRAFT_VERSION) {
  $expectedMsiVersion = ($env:SOLIDCRAFT_VERSION -split '-')[0]
  if ($meta.ProductVersion -ne $expectedMsiVersion) {
    throw "the MSI's ProductVersion is $($meta.ProductVersion), expected $expectedMsiVersion from SOLIDCRAFT_VERSION=$env:SOLIDCRAFT_VERSION"
  }
}
Write-Output "MSI identity: ProductCode $($meta.ProductCode), UpgradeCode $($meta.UpgradeCode), ProductVersion $($meta.ProductVersion)"
Write-Output "MSI Upgrade table rows: $($meta.Upgrades.Count); same-version upgrades allowed: $sameVersion"

$previousMeta = $null
$previousInstalled = $false
if ($PreviousMsi) {
  if (-not (Test-Path -LiteralPath $PreviousMsi)) { throw "the previous MSI was not found: $PreviousMsi" }
  $previousMeta = Get-MsiMetadata (Resolve-Path -LiteralPath $PreviousMsi).Path
  foreach ($field in 'ProductCode', 'ProductVersion', 'UpgradeCode') {
    if (-not $previousMeta.$field) { throw "the previous MSI has no ${field}: it cannot be used as an upgrade artifact" }
  }
  if ($previousMeta.ProductCode -eq $meta.ProductCode) {
    throw "the previous MSI has the same ProductCode as this one ($($meta.ProductCode)); that is not an upgrade artifact"
  }
  if ($previousMeta.UpgradeCode -ne $meta.UpgradeCode) {
    throw "the previous MSI's UpgradeCode is $($previousMeta.UpgradeCode), expected $($meta.UpgradeCode): it is not an earlier version of this product"
  }
  Write-Output "previous MSI identity: ProductCode $($previousMeta.ProductCode), ProductVersion $($previousMeta.ProductVersion)"
  Write-Output '==> installing the previous MSI for the upgrade check'
  Invoke-MsiFile (Resolve-Path -LiteralPath $PreviousMsi).Path '/i' (Join-Path $Evidence 'msi-previous-install.log')
  $previousInstalled = $true
  $previousArp = Get-ArpEntry $previousMeta.ProductCode
  if (-not $previousArp) { throw "the previous MSI installed, but its ARP entry is missing" }
} else {
  Add-Summary "not covered: Windows upgrade check. No previous MSI was supplied (SOLIDCRAFT_PREVIOUS_MSI or -PreviousMsi), so a real major upgrade was NOT exercised. The MSI's UpgradeCode, ProductCode and Upgrade table are checked, and install/uninstall and ARP removal are checked."
}

$failure = $null
$currentInstalled = $false
try {
  Write-Output "==> installing $(Split-Path $Msi -Leaf)"
  Invoke-MsiFile $Msi '/i' (Join-Path $Evidence 'msi-install.log')
  $currentInstalled = $true

  if ($previousInstalled) {
    $left = Get-ArpEntry $previousMeta.ProductCode
    if ($left) { throw "after the upgrade, the previous product $($previousMeta.ProductCode) is still in ARP at $($left.Path)" }
    Write-Output "ok: the upgrade removed the previous product's ARP entry ($($previousMeta.ProductCode))"
  }
  $arp = Get-ArpEntry $meta.ProductCode
  if (-not $arp) { throw "the installed MSI has no ARP entry for ProductCode $($meta.ProductCode)" }
  # Windows Installer ProductVersion is numeric (0.1.0); the app's beta suffix lives in the
  # product display/CLI version, not the MSI version. Compare ARP with the MSI, then assert the
  # installed CLI still reports the full release version.
  if ($arp.DisplayVersion -and $arp.DisplayVersion -ne $meta.ProductVersion) {
    throw "the ARP DisplayVersion is $($arp.DisplayVersion), expected the MSI ProductVersion $($meta.ProductVersion)"
  }
  Write-Output "ok: the ARP entry is present ($($arp.Path), DisplayVersion $($arp.DisplayVersion))"

  $App = Join-Path $Dir 'solidcraft.exe'
  $Cli = Join-Path $Dir 'solidcraft-cli.exe'
  $Notices = Join-Path $Dir 'ASSISTANT-THIRD-PARTY-NOTICES.txt'
  foreach ($file in $App, $Cli, $Notices) { if (-not (Test-Path $file)) { throw "the MSI did not install $file" } }
  Get-ChildItem $Dir | Format-Table Name, Length
  $installed = (Get-FileHash $App -Algorithm SHA256).Hash
  $expected = (Get-FileHash $Built -Algorithm SHA256).Hash
  if ($installed -ne $expected) { throw "the installed solidcraft.exe ($installed) is not the one this job built ($expected)" }
  Write-Output "ok: the installed solidcraft.exe is the one this job built ($installed)"
  & $Cli --version
  if ($LASTEXITCODE -ne 0) { throw "the installed solidcraft-cli --version exited $LASTEXITCODE" }
  Write-Output 'ok: the installed solidcraft-cli runs'

  # The Assistant is WebView2 and has no built-in fallback. Detect before the app starts so a
  # machine without the runtime fails (or exercises recovery) rather than showing a blank panel.
  $runtime = Get-WebView2Runtime
  if ($runtime -and -not $runtime.Healthy) {
    throw "the WebView2 registry names version $($runtime.Version), but msedgewebview2.exe is missing; the runtime is damaged, not present"
  }
  if (-not $runtime) {
    if ($WebView2Recovery -eq 'ReportOnly') {
      $gap = "Windows Assistant check: the WebView2 Runtime is NOT installed, and -WebView2Recovery ReportOnly was requested. The Assistant page was NOT checked and this step does not pass. Install the runtime with packaging/windows/webview2-runtime.ps1 -Action Install (online or offline)."
      Add-Summary $gap
      throw $gap
    }
    $installMode = switch ($WebView2Recovery) {
      'Online' { 'Online' }
      'Offline' { 'Offline' }
      default { 'Auto' }
    }
    Add-Summary "Windows Assistant check: the WebView2 Runtime was absent; exercising the $WebView2Recovery recovery path."
    Write-Output "==> WebView2 Runtime absent: $WebView2Recovery recovery"
    $runtime = Install-WebView2Runtime -InstallMode $installMode -InstallScope 'Auto' `
      -InstallerPath $WebView2OfflineInstaller -Sha256 $WebView2InstallerSha256 -EvidenceDirectory $Evidence
    if (-not $runtime) { throw 'the WebView2 recovery path returned no runtime' }
    Add-Summary "Windows Assistant check: WebView2 Runtime $($runtime.Version) was installed by the recovery path and is detected after installation."
  } else {
    Add-Summary "Windows Assistant check: WebView2 Runtime $($runtime.Version) ($($runtime.Scope), $($runtime.Source)) is present; the Assistant page must load."
  }
  Write-Output "ok: WebView2 Runtime $($runtime.Version) ($($runtime.Scope), $($runtime.Source))"

  if ($Arch -ne 'x64') {
    $gap = "Windows $Arch installed check: the MSI was installed, compared with the build and removed, and the app was NOT started. On a runner the 32-bit app showed no window within three minutes (run 43112420636); the cause is not known."
    Write-Output "skipped: $gap"
    if ($env:GITHUB_STEP_SUMMARY) { Add-Content -Path $env:GITHUB_STEP_SUMMARY -Value $gap }
  } else {
    Write-Output '==> taking the network away from the installed app'
    # Windows Firewall rules name a program's file. The rule kind is tried first on a copy of
    # curl.exe: a firewall that is off, or managed some other way, would let the app out too.
    $probe = Join-Path $Evidence 'network-probe.exe'
    Copy-Item (Join-Path $env:SystemRoot 'System32\curl.exe') $probe
    $blocked = @($App, $probe)
    # The web view's processes are the WebView2 runtime's, not the app's. A machine runtime lives
    # under Program Files (x86) or Program Files; a per-user runtime lives under LocalAppData.
    $webViewRoots = @()
    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, $env:LOCALAPPDATA)) {
      if ($base) {
        $root = Join-Path $base 'Microsoft\EdgeWebView\Application'
        if (Test-Path -LiteralPath $root) { $webViewRoots += $root }
      }
    }
    foreach ($webViewRoot in $webViewRoots) {
      $blocked += @(Get-ChildItem (Join-Path $webViewRoot '*\msedgewebview2.exe') -ErrorAction SilentlyContinue | ForEach-Object FullName)
    }
    $blocked = @($blocked | Sort-Object -Unique)
    $firewallBefore = @(Get-NetFirewallProfile | ForEach-Object { @{ Name = $_.Name; Enabled = $_.Enabled } })
    Set-NetFirewallProfile -All -Enabled True
    foreach ($program in $blocked) {
      New-NetFirewallRule -Group $RuleGroup -DisplayName "installed check: no network for $(Split-Path $program -Leaf)" `
        -Direction Outbound -Program $program -Action Block | Out-Null
    }
    & $probe -s -m 8 -o NUL https://api.github.com/
    $probeExit = $LASTEXITCODE
    # curl is meant to fail here. Its exit code must not stay behind as the last one: a GitHub
    # Actions step ends with whatever $LASTEXITCODE holds, and failed on it with every check passed.
    $global:LASTEXITCODE = 0
    if ($probeExit -eq 0) { throw 'the firewall rule does not take the network away on this machine, so the installed app is not started' }
    Write-Output "ok: the firewall blocks $($blocked.Count) programs from the network (curl under the same rule: exit $probeExit)"
    $env:SOLIDCRAFT_UPDATE_CHECK = '0'

    Write-Output '==> the Assistant in the installed app'
    & (Join-Path $RepoRoot 'tools\windows-assistant-smoke.ps1') -Exe $App -Evidence (Join-Path $Evidence 'assistant') `
      -WebView2Recovery $WebView2Recovery -WebView2OfflineInstaller $WebView2OfflineInstaller `
      -WebView2InstallerSha256 $WebView2InstallerSha256

    $after = Get-WebView2Runtime
    if (-not $after -or -not $after.Healthy) {
      throw 'the WebView2 runtime is no longer detected after the Assistant check; the app or the check removed a machine runtime it does not own'
    }
    Write-Output "ok: the WebView2 runtime is still detected after the app check ($($after.Version))"
  }
} catch {
  $failure = $_
} finally {
  Get-NetFirewallRule -Group $RuleGroup -ErrorAction SilentlyContinue | Remove-NetFirewallRule
  foreach ($was in $firewallBefore) { Set-NetFirewallProfile -Name $was.Name -Enabled $was.Enabled }
  Remove-Item -Force (Join-Path $Evidence 'network-probe.exe') -ErrorAction SilentlyContinue
  Write-Output '==> uninstalling'
  # A failed check may have left the app running, and the uninstall cannot remove a running file.
  Get-Process solidcraft -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  if ($currentInstalled -or (Test-Path (Join-Path $Dir 'solidcraft.exe'))) {
    try {
      Invoke-MsiFile $Msi '/x' (Join-Path $Evidence 'msi-uninstall.log')
    } catch {
      if ($failure) { Write-Output "the uninstall failed as well: $_" } else { $failure = $_ }
    }
  }
  if ($previousInstalled) {
    $left = Get-ArpEntry $previousMeta.ProductCode
    if ($left) {
      Write-Output "the previous product $($previousMeta.ProductCode) is still installed after the upgrade; removing it"
      try {
        Invoke-MsiFile (Resolve-Path -LiteralPath $PreviousMsi).Path '/x' (Join-Path $Evidence 'msi-previous-uninstall.log')
      } catch {
        if ($failure) { Write-Output "the previous product's uninstall failed as well: $_" } else { $failure = $_ }
      }
    }
  }
}
if ($failure) { throw $failure }
if (Test-Path (Join-Path $Dir 'solidcraft.exe')) { throw "after the uninstall, solidcraft.exe is still in $Dir" }
if (Get-ArpEntry $meta.ProductCode) { throw "after the uninstall, ProductCode $($meta.ProductCode) is still in ARP" }
if ($previousMeta -and (Get-ArpEntry $previousMeta.ProductCode)) { throw "after the uninstall, the previous ProductCode $($previousMeta.ProductCode) is still in ARP" }
$installDir = (Get-ItemProperty -Path 'HKLM:\Software\SolidCraft' -Name InstallDir -ErrorAction SilentlyContinue).InstallDir
if ($installDir) { throw "after the uninstall, the InstallDir registry value is still $installDir" }
$machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
if ($machinePath -and (($machinePath -split ';') -contains $Dir)) { throw "after the uninstall, the install folder is still on the machine PATH" }
Write-Output 'ok: the uninstall removed the program files, the ARP entry, the InstallDir value and the PATH entry'
Write-Output 'PASS: the installed app'
exit 0
