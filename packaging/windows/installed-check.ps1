<#
.SYNOPSIS
  Install the MSI as a user would, open the Assistant in the installed app, and uninstall.

.DESCRIPTION
  For the Windows package job (.github/workflows/release.yml), after package.ps1. A build tree
  proves nothing about an install: the installed app lives under Program Files, where it cannot
  write beside itself, and it is whatever the MSI carries.

    1. The machine has no install yet (one already there would be what gets checked).
    2. msiexec installs the MSI silently, for the machine, into its default folder.
    3. The installed solidcraft.exe is byte for byte the one this job built.
    4. The installed solidcraft-cli.exe runs.
    5. tools/windows-assistant-smoke.ps1 opens the Assistant in the installed app: the page
       loads, takes and gives back the keyboard, and writes nothing beside the exe; with the
       WebView2 runtime hidden the built-in conversation is drawn instead.
    6. msiexec removes it again, and the program files are gone.

  A package check never starts a release build of the desktop app with the network open
  (docs/releasing.md): for step 5 the firewall blocks the installed app and the WebView2
  runtime's processes from reaching out, a copy of curl.exe under the same kind of rule shows
  that the block holds on this machine, and the app's update check is off as well
  (SOLIDCRAFT_UPDATE_CHECK=0). The rules are removed afterwards.

  It takes the mouse and the keyboard, and it installs and removes the app for the whole
  machine: for a CI runner, never a person's machine. Needs RUNNER_TEMP (the smoke check's).

.EXAMPLE
  pwsh packaging/windows/installed-check.ps1 -Arch x64 -Evidence $env:RUNNER_TEMP/installed-check
#>
param(
  [ValidateSet('x64', 'x86')] [string] $Arch = 'x64',
  [string] $Evidence = 'installed-check'
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }

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
$RuleGroup = 'open-skyfall installed check'
$firewallBefore = @()

function Invoke-Msi([string] $Verb, [string] $Log) {
  $run = Start-Process msiexec.exe -ArgumentList @($Verb, "`"$Msi`"", '/qn', '/norestart', '/L*v', "`"$Log`"") -Wait -PassThru
  if ($run.ExitCode -ne 0) {
    Get-Content $Log -Tail 60 -ErrorAction SilentlyContinue
    throw "msiexec $Verb exited $($run.ExitCode)"
  }
}

Write-Output "==> installing $(Split-Path $Msi -Leaf)"
Invoke-Msi '/i' (Join-Path $Evidence 'msi-install.log')
$failure = $null
try {
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

  Write-Output '==> taking the network away from the installed app'
  # Windows Firewall rules name a program's file. The rule kind is tried first on a copy of
  # curl.exe: a firewall that is off, or managed some other way, would let the app out too.
  $probe = Join-Path $Evidence 'network-probe.exe'
  Copy-Item (Join-Path $env:SystemRoot 'System32\curl.exe') $probe
  $blocked = @($App, $probe)
  # The web view's processes are the WebView2 runtime's, not the app's.
  $blocked += @(Get-ChildItem (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\EdgeWebView\Application\*\msedgewebview2.exe') -ErrorAction SilentlyContinue | ForEach-Object FullName)
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
  & (Join-Path $Root 'tools\windows-assistant-smoke.ps1') -Exe $App -Evidence (Join-Path $Evidence 'assistant')
} catch {
  $failure = $_
} finally {
  Get-NetFirewallRule -Group $RuleGroup -ErrorAction SilentlyContinue | Remove-NetFirewallRule
  foreach ($was in $firewallBefore) { Set-NetFirewallProfile -Name $was.Name -Enabled $was.Enabled }
  Remove-Item -Force (Join-Path $Evidence 'network-probe.exe') -ErrorAction SilentlyContinue
  Write-Output '==> uninstalling'
  # A failed check may have left the app running, and the uninstall cannot remove a running file.
  Get-Process solidcraft -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  try {
    Invoke-Msi '/x' (Join-Path $Evidence 'msi-uninstall.log')
  } catch {
    if ($failure) { Write-Output "the uninstall failed as well: $_" } else { $failure = $_ }
  }
}
if ($failure) { throw $failure }
if (Test-Path (Join-Path $Dir 'solidcraft.exe')) { throw "after the uninstall, solidcraft.exe is still in $Dir" }
Write-Output 'ok: the uninstall removed the program files'
Write-Output 'PASS: the installed app'
exit 0
