<#
.SYNOPSIS
  Verify and stage the unmodified Microsoft Visual C++ runtime files that the built-in
  Agent's native modules load on Windows x64.

.DESCRIPTION
  The release carries the runtime beside `mastra-agent\node.exe` (application-local), so a clean
  Windows install does not need a machine-wide Visual C++ Redistributable. The files are
  redistributable only as the unmodified Microsoft files covered by the recorded licence basis;
  `packaging\windows\vc-runtime-compliance.md` records that basis and the responsibilities.

  This script never downloads, guesses or repairs a file. It reads
  `packaging\windows\vc-runtime-files.json`, finds each file under the licensed Visual Studio
  `VC\Redist` tree (or the exact runtime folder), and checks, before anything is copied:

    - the receipt's schema, file list and source are complete;
    - the file's SHA-256, PE machine (AMD64), exact file version and product version;
    - a Valid Authenticode signature by Microsoft Corporation.

  Any mismatch fails the build with the file named. There is no fallback to System32, a hosted
  image's installed runtime, a previous package or an unrelated product's folder.

  `-Action Verify` checks an already-staged folder (the installed MSI or an extracted ZIP) against
  the same receipt, without copying.

.PARAMETER Action
  Stage: verify the licensed files, then copy them beside the Agent's Node.
  Verify: check a staged folder against the receipt.

.PARAMETER Receipt
  The checked-in receipt. Default: packaging\windows\vc-runtime-files.json.

.PARAMETER RedistRoot
  The Visual Studio `VC\Redist\MSVC` folder, its `[version]\x64` folder, or the exact
  `Microsoft.VC###.CRT` folder. Default: resolved from the newest Visual Studio installation with
  `vswhere`; the build fails rather than picking another runtime when no installation answers.

.PARAMETER Destination
  Stage: the Agent folder to receive the files. Verify: the folder that should hold them.

.PARAMETER Evidence
  Optional folder for a JSON record of what was verified (public hashes and versions only).

.EXAMPLE
  ./packaging/windows/vc-runtime.ps1 -Action Stage -Destination stage/mastra-agent -Evidence evidence
#>
param(
  [Parameter(Mandatory = $true)][ValidateSet('Stage', 'Verify')][string] $Action,
  [string] $Receipt = (Join-Path $PSScriptRoot 'vc-runtime-files.json'),
  [string] $RedistRoot,
  [Parameter(Mandatory = $true)][string] $Destination,
  [string] $Evidence
)
$ErrorActionPreference = 'Stop'

function Fail([string] $message) { throw "Visual C++ runtime: $message" }

function Read-Receipt {
  if (-not (Test-Path -LiteralPath $Receipt)) { Fail "no receipt at $Receipt" }
  $data = Get-Content -LiteralPath $Receipt -Raw | ConvertFrom-Json
  if ($data.schema -ne 1) { Fail "unsupported receipt schema '$($data.schema)'" }
  if (-not $data.source -or -not $data.source.channel -or -not $data.source.installer) {
    Fail 'the receipt does not name its licensed source channel'
  }
  if (-not $data.files -or @($data.files).Count -eq 0) { Fail 'the receipt lists no files' }
  foreach ($file in @($data.files)) {
    foreach ($field in 'name', 'version', 'productVersion', 'machine', 'sha256', 'usedBy') {
      if (-not $file.$field) { Fail "receipt entry '$($file.name)' has no $field" }
    }
    if ($file.machine -ne 'AMD64') { Fail "receipt entry '$($file.name)' is not for AMD64" }
    if ($file.sha256 -notmatch '^[0-9a-f]{64}$') { Fail "receipt entry '$($file.name)' has no lowercase SHA-256" }
    if (-not $file.redistributable) { Fail "receipt entry '$($file.name)' is not marked redistributable" }
    if (-not $file.targets -or @($file.targets).Count -eq 0) { Fail "receipt entry '$($file.name)' has no target" }
  }
  return $data
}

function Resolve-RedistRoot([string] $root) {
  if (-not $root) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { Fail 'no vswhere; pass -RedistRoot with the licensed VC\Redist path' }
    $installation = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $installation) { Fail 'no Visual Studio installation with the C++ toolset; pass -RedistRoot' }
    $root = Join-Path $installation 'VC\Redist\MSVC'
  }
  if (-not (Test-Path -LiteralPath $root)) { Fail "the redistribution root does not exist: $root" }
  return $root
}

# The receipt names the exact runtime folder under the licensed VC\Redist tree. The build may be
# given that folder, its [version]\x64 parent, or the VC\Redist\MSVC root.
function Find-File([string] $root, $file) {
  $relative = $file.sourcePath -replace '/', '\'
  foreach ($candidate in @((Join-Path $root $relative), (Join-Path (Join-Path $root $file.redistVersion) $relative), (Join-Path (Join-Path (Join-Path $root $file.redistVersion) 'x64') $relative))) {
    if (Test-Path -LiteralPath $candidate -PathType Leaf) { return (Resolve-Path -LiteralPath $candidate).Path }
  }
  # A folder already given as ...\Microsoft.VC###.CRT.
  $direct = Join-Path $root $file.name
  if (Test-Path -LiteralPath $direct -PathType Leaf) { return (Resolve-Path -LiteralPath $direct).Path }
  Fail "the licensed VC\Redist tree does not contain $($file.name) (expected under $root\$($file.redistVersion)\x64\$($file.sourcePath))"
}

function Get-PeMachine([string] $path) {
  $stream = [IO.File]::OpenRead($path)
  try {
    $reader = New-Object IO.BinaryReader($stream)
    if ($reader.ReadUInt16() -ne 0x5A4D) { Fail "$path is not a PE file" }
    $stream.Position = 0x3C
    $offset = $reader.ReadInt32()
    $stream.Position = $offset
    if ($reader.ReadUInt32() -ne 0x00004550) { Fail "$path is not a PE file" }
    $machine = $reader.ReadUInt16()
  } finally { $stream.Dispose() }
  switch ($machine) {
    0x8664 { return 'AMD64' }
    0xAA64 { return 'ARM64' }
    0x014c { return 'I386' }
    default { return ('UNKNOWN-0x{0:X4}' -f $machine) }
  }
}

function Test-File([string] $path, $file) {
  $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($hash -ne $file.sha256) { Fail "$($file.name) SHA-256 is $hash, not the receipt's $($file.sha256)" }
  $machine = Get-PeMachine $path
  if ($machine -ne $file.machine) { Fail "$($file.name) is $machine, not $($file.machine)" }
  $info = (Get-Item -LiteralPath $path).VersionInfo
  if ($info.FileVersion -ne $file.version) { Fail "$($file.name) file version is $($info.FileVersion), not $($file.version)" }
  if ($info.ProductVersion -ne $file.productVersion) { Fail "$($file.name) product version is $($info.ProductVersion), not $($file.productVersion)" }
  $signature = Get-AuthenticodeSignature -LiteralPath $path
  if ($signature.Status -ne 'Valid') { Fail "$($file.name) has no Valid Authenticode signature ($($signature.Status))" }
  if ($signature.SignerCertificate.Subject -notmatch 'CN=Microsoft (Corporation|Windows Software Compatibility Publisher)(,|$)') {
    Fail "$($file.name) is signed by $($signature.SignerCertificate.Subject), not Microsoft Corporation"
  }
  return [ordered]@{ name = $file.name; sha256 = $hash; version = $info.FileVersion; productVersion = $info.ProductVersion; machine = $machine; signer = $signature.SignerCertificate.Subject }
}

$receiptData = Read-Receipt
$verified = @()
if ($Action -eq 'Stage') {
  $root = Resolve-RedistRoot $RedistRoot
  if (-not (Test-Path -LiteralPath $Destination)) { Fail "no Agent folder to stage into: $Destination" }
  # Check every file before the first copy, so a mismatch leaves the stage untouched.
  $sources = @()
  foreach ($file in @($receiptData.files)) {
    $path = Find-File $root $file
    $record = Test-File $path $file
    $record['source'] = $path
    $record['usedBy'] = $file.usedBy
    $sources += , @($path, $file, $record)
  }
  foreach ($pair in $sources) {
    $source, $file, $record = $pair
    $targets = @()
    foreach ($relative in @($file.targets)) {
      $within = $relative -replace '^mastra-agent/', ''
      $target = Join-Path $Destination ($within -replace '/', '\')
      $targetDirectory = Split-Path -Parent $target
      if (-not (Test-Path -LiteralPath $targetDirectory)) { New-Item -ItemType Directory -Force -Path $targetDirectory | Out-Null }
      Copy-Item -LiteralPath $source -Destination $target -Force
      $targets += $target
    }
    $record['targets'] = $targets
    $verified += $record
    Write-Output "staged $($file.name) $($record.version) $($record.sha256) beside $($targets.Count) module(s)"
  }
  Copy-Item -LiteralPath $Receipt -Destination (Join-Path $Destination 'vc-runtime-receipt.json') -Force
  # The third-party notice and the end-user terms travel with the files they describe.
  $licenses = Join-Path $Destination 'licenses'
  if (-not (Test-Path -LiteralPath $licenses)) { New-Item -ItemType Directory -Force -Path $licenses | Out-Null }
  Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'VC-RUNTIME-NOTICE.txt') -Destination (Join-Path $licenses 'VC-RUNTIME-NOTICE.txt') -Force
  Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'VC-RUNTIME-TERMS.txt') -Destination (Join-Path $Destination 'VC-RUNTIME-TERMS.txt') -Force
} else {
  foreach ($file in @($receiptData.files)) {
    $targets = @()
    foreach ($relative in @($file.targets)) {
      $within = $relative -replace '^mastra-agent/', ''
      $path = Join-Path $Destination ($within -replace '/', '\')
      if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { Fail "the package does not carry $($file.name) at $relative" }
      $record = Test-File $path $file
      $targets += $path
    }
    $record['targets'] = $targets
    $record['usedBy'] = $file.usedBy
    $verified += $record
    Write-Output "verified $($file.name) $($record.version) $($record.sha256) at $($targets.Count) place(s)"
  }
}
if ($Evidence) {
  New-Item -ItemType Directory -Force -Path $Evidence | Out-Null
  $record = [ordered]@{
    schema = 1
    source = $receiptData.source
    files = $verified
    checkedAt = [DateTime]::UtcNow.ToString('o')
  }
  $record | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $Evidence "vc-runtime-$($Action.ToLowerInvariant()).json") -Encoding utf8
}
Write-Output "ok: $($verified.Count) Visual C++ runtime file(s) checked: $Action"
