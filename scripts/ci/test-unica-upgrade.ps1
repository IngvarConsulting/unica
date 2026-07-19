param(
    [Parameter(Mandatory = $true)]
    [string]$CodexPath,
    [Parameter(Mandatory = $true)]
    [string]$LegacyMarketplaceRoot,
    [Parameter(Mandatory = $true)]
    [string]$CandidatePluginRoot,
    [Parameter(Mandatory = $true)]
    [string]$ReportPath,
    [ValidateSet("unica-local", "unica")]
    [string]$LegacyManagedName = "unica-local",
    [ValidateSet("Preflight", "Full")]
    [string]$Mode = "Preflight"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 2.0
$ExpectedCodexVersion = "codex-cli 0.145.0-alpha.18"
$ExpectedLegacyVersion = "0.6.1"

function Resolve-RequiredPath {
    param([string]$Path, [string]$Label, [switch]$Directory)
    $expectedType = if ($Directory) { "Container" } else { "Leaf" }
    if (-not (Test-Path -LiteralPath $Path -PathType $expectedType)) {
        throw "$Label is missing: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    $output = & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program exited with code $LASTEXITCODE"
    }
    return ($output -join [Environment]::NewLine)
}

function Read-JsonOutput {
    param([string]$Label, [string]$Output)
    try {
        return $Output | ConvertFrom-Json
    } catch {
        throw "$Label returned invalid JSON: $($_.Exception.Message)"
    }
}

$resolvedCodex = Resolve-RequiredPath -Path $CodexPath -Label "Codex CLI"
$legacySource = Resolve-RequiredPath -Path $LegacyMarketplaceRoot -Label "legacy marketplace" -Directory
$candidateRoot = Resolve-RequiredPath -Path $CandidatePluginRoot -Label "candidate plugin" -Directory
$candidateManifestPath = Join-Path $candidateRoot ".codex-plugin\plugin.json"
$candidateManifestPath = Resolve-RequiredPath -Path $candidateManifestPath -Label "candidate plugin manifest"
$candidateManifest = Get-Content -LiteralPath $candidateManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
$candidateVersion = [string]$candidateManifest.version
if ([string]::IsNullOrWhiteSpace($candidateVersion)) {
    throw "candidate plugin manifest has no version"
}
$bootstrap = Join-Path $candidateRoot "bootstrap\bin\win-x64\unica-bootstrap.exe"
$bootstrap = Resolve-RequiredPath -Path $bootstrap -Label "candidate Windows bootstrap"

$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("unica-legacy-upgrade-" + [Guid]::NewGuid().ToString("N"))
$codexHome = Join-Path $testRoot "codex-home"
$codexBinRoot = Split-Path -Parent $resolvedCodex
$codexCommand = $resolvedCodex
$legacyParent = Join-Path $codexHome "marketplaces"
$legacyManagedRoot = Join-Path $legacyParent $LegacyManagedName
$originalCodexHome = $env:CODEX_HOME
$originalPath = $env:PATH

try {
    New-Item -ItemType Directory -Path $codexHome, $legacyParent | Out-Null
    Copy-Item -LiteralPath $legacySource -Destination $legacyManagedRoot -Recurse
    $env:CODEX_HOME = $codexHome
    $env:PATH = "$codexBinRoot;$originalPath"

    $codexVersion = Invoke-Checked -Program $codexCommand -Arguments @("--version")
    if ($codexVersion.Trim() -ne $ExpectedCodexVersion) {
        throw "unexpected Codex CLI version: $($codexVersion.Trim())"
    }

    Invoke-Checked -Program $codexCommand -Arguments @(
        "plugin", "marketplace", "add", $legacyManagedRoot, "--json"
    ) | Out-Null
    Invoke-Checked -Program $codexCommand -Arguments @(
        "plugin", "add", "unica@unica", "--json"
    ) | Out-Null

    $legacyPlugins = Read-JsonOutput -Label "legacy plugin discovery" -Output (
        Invoke-Checked -Program $codexCommand -Arguments @(
            "plugin", "list", "--available", "--json"
        )
    )
    $legacyPlugin = @($legacyPlugins.installed | Where-Object { $_.pluginId -eq "unica@unica" })
    if ($legacyPlugin.Count -ne 1 -or [string]$legacyPlugin[0].version -ne $ExpectedLegacyVersion) {
        throw "failed to seed exactly one installed Unica $ExpectedLegacyVersion"
    }

    $preflight = Read-JsonOutput -Label "candidate migration preflight" -Output (
        Invoke-Checked -Program $bootstrap -Arguments @(
            "migrate-preflight", "--plugin-root", $candidateRoot
        )
    )
    if (@($preflight.removePluginIds) -notcontains "unica@unica" -or
        @($preflight.removeMarketplaces) -notcontains "unica" -or
        -not [bool]$preflight.addCanonicalMarketplace -or
        -not [bool]$preflight.installCanonicalPlugin) {
        throw "candidate preflight did not classify the legacy Unica installation"
    }

    $migrated = $false
    $idempotent = $false
    if ($Mode -eq "Full") {
        $migration = Read-JsonOutput -Label "candidate migration" -Output (
            Invoke-Checked -Program $bootstrap -Arguments @(
                "migrate", "--plugin-root", $candidateRoot
            )
        )
        if (-not [bool]$migration.changed) {
            throw "candidate migration reported no change for a legacy installation"
        }
        $migrated = $true

        $currentPlugins = Read-JsonOutput -Label "current plugin discovery" -Output (
            Invoke-Checked -Program $codexCommand -Arguments @(
                "plugin", "list", "--available", "--json"
            )
        )
        $currentPlugin = @($currentPlugins.installed | Where-Object { $_.pluginId -eq "unica@unica" })
        if ($currentPlugin.Count -ne 1 -or
            [string]$currentPlugin[0].version -ne $candidateVersion -or
            -not [bool]$currentPlugin[0].enabled) {
            throw "migration did not install exactly one enabled Unica $candidateVersion"
        }

        $secondPreflight = Read-JsonOutput -Label "idempotent migration preflight" -Output (
            Invoke-Checked -Program $bootstrap -Arguments @(
                "migrate-preflight", "--plugin-root", $candidateRoot
            )
        )
        $idempotent = @($secondPreflight.removePluginIds).Count -eq 0 -and
            @($secondPreflight.removeMarketplaces).Count -eq 0 -and
            -not [bool]$secondPreflight.addCanonicalMarketplace -and
            -not [bool]$secondPreflight.upgradeCanonicalMarketplace -and
            -not [bool]$secondPreflight.installCanonicalPlugin
        if (-not $idempotent) {
            throw "migration is not idempotent"
        }
    }

    $report = [ordered]@{
        schemaVersion = 1
        mode = $Mode
        codexVersion = $ExpectedCodexVersion
        legacyVersion = $ExpectedLegacyVersion
        legacyManagedName = $LegacyManagedName
        candidateVersion = $candidateVersion
        preflightClassified = $true
        changed = $migrated
        idempotent = $idempotent
    }
    $reportDirectory = Split-Path -Parent $ReportPath
    if ($reportDirectory) {
        New-Item -ItemType Directory -Force -Path $reportDirectory | Out-Null
    }
    $report | ConvertTo-Json | Set-Content -LiteralPath $ReportPath -Encoding UTF8
    $report | ConvertTo-Json
} finally {
    $env:CODEX_HOME = $originalCodexHome
    $env:PATH = $originalPath
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
