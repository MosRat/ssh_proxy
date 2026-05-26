param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $CargoArgs
)

$ErrorActionPreference = "Stop"

$sccache = Get-Command sccache -ErrorAction SilentlyContinue
if (-not $sccache) {
    Write-Error "sccache was not found on PATH. Install it with `cargo install sccache` or your package manager, then rerun this script."
}

$env:RUSTC_WRAPPER = "sccache"
sccache --start-server | Out-Null

if (-not $CargoArgs -or $CargoArgs.Count -eq 0) {
    $CargoArgs = @("check")
}

cargo @CargoArgs
$status = $LASTEXITCODE
sccache --show-stats
exit $status
