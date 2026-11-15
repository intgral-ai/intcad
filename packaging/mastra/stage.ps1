<#
.SYNOPSIS
  Stage the approved Mastra runtime for a Windows package, or fail closed.

.DESCRIPTION
  The runtime artifact is currently UNAPPROVED (#15): four shipped @mastra/core/dist/ee-* files
  carry the Mastra Enterprise License. This script requires `cargo xtask runtime-artifact
  --mode artifact` to admit an approval from packaging/mastra/policy.json before it copies
  anything. Until that approval exists it stops with UNAPPROVED_ARTIFACT, which is the legal gate,
  not a failure to work around. It never signs or publishes anything.

  The 1.0 feature matrix supports Windows x86_64 MSVC only; x86 and arm64 packages are built
  without the built-in workflow.

.EXAMPLE
  pwsh packaging/mastra/stage.ps1 -Target x86_64-pc-windows-msvc -Dest target\windows-package\x64\mastra
#>
param(
  [Parameter(Mandatory = $true)] [string] $Target,
  [Parameter(Mandatory = $true)] [string] $Dest,
  [string] $Artifact = $(if ($env:SOLIDCRAFT_MASTRA_ARTIFACT) { $env:SOLIDCRAFT_MASTRA_ARTIFACT } else { '' }),
  [string] $Inputs = $(if ($env:SOLIDCRAFT_MASTRA_INPUTS) { $env:SOLIDCRAFT_MASTRA_INPUTS } else { '' }),
  [string] $Approval = $(if ($env:SOLIDCRAFT_MASTRA_APPROVAL) { $env:SOLIDCRAFT_MASTRA_APPROVAL } else { '' }),
  [string] $Policy = $(if ($env:SOLIDCRAFT_MASTRA_POLICY) { $env:SOLIDCRAFT_MASTRA_POLICY } else { '' })
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Artifact) { $Artifact = Join-Path $Root 'target\mastra-runtime-artifact' }
if (-not $Inputs) { $Inputs = Join-Path $Root 'target\mastra-runtime-inputs' }
if (-not $Policy) { $Policy = Join-Path $Root 'packaging\mastra\policy.json' }
$TestMode = $env:SOLIDCRAFT_MASTRA_STAGE_TEST -eq '1'
$DefaultPolicy = Join-Path $Root 'packaging\mastra\policy.json'

function Fail([string] $Message) {
  Write-Error "error: $Message" -ErrorAction Continue
  exit 1
}

if (-not $TestMode) {
  if ($env:SOLIDCRAFT_RUNTIME_ARTIFACT_CHECK) {
    Fail 'SOLIDCRAFT_RUNTIME_ARTIFACT_CHECK is a test-only seam; refusing to bypass the real runtime-artifact gate'
  }
  if ($Policy -ne $DefaultPolicy) { Fail '-Policy is test-only; release packaging always uses packaging\mastra\policy.json' }
}

switch ($Target) {
  'x86_64-pc-windows-msvc' { if (-not $Approval) { $Approval = 'mastra-runtime-windows-x64-v1' } }
  'x86_64-unknown-linux-gnu' { if (-not $Approval) { $Approval = 'mastra-runtime-linux-x64-v1' } }
  'aarch64-apple-darwin' { if (-not $Approval) { $Approval = 'mastra-runtime-macos-arm64-v1' } }
  'x86_64-apple-darwin' { if (-not $Approval) { $Approval = 'mastra-runtime-macos-x64-v1' } }
  default { Fail "Mastra 1.0 does not ship for $Target" }
}
if (-not (Test-Path (Join-Path $Artifact 'runtime.json'))) {
  Fail "no runtime artifact at $Artifact; packaging fails closed until #15 records an approved artifact (set SOLIDCRAFT_MASTRA_ARTIFACT)"
}
if (-not (Test-Path $Inputs -PathType Container)) { Fail "no artifact input tree at $Inputs; set SOLIDCRAFT_MASTRA_INPUTS" }

function Get-Digest([string] $Relative) {
  $path = if ([IO.Path]::IsPathRooted($Relative)) { $Relative } else { Join-Path $Root $Relative }
  'sha256:' + (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant()
}

$Revision = (& git -C $Root rev-parse HEAD).Trim()
$Contract = Get-Digest 'schema/mastra-runtime/v1/manifest.json'
$PolicyDigest = Get-Digest $Policy
$Sources = Get-Digest 'packaging/mastra/source-records.json'
$RuntimeVersion = (Get-Content (Join-Path $Root 'packages\mastra-runtime\package.json') -Raw | ConvertFrom-Json).version
$NodeVersion = (Get-Content (Join-Path $Root '.nvmrc') -Raw).Trim()

Write-Output "==> Mastra artifact admission gate ($Target, approval $Approval)"
if ($env:SOLIDCRAFT_RUNTIME_ARTIFACT_CHECK) {
  & $env:SOLIDCRAFT_RUNTIME_ARTIFACT_CHECK --mode artifact --tree $Artifact --inputs-root $Inputs `
    --approval $Approval --target $Target --expected-revision $Revision `
    --expected-contract-digest $Contract --expected-policy-digest $PolicyDigest --expected-sources-digest $Sources `
    --advertised true
} else {
  Push-Location $Root
  try {
    & cargo xtask runtime-artifact --mode artifact --tree $Artifact --inputs-root $Inputs `
      --approval $Approval --target $Target --expected-revision $Revision `
      --expected-contract-digest $Contract --expected-policy-digest $PolicyDigest --expected-sources-digest $Sources `
      --advertised true
  } finally {
    Pop-Location
  }
}
if ($LASTEXITCODE -ne 0) {
  Fail 'the Mastra runtime artifact was not admitted; it is unapproved or does not match this revision. No package was produced.'
}

$Python = if (Get-Command python3 -ErrorAction SilentlyContinue) { 'python3' } else { 'python' }
& $Python (Join-Path $Root 'packaging\mastra\validate_runtime.py') --runtime $Artifact --target $Target `
  --approval $Approval --policy $Policy --revision $Revision --contract-digest $Contract --policy-digest $PolicyDigest --sources-digest $Sources `
  --runtime-version $RuntimeVersion --node-version $NodeVersion
if ($LASTEXITCODE -ne 0) { Fail 'the runtime tree failed packaging validation' }

# No Codex binary or credential is ever bundled: Codex is discovered from the user's install.
$forbidden = Get-ChildItem -LiteralPath $Artifact -Recurse -File | Where-Object {
  $_.Name -match '^codex(-acp)?(\.exe|\.cmd|\.bat|\.ps1)?$' -or
  ($_.FullName -match '[\\/]\.codex[\\/]' -and $_.Name -match '^(auth\.json|\.credentials\.json)$')
}
if ($forbidden) { Fail "runtime tree bundles Codex files: $(($forbidden | ForEach-Object { $_.FullName }) -join ', ')" }

Write-Output "==> staging Mastra runtime in $Dest"
if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
Copy-Item -Path (Join-Path $Artifact '*') -Destination $Dest -Recurse -Force
Write-Output 'ok Mastra runtime staged'
