<#
.SYNOPSIS
  Put the built-in agent's folder into a package tree, after checking it.

.DESCRIPTION
  The folder (`mastra-agent`: node.exe, the agent's entry files, Mastra Code's SDK without
  Mastra's enterprise-licensed content) is built beforehand by
    node packages\mastra-runtime\scripts\agent-package.mjs build --target <triple> --out <dir>
  This script runs that script's `check` on it for the same target and copies it only when the
  check passes: whole, for this target, no enterprise content, no refused or unstated licence,
  nothing that looks like a secret or a stored sign-in. On a failure nothing is copied.

  This is not the admission gate of the Mastra workflow runtime (stage.ps1, `mastra\`): the
  agent's folder is for test packages, goes through no approval and adds none.
  See docs/mastra-packaging.md.

.PARAMETER From
  The built folder. Default: $env:SOLIDCRAFT_MASTRA_AGENT_DIR, else
  <cargo target dir>\mastra-agent\<triple>.
#>
param(
  [Parameter(Mandatory = $true)][string] $Target,
  [Parameter(Mandatory = $true)][string] $Dest,
  [string] $From = $env:SOLIDCRAFT_MASTRA_AGENT_DIR
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $From) {
  $TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
  $From = Join-Path $TargetDir "mastra-agent\$Target"
}
if (-not (Test-Path -PathType Container $From)) {
  throw "no built agent folder at $From; build it first: node packages\mastra-runtime\scripts\agent-package.mjs build --target $Target --out $From"
}
if (-not (Get-Command node -ErrorAction SilentlyContinue)) { throw 'node is needed to check the agent folder' }

Write-Output "==> checking the built-in agent's folder ($Target)"
& node (Join-Path $Root 'packages\mastra-runtime\scripts\agent-package.mjs') check --target $Target $From
if ($LASTEXITCODE -ne 0) { throw "the built-in agent's folder did not pass its check; nothing was staged" }

Write-Output "==> staging the built-in agent in $Dest"
if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Dest) | Out-Null
Copy-Item -Recurse $From $Dest
Write-Output "ok built-in agent staged in $Dest"
