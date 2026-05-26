param(
    [switch]$SkipRust,
    [switch]$SkipVscode,
    [switch]$SkipFmt,
    [switch]$InstallNodeModules
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$extensionDir = Join-Path $root "apps\vscode-remote-proxy"
$oldAllowMissingSidecar = $env:SSH_PROXY_ALLOW_MISSING_SIDECAR

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

Push-Location $root
try {
    if (-not $SkipRust) {
        $env:SSH_PROXY_ALLOW_MISSING_SIDECAR = if ($null -ne $oldAllowMissingSidecar) { $oldAllowMissingSidecar } else { "1" }

        if (-not $SkipFmt) {
            Invoke-NativeChecked "cargo fmt" { cargo fmt -- --check }
        }
        Invoke-NativeChecked "cargo check" { cargo check }
        Invoke-NativeChecked "cargo test" { cargo test --tests }
    }

    if (-not $SkipVscode) {
        if ($InstallNodeModules -or -not (Test-Path -LiteralPath (Join-Path $extensionDir "node_modules") -PathType Container)) {
            Invoke-NativeChecked "npm ci" { npm --prefix $extensionDir ci }
        }
        Invoke-NativeChecked "npm test" { npm --prefix $extensionDir test }
    }
}
finally {
    if ($null -eq $oldAllowMissingSidecar) {
        Remove-Item Env:\SSH_PROXY_ALLOW_MISSING_SIDECAR -ErrorAction SilentlyContinue
    } else {
        $env:SSH_PROXY_ALLOW_MISSING_SIDECAR = $oldAllowMissingSidecar
    }
    Pop-Location
}
