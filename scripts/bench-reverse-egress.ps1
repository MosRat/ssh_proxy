param(
    [string[]]$Targets = @(),
    [string]$Url = "http://cachefly.cachefly.net/100mb.test",
    [string]$ReadinessUrl = "https://www.google.com/generate_204",
    [int]$Concurrency = 8,
    [string]$UpstreamProxy = "",
    [string]$RemoteProxyScheme = "socks5h",
    [string]$LogLevel = "warn",
    [int]$SshConnectTimeout = 20,
    [int]$RemoteCommandTimeout = 360,
    [switch]$KeepRemote
)

$ErrorActionPreference = "Stop"

function Import-LocalBenchEnv {
    $path = Join-Path $PSScriptRoot "bench.local.ps1"
    if (Test-Path -LiteralPath $path) {
        . $path
    }
}

function Split-EnvList {
    param([string]$Value)
    if ([string]::IsNullOrWhiteSpace($Value)) {
        return @()
    }
    @($Value -split '[,;]' | ForEach-Object { $_.Trim() } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
}

Import-LocalBenchEnv
if ($Targets.Count -eq 0) {
    $Targets = Split-EnvList $env:SSH_PROXY_BENCH_TARGETS
}
if ([string]::IsNullOrWhiteSpace($UpstreamProxy)) {
    $UpstreamProxy = $env:SSH_PROXY_BENCH_UPSTREAM_PROXY
}
if (-not $PSBoundParameters.ContainsKey("Url") -and -not [string]::IsNullOrWhiteSpace($env:SSH_PROXY_BENCH_URL)) {
    $Url = $env:SSH_PROXY_BENCH_URL
}
if (-not $PSBoundParameters.ContainsKey("ReadinessUrl") -and -not [string]::IsNullOrWhiteSpace($env:SSH_PROXY_BENCH_READINESS_URL)) {
    $ReadinessUrl = $env:SSH_PROXY_BENCH_READINESS_URL
}

if ($Targets.Count -eq 1 -and $Targets[0].Contains(",")) {
    throw "ambiguous -Targets value '$($Targets[0])'; use -Targets @('ssh-only-peer','direct-peer') when dot-sourcing, or run separate -Targets calls with powershell -File"
}
if ($Targets.Count -eq 0) {
    throw "missing -Targets; pass one or more SSH target aliases, for example -Targets @('ssh-only-peer','direct-peer')"
}
if ([string]::IsNullOrWhiteSpace($UpstreamProxy)) {
    throw "missing -UpstreamProxy or SSH_PROXY_BENCH_UPSTREAM_PROXY; keep concrete local proxy endpoints in scripts/bench.local.ps1"
}
Write-Host "targets=$($Targets -join ',')"
Write-Host "ssh_connect_timeout=$SshConnectTimeout"
Write-Host "remote_command_timeout=$RemoteCommandTimeout"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$bin = Join-Path $root "target\release\ssh_proxy.exe"
$remoteBinSource = Join-Path $root "target\x86_64-unknown-linux-musl\release\ssh_proxy"
if (-not (Test-Path -LiteralPath $bin)) {
    throw "missing release binary: $bin"
}
if (-not (Test-Path -LiteralPath $remoteBinSource)) {
    throw "missing Linux musl sidecar: $remoteBinSource"
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$work = Join-Path $env:TEMP "ssh_proxy-reverse-bench-$stamp"
New-Item -ItemType Directory -Force -Path $work | Out-Null
$routesPath = Join-Path $work "routes.json"

function Get-FreePort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    return $port
}

function Get-RemoteCandidatePort {
    return Get-Random -Minimum 20000 -Maximum 56000
}

function Get-SshOptions {
    @(
        "-o", "StrictHostKeyChecking=accept-new",
        "-o", "ConnectTimeout=$SshConnectTimeout",
        "-o", "ServerAliveInterval=15",
        "-o", "ServerAliveCountMax=2"
    )
}

function Get-UpstreamEndpoint {
    param([string]$ProxyUrl)
    $uri = [Uri]$ProxyUrl
    if ($uri.Port -le 0) {
        throw "UpstreamProxy must include an explicit port: $ProxyUrl"
    }
    [pscustomobject]@{
        host = $uri.Host
        port = $uri.Port
    }
}

function Wait-Tcp {
    param([int]$Port, [int]$TimeoutSeconds = 20)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $client = [System.Net.Sockets.TcpClient]::new()
        try {
            $task = $client.ConnectAsync("127.0.0.1", $Port)
            if ($task.Wait(500) -and $client.Connected) {
                return $true
            }
        }
        catch {
        }
        finally {
            $client.Dispose()
        }
        Start-Sleep -Milliseconds 200
    }
    return $false
}

function Start-LocalDaemon {
    param([int]$ControlPort, [int]$TransportPort, [string]$Token)
    $out = Join-Path $work "local-daemon.out.log"
    $err = Join-Path $work "local-daemon.err.log"
    $homeDir = Join-Path $work "home"
    New-Item -ItemType Directory -Force -Path $homeDir | Out-Null
    $args = @(
        "--log", $LogLevel,
        "node", "daemon",
        "--control", "tcp://127.0.0.1:$ControlPort",
        "--transport", "127.0.0.1:$TransportPort",
        "--token", $Token,
        "--routes-path", $routesPath,
        "--no-route-autostart"
    )
    $previousHome = $env:SSH_PROXY_HOME
    try {
        $env:SSH_PROXY_HOME = $homeDir
        return Start-Process -FilePath $bin -ArgumentList $args -PassThru -WindowStyle Hidden `
            -RedirectStandardOutput $out -RedirectStandardError $err
    }
    finally {
        $env:SSH_PROXY_HOME = $previousHome
    }
}

function Stop-ProcessQuiet {
    param($Process)
    if ($null -ne $Process -and -not $Process.HasExited) {
        Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
        $Process.WaitForExit(5000) | Out-Null
    }
}

function Invoke-Remote {
    param([string]$Target, [string]$Command, [int]$TimeoutSeconds = $RemoteCommandTimeout)
    $tmpOut = Join-Path $work "ssh-$($Target -replace '[^A-Za-z0-9_.-]', '_')-$(Get-Random).out"
    $tmpErr = Join-Path $work "ssh-$($Target -replace '[^A-Za-z0-9_.-]', '_')-$(Get-Random).err"
    $sshArgs = (Get-SshOptions) + @($Target, $Command)
    $proc = Start-Process -FilePath "ssh.exe" -ArgumentList $sshArgs -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $tmpOut -RedirectStandardError $tmpErr
    if (-not $proc.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-ProcessQuiet $proc
        throw "ssh $Target timed out after ${TimeoutSeconds}s"
    }
    $proc.Refresh()
    $code = $proc.ExitCode
    $stdout = if (Test-Path $tmpOut) { Get-Content -Raw $tmpOut } else { "" }
    $stderr = if (Test-Path $tmpErr) { Get-Content -Raw $tmpErr } else { "" }
    if ($code -ne 0) {
        throw "ssh $Target failed exit=${code}: $stderr"
    }
    return $stdout
}

function Measure-RemoteCurl {
    param([string]$Target, [int]$RemotePort, [string]$Case)
    $proxy = "${RemoteProxyScheme}://127.0.0.1:$RemotePort"
    $warm = "curl -fsSL --proxy '$proxy' --max-time 15 -o /dev/null -w '%{http_code} %{time_total}\n' '$ReadinessUrl'"
    $ready = $false
    $lastReadyError = $null
    for ($i = 0; $i -lt 6; $i++) {
        try {
            Invoke-Remote -Target $Target -Command $warm -TimeoutSeconds 120 | Out-Null
            $ready = $true
            break
        }
        catch {
            $lastReadyError = $_.Exception.Message
            Start-Sleep -Seconds 1
        }
    }
    if (-not $ready) {
        throw "remote proxy on $Target port $RemotePort never became ready: $lastReadyError"
    }

    $largeCommand = "curl -fsSL --proxy '$proxy' --max-time 360 -o /dev/null '$Url'"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    Invoke-Remote -Target $Target -Command $largeCommand -TimeoutSeconds 420 | Out-Null
    $sw.Stop()
    $largeSeconds = [Math]::Round($sw.Elapsed.TotalSeconds, 3)

    $remoteTmp = "/tmp/transport_bench_$stamp`_$RemotePort"
    $concurrentCommand = "rm -rf '$remoteTmp'; mkdir -p '$remoteTmp'; i=1; while [ `$i -le $Concurrency ]; do (curl -fsSL --proxy '$proxy' --max-time 180 -r 0-1048575 -o /dev/null '$Url' && touch '$remoteTmp/ok.'`$i) & i=`$((i+1)); done; wait; count=`$(ls '$remoteTmp'/ok.* 2>/dev/null | wc -l); rm -rf '$remoteTmp'; echo `$count"
    $sw.Restart()
    $out = Invoke-Remote -Target $Target -Command $concurrentCommand -TimeoutSeconds 240
    $sw.Stop()
    $ok = [int]($out.Trim().Split()[-1])
    $concurrentSeconds = [Math]::Round($sw.Elapsed.TotalSeconds, 3)

    [pscustomobject]@{
        target = $Target
        case = $Case
        large_seconds = $largeSeconds
        large_mibps = [Math]::Round(100.0 / $largeSeconds, 3)
        concurrent = $Concurrency
        concurrent_ok = $ok
        concurrent_seconds = $concurrentSeconds
        concurrent_mibps = if ($concurrentSeconds -gt 0) { [Math]::Round($ok / $concurrentSeconds, 3) } else { 0 }
        error = $null
    }
}

function Measure-SshdR {
    param([string]$Target)
    $remotePort = Get-RemoteCandidatePort
    $upstream = Get-UpstreamEndpoint -ProxyUrl $UpstreamProxy
    $out = Join-Path $work "$Target-sshd-r.out.log"
    $err = Join-Path $work "$Target-sshd-r.err.log"
    $proc = $null
    try {
        $proc = Start-Process -FilePath "ssh.exe" `
            -ArgumentList (@("-N", "-R", "127.0.0.1:${remotePort}:$($upstream.host):$($upstream.port)", "-o", "ExitOnForwardFailure=yes") + (Get-SshOptions) + @($Target)) `
            -PassThru -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError $err
        Start-Sleep -Seconds 2
        if ($proc.HasExited) {
            $stderr = if (Test-Path $err) { Get-Content -Raw $err } else { "" }
            throw "ssh -R exited before benchmark start: $stderr"
        }
        Measure-RemoteCurl -Target $Target -RemotePort $remotePort -Case "sshd-R-to-local-upstream"
    }
    catch {
        [pscustomobject]@{
            target = $Target
            case = "sshd-R-to-local-upstream"
            large_seconds = $null
            large_mibps = $null
            concurrent = $Concurrency
            concurrent_ok = 0
            concurrent_seconds = $null
            concurrent_mibps = $null
            error = $_.Exception.Message
        }
    }
    finally {
        Stop-ProcessQuiet $proc
        $cleanupResults.Add([pscustomobject]@{
            target = $Target
            case = "sshd-R-to-local-upstream"
            removed = $true
            kept = $false
            error = $null
        })
    }
}

function Measure-SpxReverse {
    param([string]$Target)
    $remotePort = Get-RemoteCandidatePort
    $remoteHelper = "/tmp/ssh_proxy-reverse-helper-$stamp-$($Target -replace '[^A-Za-z0-9_.-]', '_')"
    $proc = $null
    try {
        $proc = Start-Process -FilePath $bin `
            -ArgumentList @(
                "--log", $LogLevel,
                "reverse", $Target,
                "--remote-listen", "127.0.0.1:$remotePort",
                "--egress-proxy", $UpstreamProxy,
                "--deploy", "always",
                "--remote-bin", $remoteBinSource,
                "--remote-path", $remoteHelper,
                "--accept-new"
            ) `
            -PassThru -WindowStyle Hidden `
            -RedirectStandardOutput (Join-Path $work "$Target-spx-reverse.out.log") `
            -RedirectStandardError (Join-Path $work "$Target-spx-reverse.err.log")
        Start-Sleep -Seconds 2
        if ($proc.HasExited) {
            $errPath = Join-Path $work "$Target-spx-reverse.err.log"
            $stderr = if (Test-Path $errPath) { Get-Content -Raw $errPath } else { "" }
            throw "ssh_proxy reverse exited before benchmark start: $stderr"
        }
        Measure-RemoteCurl -Target $Target -RemotePort $remotePort -Case "spx-reverse-link"
    }
    catch {
        [pscustomobject]@{
            target = $Target
            case = "spx-reverse-link"
            large_seconds = $null
            large_mibps = $null
            concurrent = $Concurrency
            concurrent_ok = 0
            concurrent_seconds = $null
            concurrent_mibps = $null
            error = $_.Exception.Message
        }
    }
    finally {
        Stop-ProcessQuiet $proc
        try {
            if (-not $KeepRemote) {
                Invoke-Remote -Target $Target -Command "rm -f '$remoteHelper'" -TimeoutSeconds 30 | Out-Null
                $cleanupResults.Add([pscustomobject]@{
                    target = $Target
                    case = "spx-reverse-link"
                    removed = $true
                    kept = $false
                    error = $null
                })
            } else {
                $cleanupResults.Add([pscustomobject]@{
                    target = $Target
                    case = "spx-reverse-link"
                    removed = $false
                    kept = $true
                    error = $null
                })
            }
        }
        catch {
            $cleanupResults.Add([pscustomobject]@{
                target = $Target
                case = "spx-reverse-link"
                removed = $false
                kept = $false
                error = $_.Exception.Message
            })
        }
    }
}

$cleanupResults = New-Object System.Collections.Generic.List[object]
$results = New-Object System.Collections.Generic.List[object]

try {
    foreach ($target in $Targets) {
        $results.Add((Measure-SshdR -Target $target))
        $results.Add((Measure-SpxReverse -Target $target))
    }
}
finally {
}

$csv = Join-Path $work "results.csv"
$json = Join-Path $work "results.json"
$results | Export-Csv -NoTypeInformation -Path $csv
$summary = [pscustomobject]@{
    stamp = $stamp
    targets = $Targets
    url = $Url
    readiness_url = $ReadinessUrl
    concurrency = $Concurrency
    upstream_proxy = $UpstreamProxy
    remote_proxy_scheme = $RemoteProxyScheme
    log_level = $LogLevel
    ssh_connect_timeout = $SshConnectTimeout
    remote_command_timeout = $RemoteCommandTimeout
    keep_remote = [bool]$KeepRemote
    local_work = $work
    cleanup = $cleanupResults
    results = $results
}
$summary | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -Path $json
$results | Format-Table -AutoSize
Write-Host "results_csv=$csv"
Write-Host "results_json=$json"
