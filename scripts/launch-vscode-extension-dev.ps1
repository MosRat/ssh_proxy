param(
    [string]$ExtensionDir,
    [string]$CodeCommand = "code",
    [string]$OpenPath,
    [switch]$IsolatedProfile
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
if (-not $ExtensionDir) {
    $ExtensionDir = Join-Path $root "apps\vscode-remote-proxy"
}

$extensionRoot = (Resolve-Path -LiteralPath $ExtensionDir).Path
$code = Get-Command $CodeCommand -ErrorAction Stop
$arguments = @(
    "--new-window",
    "--extensionDevelopmentPath=$extensionRoot"
)

if ($IsolatedProfile) {
    $profileRoot = Join-Path $root "target\vscode-extension-dev"
    $userDataDir = Join-Path $profileRoot "user-data"
    $extensionsDir = Join-Path $profileRoot "extensions"
    New-Item -ItemType Directory -Force -Path $userDataDir, $extensionsDir | Out-Null
    $arguments += @(
        "--user-data-dir=$userDataDir",
        "--extensions-dir=$extensionsDir"
    )
}

if ($OpenPath) {
    $arguments += $OpenPath
}

Write-Host "Launching VS Code Extension Development Host"
Write-Host "extensionDevelopmentPath=$extensionRoot"
Start-Process -FilePath $code.Source -ArgumentList $arguments
