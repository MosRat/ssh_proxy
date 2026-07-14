param(
    [string]$Target = "x86_64-unknown-linux-musl",
    [switch]$FullRust,
    [switch]$SkipRustChecks,
    [switch]$SkipVscodeTests,
    [switch]$SkipPackage,
    [switch]$InstallNodeModules,
    [switch]$NoSccache,
    [switch]$CleanLocalState,
    [switch]$CleanProgramData,
    [switch]$LaunchVscode,
    [switch]$IsolatedVscodeProfile,
    [string]$CodeCommand = "code"
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$extensionDir = Join-Path $root "apps\vscode-remote-proxy"
$releaseBin = Join-Path $root "target\release\ssh_proxy.exe"
if ($env:OS -ne "Windows_NT") {
    $releaseBin = Join-Path $root "target\release\ssh_proxy"
}

$oldRustcWrapper = $env:RUSTC_WRAPPER
$oldCargoIncremental = $env:CARGO_INCREMENTAL
$oldLinuxMuslBin = $env:SSH_PROXY_LINUX_MUSL_BIN

function Invoke-NativeChecked {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$Command
    )

    Write-Host "==> $Name"
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Name failed with exit code $LASTEXITCODE"
    }
}

function Invoke-NativeBestEffort {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$Command
    )

    Write-Host "==> $Name"
    try {
        & $Command
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "$Name exited with code $LASTEXITCODE"
        }
    } catch {
        Write-Warning "$Name failed: $($_.Exception.Message)"
    }
}

function Enable-SccacheIfAvailable {
    if ($NoSccache -or $env:RUSTC_WRAPPER -or -not (Get-Command sccache -ErrorAction SilentlyContinue)) {
        return
    }
    $env:RUSTC_WRAPPER = "sccache"
    $env:CARGO_INCREMENTAL = "0"
    Invoke-NativeBestEffort "sccache start-server" { sccache --start-server }
}

function Stop-RepoSshProxyProcesses {
    if ($env:OS -ne "Windows_NT") {
        return
    }
    $rootFull = [System.IO.Path]::GetFullPath($root).TrimEnd('\')
    $processes = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Name -eq "ssh_proxy.exe" -and
            $_.ExecutablePath -and
            [System.IO.Path]::GetFullPath($_.ExecutablePath).StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)
        }
    foreach ($process in $processes) {
        Write-Host "Stopping repo ssh_proxy process $($process.ProcessId) at $($process.ExecutablePath)"
        Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
    }
}

function Remove-PathIfPresent {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (Test-Path -LiteralPath $Path) {
        Write-Host "Removing $Path"
        Remove-Item -LiteralPath $Path -Recurse -Force
    }
}

function Invoke-LocalStateCleanup {
    Stop-RepoSshProxyProcesses
    if (Test-Path -LiteralPath $releaseBin -PathType Leaf) {
        foreach ($scope in @("user", "system")) {
            Invoke-NativeBestEffort "daemon stop --scope $scope" {
                & $releaseBin daemon --scope $scope --json stop
            }
            Invoke-NativeBestEffort "daemon uninstall --scope $scope" {
                & $releaseBin daemon --scope $scope --json uninstall
            }
        }
    } else {
        Write-Warning "release binary not found at $releaseBin; daemon stop/uninstall cleanup skipped"
    }

    $userProfile = [Environment]::GetFolderPath("UserProfile")
    if ($userProfile) {
        Remove-PathIfPresent -Path (Join-Path $userProfile ".ssh_proxy")
    }
    if ($env:LOCALAPPDATA) {
        Remove-PathIfPresent -Path (Join-Path $env:LOCALAPPDATA "ssh_proxy")
    }
    Remove-PathIfPresent -Path (Join-Path $root "target\vscode-extension-dev")

    if ($CleanProgramData -and $env:ProgramData) {
        Remove-PathIfPresent -Path (Join-Path $env:ProgramData "ssh_proxy")
    }
}

Push-Location $root
try {
    Enable-SccacheIfAvailable

    if ($CleanLocalState) {
        Invoke-LocalStateCleanup
    }

    if (-not $SkipRustChecks) {
        Invoke-NativeChecked "cargo fmt" { cargo fmt --all -- --check }
    }

    Invoke-NativeChecked "release build" {
        & (Join-Path $root "scripts\build-release.ps1") -Target $Target -NoSccache:$NoSccache
    }

    if (-not $SkipRustChecks) {
        Invoke-NativeChecked "build contract" { cargo test -p ssh_proxy --test build_contract }
        Invoke-NativeChecked "workspace test check" { cargo check --workspace --tests }
        if ($FullRust) {
            Invoke-NativeChecked "workspace tests" { cargo test --workspace --tests -- --test-threads=1 }
        }
    }

    Invoke-NativeChecked "stage extension binaries" {
        & (Join-Path $root "scripts\stage-vscode-extension-binaries.ps1") -Target $Target -SkipBuild
    }

    if (-not $SkipVscodeTests) {
        if ($InstallNodeModules -or -not (Test-Path -LiteralPath (Join-Path $extensionDir "node_modules") -PathType Container)) {
            Invoke-NativeChecked "npm ci" { npm --prefix $extensionDir ci }
        }
        Invoke-NativeChecked "extension tests" { npm --prefix $extensionDir test }
    }

    if (-not $SkipPackage) {
        Invoke-NativeChecked "extension package" {
            & (Join-Path $root "scripts\package-vscode-extension.ps1") -Target $Target -SkipBuild -InstallNodeModules:$InstallNodeModules
        }
    }

    if ($env:RUSTC_WRAPPER -eq "sccache" -and (Get-Command sccache -ErrorAction SilentlyContinue)) {
        Invoke-NativeBestEffort "sccache stats" { sccache --show-stats }
    }

    if ($LaunchVscode) {
        & (Join-Path $root "scripts\launch-vscode-extension-dev.ps1") `
            -CodeCommand $CodeCommand `
            -IsolatedProfile:$IsolatedVscodeProfile
    }
}
finally {
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
    if ($null -eq $oldLinuxMuslBin) {
        Remove-Item Env:\SSH_PROXY_LINUX_MUSL_BIN -ErrorAction SilentlyContinue
    } else {
        $env:SSH_PROXY_LINUX_MUSL_BIN = $oldLinuxMuslBin
    }
    Pop-Location
}
