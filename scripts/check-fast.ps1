param(
    [switch]$SkipRust,
    [switch]$SkipVscode,
    [switch]$InstallNodeModules,
    [switch]$NoSccache,
    [switch]$NoProcessCleanup,
    [switch]$Full,
    [switch]$Transport,
    [switch]$Contracts
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$extensionDir = Join-Path $root "apps\vscode-remote-proxy"
$oldAllowMissingSidecar = $env:SSH_PROXY_ALLOW_MISSING_SIDECAR
$oldRustcWrapper = $env:RUSTC_WRAPPER
$oldCargoIncremental = $env:CARGO_INCREMENTAL
$cargoConfigArgs = @()

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

function Test-CargoNextest {
    try {
        cargo nextest --version *> $null
        return $LASTEXITCODE -eq 0
    } catch {
        return $false
    }
}

function Invoke-CargoChecked {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    Invoke-NativeChecked $Name { cargo @cargoConfigArgs @Arguments }
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
        Stop-StaleRustTestProcesses "fast check"

        if (-not $NoSccache -and -not $env:RUSTC_WRAPPER -and (Get-Command sccache -ErrorAction SilentlyContinue)) {
            $env:RUSTC_WRAPPER = "sccache"
            $env:CARGO_INCREMENTAL = "0"
            $cargoConfigArgs = @(
                "--config", "profile.dev.incremental=false",
                "--config", "profile.test.incremental=false"
            )
            try {
                sccache --start-server *> $null
            } catch {
            }
        }

        Invoke-CargoChecked "cargo check --workspace --tests" @("check", "--workspace", "--tests")
        if ($Full) {
            if (Test-CargoNextest) {
                Invoke-CargoChecked "cargo nextest run --workspace --tests" @("nextest", "run", "--workspace", "--tests")
            } else {
                Invoke-CargoChecked "cargo test --workspace --tests" @("test", "--workspace", "--tests", "--", "--test-threads=1")
            }
        } else {
            Invoke-CargoChecked "cargo test -p ssh-proxy-protocol" @("test", "-p", "ssh-proxy-protocol")
            Invoke-CargoChecked "cargo test -p ssh-proxy-lifecycle" @("test", "-p", "ssh-proxy-lifecycle")
            Invoke-CargoChecked "cargo test -p ssh-proxy-config" @("test", "-p", "ssh-proxy-config")
            Invoke-CargoChecked "cargo test -p ssh-proxy-transport" @("test", "-p", "ssh-proxy-transport")
            Invoke-CargoChecked "cargo test -p ssh_proxy --bin ssh_proxy deploy" @("test", "-p", "ssh_proxy", "--bin", "ssh_proxy", "deploy")
            Invoke-CargoChecked "cargo test -p ssh_proxy --bin ssh_proxy remote peer config" @("test", "-p", "ssh_proxy", "--bin", "ssh_proxy", "remote_config_write")
            Invoke-CargoChecked "cargo test -p ssh_proxy --bin ssh_proxy remote resolve defaults" @("test", "-p", "ssh_proxy", "--bin", "ssh_proxy", "remote_resolve_defaults")
            Invoke-CargoChecked "cargo test -p ssh_proxy --bin ssh_proxy handoff" @("test", "-p", "ssh_proxy", "--bin", "ssh_proxy", "node_daemon::handoff")
            Invoke-CargoChecked "cargo test -p ssh_proxy --test node_daemon route smoke" @("test", "-p", "ssh_proxy", "--test", "node_daemon", "node_daemon_reuses_duplicate_route_start_for_same_spec", "--", "--test-threads=1")
            if ($Contracts) {
                Invoke-CargoChecked "cargo test -p ssh_proxy --test build_contract" @("test", "-p", "ssh_proxy", "--test", "build_contract")
                Invoke-CargoChecked "cargo test -p ssh_proxy --test cli smoke" @("test", "-p", "ssh_proxy", "--test", "cli", "cli_help_exposes_only_production_daemon_commands")
            }
            if ($Transport) {
                Invoke-CargoChecked "cargo test -p ssh_proxy --test node_daemon transport smoke" @("test", "-p", "ssh_proxy", "--test", "node_daemon", "fixed_tcp_target_can_proxy_to_specific_port", "--", "--test-threads=1")
            }
        }

        if ($env:RUSTC_WRAPPER -eq "sccache") {
            sccache --show-stats
        }
    }

    if (-not $SkipVscode) {
        if ($InstallNodeModules -or -not (Test-Path -LiteralPath (Join-Path $extensionDir "node_modules") -PathType Container)) {
            Invoke-NativeChecked "npm ci" { npm --prefix $extensionDir ci }
        }
        Invoke-NativeChecked "npm test" { npm --prefix $extensionDir test }
    }
}
finally {
    Stop-StaleRustTestProcesses "fast check cleanup"

    if ($null -eq $oldAllowMissingSidecar) {
        Remove-Item Env:\SSH_PROXY_ALLOW_MISSING_SIDECAR -ErrorAction SilentlyContinue
    } else {
        $env:SSH_PROXY_ALLOW_MISSING_SIDECAR = $oldAllowMissingSidecar
    }

    if ($null -eq $oldRustcWrapper) {
        Remove-Item Env:\RUSTC_WRAPPER -ErrorAction SilentlyContinue
    } else {
        $env:RUSTC_WRAPPER = $oldRustcWrapper
    }

    if ($null -eq $oldCargoIncremental) {
        Remove-Item Env:\CARGO_INCREMENTAL -ErrorAction SilentlyContinue
    } else {
        $env:CARGO_INCREMENTAL = $oldCargoIncremental
    }
    Pop-Location
}
