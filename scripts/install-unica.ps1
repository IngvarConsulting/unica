param(
    [string]$Version = "latest",
    [ValidateSet("win-x64")]
    [string]$Target = "win-x64",
    [string]$MarketplaceName = "unica-local",
    [string]$CodexHome = "",
    [switch]$SkipVerify,
    [switch]$PrintDownloadUrl
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 2.0

$Repo = if ($env:UNICA_REPO) { $env:UNICA_REPO } else { "IngvarConsulting/unica" }
if ($env:UNICA_VERSION -and $Version -eq "latest") {
    $Version = $env:UNICA_VERSION
}
if ($env:UNICA_CODEX_MARKETPLACE_NAME -and $MarketplaceName -eq "unica-local") {
    $MarketplaceName = $env:UNICA_CODEX_MARKETPLACE_NAME
}
if ($env:CODEX_HOME -and [string]::IsNullOrWhiteSpace($CodexHome)) {
    $CodexHome = $env:CODEX_HOME
}

function Get-ArchiveExtension {
    param([string]$Target)
    if ($Target -eq "win-x64") {
        return "zip"
    }
    throw "Unsupported Unica release target: $Target"
}

function Get-ReleaseAssetUrl {
    param(
        [string]$Target,
        [string]$Version
    )
    $asset = "unica-codex-marketplace-$Target.$(Get-ArchiveExtension -Target $Target)"
    if ($Version -eq "latest") {
        return "https://github.com/$Repo/releases/latest/download/$asset"
    }
    return "https://github.com/$Repo/releases/download/$Version/$asset"
}

function Get-DefaultCodexHome {
    if ($env:USERPROFILE) {
        return (Join-Path $env:USERPROFILE ".codex")
    }
    if ($env:HOME) {
        return (Join-Path $env:HOME ".codex")
    }
    throw "CODEX_HOME, USERPROFILE, or HOME is required to install Unica."
}

function Invoke-DownloadFile {
    param(
        [string]$Url,
        [string]$Destination
    )
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    } catch {
        # Older hosts may already have an acceptable TLS policy; continue to the real download error if not.
    }
    try {
        Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
    } catch {
        $client = New-Object System.Net.WebClient
        try {
            $client.DownloadFile($Url, $Destination)
        } finally {
            $client.Dispose()
        }
    }
}

function Find-MarketplaceRoot {
    param([string]$Root)
    $marker = Get-ChildItem -LiteralPath $Root -Filter "marketplace.json" -Recurse |
        Where-Object {
            -not $_.PSIsContainer -and
            $_.FullName.EndsWith((Join-Path ".agents" (Join-Path "plugins" "marketplace.json")))
        } |
        Select-Object -First 1
    if (-not $marker) {
        throw "Downloaded archive does not contain .agents/plugins/marketplace.json"
    }
    return (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $marker.FullName)))
}

function Read-PluginVersion {
    param([string]$PluginJsonPath)
    $plugin = Get-Content -LiteralPath $PluginJsonPath -Raw -Encoding UTF8 | ConvertFrom-Json
    return [string]$plugin.version
}

function Enable-CodexPlugin {
    param(
        [string]$ConfigPath,
        [string]$MarketplaceName
    )
    $table = "[plugins.`"unica@$MarketplaceName`"]"
    $configDir = Split-Path -Parent $ConfigPath
    if (-not (Test-Path -LiteralPath $configDir)) {
        New-Item -ItemType Directory -Path $configDir | Out-Null
    }

    $lines = @()
    if (Test-Path -LiteralPath $ConfigPath -PathType Leaf) {
        $lines = Get-Content -LiteralPath $ConfigPath -Encoding UTF8
    }

    $out = New-Object System.Collections.Generic.List[string]
    $skip = $false
    foreach ($line in $lines) {
        if ($line -eq $table) {
            $skip = $true
            continue
        }
        if ($skip -and $line.StartsWith("[")) {
            $skip = $false
        }
        if (-not $skip) {
            $out.Add($line)
        }
    }

    if ($out.Count -gt 0 -and -not [string]::IsNullOrWhiteSpace($out[$out.Count - 1])) {
        $out.Add("")
    }
    $out.Add($table)
    $out.Add("enabled = true")
    [System.IO.File]::WriteAllText($ConfigPath, (($out -join [Environment]::NewLine) + [Environment]::NewLine), [System.Text.Encoding]::UTF8)
}

function Update-UnicaMcpJson {
    param(
        [string]$McpPath,
        [string]$Command
    )
    $mcp = Get-Content -LiteralPath $McpPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not $mcp.mcpServers -or -not $mcp.mcpServers.unica) {
        throw "MCP configuration does not contain mcpServers.unica: $McpPath"
    }
    $mcp.mcpServers.unica.command = $Command
    $mcp.mcpServers.unica.args = @()
    $json = $mcp | ConvertTo-Json -Depth 20
    [System.IO.File]::WriteAllText($McpPath, ($json + [Environment]::NewLine), [System.Text.Encoding]::UTF8)
}

function Get-ToolBinary {
    param(
        [string]$MarketplaceDir = "",
        [string]$PluginDir = "",
        [string]$Target,
        [string]$Tool
    )
    if ([string]::IsNullOrWhiteSpace($PluginDir)) {
        if ([string]::IsNullOrWhiteSpace($MarketplaceDir)) {
            throw "MarketplaceDir or PluginDir is required to locate $Tool."
        }
        $PluginDir = Join-Path $MarketplaceDir (Join-Path "plugins" "unica")
    }
    $toolPath = Join-Path $PluginDir (Join-Path "bin" (Join-Path $Target "$Tool.exe"))
    return [System.IO.Path]::GetFullPath($toolPath)
}

function Invoke-NativeChecked {
    param(
        [string]$Program,
        [string[]]$Arguments
    )
    & $Program @Arguments | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "$Program exited with code $LASTEXITCODE"
    }
}

$Url = Get-ReleaseAssetUrl -Target $Target -Version $Version
if ($PrintDownloadUrl) {
    Write-Output $Url
    exit 0
}

if ([string]::IsNullOrWhiteSpace($CodexHome)) {
    $CodexHome = Get-DefaultCodexHome
}

if (-not (Get-Command codex -ErrorAction SilentlyContinue)) {
    throw "codex CLI is required to install Unica."
}

$tmpRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("unica-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tmpRoot | Out-Null

try {
    $archive = Join-Path $tmpRoot "unica-codex-marketplace-$Target.zip"
    $extractDir = Join-Path $tmpRoot "extract"
    New-Item -ItemType Directory -Path $extractDir | Out-Null

    Write-Output "==> Unica target: $Target"
    Write-Output "==> Download: $Url"
    Invoke-DownloadFile -Url $Url -Destination $archive
    Expand-Archive -LiteralPath $archive -DestinationPath $extractDir -Force

    $extractedMarketplaceDir = Find-MarketplaceRoot -Root $extractDir
    $marketplacesRoot = Join-Path $CodexHome "marketplaces"
    $marketplaceDir = Join-Path $marketplacesRoot $MarketplaceName
    if (Test-Path -LiteralPath $marketplaceDir) {
        Remove-Item -LiteralPath $marketplaceDir -Recurse -Force
    }
    if (-not (Test-Path -LiteralPath $marketplacesRoot)) {
        New-Item -ItemType Directory -Path $marketplacesRoot | Out-Null
    }
    Copy-Item -LiteralPath $extractedMarketplaceDir -Destination $marketplaceDir -Recurse
    $marketplaceMcpPath = Join-Path $marketplaceDir (Join-Path "plugins" (Join-Path "unica" ".mcp.json"))
    Update-UnicaMcpJson -McpPath $marketplaceMcpPath -Command (Get-ToolBinary -MarketplaceDir $marketplaceDir -Target $Target -Tool "unica")

    Invoke-NativeChecked -Program (Get-ToolBinary -MarketplaceDir $marketplaceDir -Target $Target -Tool "v8-runner") -Arguments @("config", "init", "--help")
    Invoke-NativeChecked -Program (Get-ToolBinary -MarketplaceDir $marketplaceDir -Target $Target -Tool "unica") -Arguments @("--help")

    $pluginVersion = Read-PluginVersion -PluginJsonPath (Join-Path $marketplaceDir (Join-Path "plugins" (Join-Path "unica" (Join-Path ".codex-plugin" "plugin.json"))))
    $pluginCacheDir = Join-Path $CodexHome (Join-Path "plugins" (Join-Path "cache" (Join-Path $MarketplaceName "unica")))
    $pluginCacheVersionDir = Join-Path $pluginCacheDir $pluginVersion

    & codex plugin marketplace remove $MarketplaceName | Out-Null
    if (Test-Path -LiteralPath $pluginCacheDir) {
        Write-Output "==> Removing stale Codex plugin cache: $pluginCacheDir"
        Remove-Item -LiteralPath $pluginCacheDir -Recurse -Force
    }

    & codex plugin marketplace add $marketplaceDir | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "codex plugin marketplace add failed with code $LASTEXITCODE"
    }

    New-Item -ItemType Directory -Path $pluginCacheDir -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $marketplaceDir (Join-Path "plugins" "unica")) -Destination $pluginCacheVersionDir -Recurse
    $pluginCacheMcpPath = Join-Path $pluginCacheVersionDir ".mcp.json"
    Update-UnicaMcpJson -McpPath $pluginCacheMcpPath -Command (Get-ToolBinary -PluginDir $pluginCacheVersionDir -Target $Target -Tool "unica")
    Enable-CodexPlugin -ConfigPath (Join-Path $CodexHome "config.toml") -MarketplaceName $MarketplaceName

    if (-not $SkipVerify) {
        $tmpDir = Join-Path $CodexHome "tmp"
        New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
        $promptProof = Join-Path $tmpDir "unica-install-prompt-input.json"
        & codex debug prompt-input "test" | Out-File -FilePath $promptProof -Encoding UTF8
        if ($LASTEXITCODE -ne 0) {
            throw "codex debug prompt-input failed with code $LASTEXITCODE"
        }
        $promptText = Get-Content -LiteralPath $promptProof -Raw -Encoding UTF8
        foreach ($needle in @($MarketplaceName, "unica:meta-compile", "unica:v8-runner", "unica:db-auth-check")) {
            if ($promptText.IndexOf($needle, [StringComparison]::Ordinal) -lt 0) {
                throw "Codex prompt verification did not contain '$needle'. Saved prompt proof: $promptProof"
            }
        }
    }

    Write-Output "==> Installed Unica $pluginVersion in Codex as marketplace '$MarketplaceName'"
    Write-Output "==> Plugin cache: $pluginCacheVersionDir"
} finally {
    if (Test-Path -LiteralPath $tmpRoot) {
        Remove-Item -LiteralPath $tmpRoot -Recurse -Force
    }
}
