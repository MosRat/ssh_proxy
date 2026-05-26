param(
    [string]$Target = "x86_64-unknown-linux-musl",
    [string]$ExtensionDir,
    [switch]$SkipBuild,
    [switch]$NoSccache
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
if (-not $ExtensionDir) {
    $ExtensionDir = Join-Path $root "apps\vscode-remote-proxy"
}
$extensionRoot = (Resolve-Path -LiteralPath $ExtensionDir).Path
$assetRoot = Join-Path $extensionRoot "assets\bin"

if (-not $SkipBuild) {
    $buildScript = Join-Path $root "scripts\build-release.ps1"
    & $buildScript -Target $Target -NoSccache:$NoSccache
}

function Copy-ExtensionBinary {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination,
        [switch]$Required
    )

    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        if ($Required) {
            throw "required ssh_proxy release binary was not found at $Source"
        }
        Write-Warning "optional ssh_proxy release binary was not found at $Source"
        return
    }

    $parent = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    Copy-Item -LiteralPath $Source -Destination $Destination -Force
    Write-Host "staged $Source -> $Destination"
}

$windowsBinary = Join-Path $root "target\release\ssh_proxy.exe"
$linuxBinary = Join-Path $root "target\$Target\release\ssh_proxy"

Copy-ExtensionBinary `
    -Source $windowsBinary `
    -Destination (Join-Path $assetRoot "win32-x64\ssh_proxy.exe") `
    -Required:($IsWindows -or $env:OS -eq "Windows_NT")

Copy-ExtensionBinary `
    -Source $linuxBinary `
    -Destination (Join-Path $assetRoot "linux-x64\ssh_proxy") `
    -Required
