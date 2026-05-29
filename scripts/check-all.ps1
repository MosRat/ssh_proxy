param(
    [switch]$SkipRust,
    [switch]$SkipVscode,
    [switch]$SkipFmt,
    [switch]$InstallNodeModules,
    [switch]$NoProcessCleanup
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

function Stop-StaleRustTestProcesses {
    param([string]$Reason)

    if ($NoProcessCleanup -or $env:OS -ne "Windows_NT") {
        return
    }

    $debugBinary = [System.IO.Path]::GetFullPath((Join-Path $root "target\debug\ssh_proxy.exe"))
    $processes = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Name -eq "ssh_proxy.exe" -and
            $_.ExecutablePath -and
            ([System.IO.Path]::GetFullPath($_.ExecutablePath) -ieq $debugBinary)
        }
    foreach ($process in $processes) {
        Write-Host "Stopping stale Rust test process $($process.ProcessId) before $Reason"
        Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
    }
}

Push-Location $root
try {
    if (-not $SkipRust) {
        $env:SSH_PROXY_ALLOW_MISSING_SIDECAR = if ($null -ne $oldAllowMissingSidecar) { $oldAllowMissingSidecar } else { "1" }
        Stop-StaleRustTestProcesses "full check"

        if (-not $SkipFmt) {
            Invoke-NativeChecked "cargo fmt" { cargo fmt -- --check }
        }
        Invoke-NativeChecked "cargo check" { cargo check --workspace }
        Invoke-NativeChecked "cargo test" { cargo test --workspace --tests }
    }

    if (-not $SkipVscode) {
        if ($InstallNodeModules -or -not (Test-Path -LiteralPath (Join-Path $extensionDir "node_modules") -PathType Container)) {
            Invoke-NativeChecked "npm ci" { npm --prefix $extensionDir ci }
        }
        Invoke-NativeChecked "npm test" { npm --prefix $extensionDir test }
    }
}
finally {
    Stop-StaleRustTestProcesses "full check cleanup"

    if ($null -eq $oldAllowMissingSidecar) {
        Remove-Item Env:\SSH_PROXY_ALLOW_MISSING_SIDECAR -ErrorAction SilentlyContinue
    } else {
        $env:SSH_PROXY_ALLOW_MISSING_SIDECAR = $oldAllowMissingSidecar
    }
    Pop-Location
}
