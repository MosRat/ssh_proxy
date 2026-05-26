param(
    [string]$Target = "x86_64-unknown-linux-musl",
    [switch]$NoSccache
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)

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
    if (-not $NoSccache -and (Get-Command sccache -ErrorAction SilentlyContinue)) {
        $env:RUSTC_WRAPPER = "sccache"
        Invoke-NativeChecked "sccache start-server" { sccache --start-server }
    }

    Invoke-NativeChecked "cargo zigbuild" { cargo zigbuild --target $Target --release }
    $sidecar = Join-Path $root "target\$Target\release\ssh_proxy"
    if (-not (Test-Path -LiteralPath $sidecar)) {
        throw "Linux musl sidecar was not produced at $sidecar"
    }

    $env:SSH_PROXY_LINUX_MUSL_BIN = (Resolve-Path $sidecar).Path
    Invoke-NativeChecked "cargo build" { cargo build --release }

    if ($env:RUSTC_WRAPPER -eq "sccache") {
        Invoke-NativeChecked "sccache show-stats" { sccache --show-stats }
    }
}
finally {
    Pop-Location
}
