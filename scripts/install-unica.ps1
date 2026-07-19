param(
    [string]$Ref = "main",
    [ValidateSet("win-x64")]
    [string]$Target = "win-x64",
    [string]$CodexHome = "",
    [switch]$Help
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 2.0
$MarketplaceRepository = "https://github.com/IngvarConsulting/unica-marketplace.git"

if ($env:UNICA_MARKETPLACE_REF -and $Ref -eq "main") {
    $Ref = $env:UNICA_MARKETPLACE_REF
}
if ($env:CODEX_HOME -and [string]::IsNullOrWhiteSpace($CodexHome)) {
    $CodexHome = $env:CODEX_HOME
}
if ($Help) {
    Write-Output "Migrate Unica through IngvarConsulting/unica-marketplace using Git and a native transactional bootstrap."
    Write-Output "The migration backup is retained and printed for rollback diagnostics."
    exit 0
}

if ($Ref -notmatch '^[A-Za-z0-9._/-]+$') {
    throw "Unsafe marketplace ref: $Ref"
}
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw "Git is required to install or migrate Unica."
}
if (-not (Get-Command codex -ErrorAction SilentlyContinue)) {
    throw "Codex CLI is required to install or migrate Unica."
}

& git -c 'alias.unica-probe=!f() { exit 0; }; f' unica-probe
if ($LASTEXITCODE -ne 0) {
    throw "Git shell alias probe failed with code $LASTEXITCODE"
}

if ([string]::IsNullOrWhiteSpace($CodexHome)) {
    if ($env:USERPROFILE) {
        $CodexHome = Join-Path $env:USERPROFILE ".codex"
    } elseif ($env:HOME) {
        $CodexHome = Join-Path $env:HOME ".codex"
    } else {
        throw "CODEX_HOME, USERPROFILE, or HOME is required."
    }
}
$env:CODEX_HOME = $CodexHome

function Invoke-Checked {
    param(
        [string]$Program,
        [string[]]$Arguments
    )
    $output = & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program exited with code $LASTEXITCODE"
    }
    return $output
}

$tmpRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("unica-migrate-" + [Guid]::NewGuid().ToString("N"))
$marketplaceDir = Join-Path $tmpRoot "marketplace"
New-Item -ItemType Directory -Path $tmpRoot | Out-Null

try {
    Invoke-Checked -Program "git" -Arguments @("clone", "--depth", "1", "--branch", $Ref, $MarketplaceRepository, $marketplaceDir) | Out-Null
    $catalogPath = Join-Path $marketplaceDir (Join-Path ".agents" (Join-Path "plugins" "marketplace.json"))
    if (-not (Test-Path -LiteralPath $catalogPath -PathType Leaf)) {
        throw "Stable marketplace catalog is missing: $catalogPath"
    }
    $catalog = Get-Content -LiteralPath $catalogPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($catalog.plugins.Count -ne 1) {
        throw "Stable marketplace must contain exactly one plugin."
    }
    $source = $catalog.plugins[0].source
    if ($source.source -ne "git-subdir" -or $source.url -ne $MarketplaceRepository -or $source.path -ne "./plugins/unica") {
        throw "Stable marketplace has an unexpected Unica source."
    }
    $pinnedRef = [string]$source.ref
    if ($pinnedRef -notmatch '^v[0-9A-Za-z._-]+$') {
        throw "Marketplace catalog does not contain an immutable Unica tag."
    }

    Invoke-Checked -Program "git" -Arguments @("-C", $marketplaceDir, "fetch", "--depth", "1", "origin", "refs/tags/$pinnedRef`:refs/tags/$pinnedRef") | Out-Null
    Invoke-Checked -Program "git" -Arguments @("-C", $marketplaceDir, "checkout", "--detach", $pinnedRef) | Out-Null

    $pluginRoot = Join-Path $marketplaceDir (Join-Path "plugins" "unica")
    $bootstrap = Join-Path $pluginRoot (Join-Path "bootstrap" (Join-Path "bin" (Join-Path $Target "unica-bootstrap.exe")))
    if (-not (Test-Path -LiteralPath $bootstrap -PathType Leaf)) {
        throw "Native Unica bootstrap is missing: $bootstrap"
    }

    Write-Output "==> Preflight Unica migration from $pinnedRef"
    Invoke-Checked -Program $bootstrap -Arguments @("migrate-preflight", "--plugin-root", $pluginRoot) | Write-Output
    Write-Output "==> Apply transactional Unica migration"
    $migrationOutput = (Invoke-Checked -Program $bootstrap -Arguments @("migrate", "--plugin-root", $pluginRoot)) -join [Environment]::NewLine
    Write-Output $migrationOutput
    $report = $migrationOutput | ConvertFrom-Json
    if ($report.backupDir) {
        Write-Output "==> Migration backup: $($report.backupDir)"
    } else {
        Write-Output "==> Migration backup: not required (already canonical)"
    }
} finally {
    if (Test-Path -LiteralPath $tmpRoot) {
        Remove-Item -LiteralPath $tmpRoot -Recurse -Force
    }
}
