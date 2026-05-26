param(
    [string]$Target = "x86_64-unknown-linux-musl",
    [switch]$SkipBuild,
    [switch]$NoSccache,
    [switch]$InstallNodeModules
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$extensionDir = Join-Path $root "apps\vscode-remote-proxy"

function Invoke-NativeChecked {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$Command
    )

    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Name failed with exit code $LASTEXITCODE"
    }
}

if ($SkipBuild) {
    & (Join-Path $root "scripts\stage-vscode-extension-binaries.ps1") -Target $Target -SkipBuild
} else {
    & (Join-Path $root "scripts\stage-vscode-extension-binaries.ps1") -Target $Target -NoSccache:$NoSccache
}

if ($InstallNodeModules -or -not (Test-Path -LiteralPath (Join-Path $extensionDir "node_modules") -PathType Container)) {
    Invoke-NativeChecked "npm ci" { npm --prefix $extensionDir ci }
}

Invoke-NativeChecked "npm package" { npm --prefix $extensionDir run package }
