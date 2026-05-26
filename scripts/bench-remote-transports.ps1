param(
    [string]$Scenario,
    [ValidateSet("smoke", "full", "stability")]
    [string]$RunLevel = "full",
    [string[]]$Targets = @(),
    [string]$Url = "http://cachefly.cachefly.net/100mb.test",
    [switch]$UseRemotePayload,
    [int]$PayloadMiB = 100,
    [int]$Concurrency = 8,
    [int]$TransportPoolSize = 1,
    [string[]]$TransportPoolSizes = @(),
    [int]$QuicMaxBidiStreams = 256,
    [int]$QuicStreamReceiveWindow = 2097152,
    [int]$QuicReceiveWindow = 16777216,
    [int]$QuicKeepAliveIntervalSecs = 10,
    [int]$QuicIdleTimeoutSecs = 60,
    [switch]$QuicDebugLog,
    [switch]$QuicProfile,
    [switch]$SpxProfile,
    [int]$SshConnectTimeout = 20,
    [int]$RemoteCommandTimeout = 360,
    [switch]$SkipDirect,
    [switch]$RespectPreflightSkip,
    [switch]$IncludeSshControlMaster,
    [ValidateSet("none", "fresh", "reused", "both")]
    [string]$SshControlMasterBaseline = "none",
    [ValidateRange(1, 86400)]
    [int]$SshControlPersistSecs = 120,
    [ValidateRange(60, 3600)]
    [int]$StabilityDurationSecs = 1800,
    [ValidateRange(1, 300)]
    [int]$StabilitySmallIntervalSecs = 5,
    [switch]$StabilityInjectRemoteDaemonRestart,
    [switch]$KeepRemote,
    [string]$CleanupStamp,
    [string]$ResumeFromResults
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

function Quote-CommandArg {
    param([string]$Value)
    if ($null -eq $Value) {
        return "''"
    }
    if ($Value -match '\s|["]') {
        return "'" + ($Value -replace "'", "''") + "'"
    }
    return $Value
}

function Format-ArrayLiteral {
    param([object[]]$Values, [switch]$Numeric)
    if ($null -eq $Values -or $Values.Count -eq 0) {
        return '@()'
    }
    if ($Numeric) {
        return "@(" + (($Values | ForEach-Object { [string]$_ }) -join ",") + ")"
    }
    return "@(" + (($Values | ForEach-Object { "'" + ($_ -replace "'", "''") + "'" }) -join ",") + ")"
}

function Get-BenchmarkInvocation {
    param(
        [string]$ScenarioName,
        [string]$ScenarioRunLevel,
        [string[]]$ScenarioTargets,
        [bool]$ScenarioUseRemotePayload,
        [int]$ScenarioPayloadMiB,
        [int]$ScenarioConcurrency,
        [string[]]$ScenarioTransportPoolSizes,
        [int]$ScenarioTransportPoolSize,
        [int]$ScenarioQuicMaxBidiStreams,
        [int]$ScenarioQuicStreamReceiveWindow,
        [int]$ScenarioQuicReceiveWindow,
        [int]$ScenarioQuicKeepAliveIntervalSecs,
        [int]$ScenarioQuicIdleTimeoutSecs,
        [bool]$ScenarioQuicDebugLog,
        [bool]$ScenarioQuicProfile,
        [bool]$ScenarioSpxProfile,
        [int]$ScenarioSshConnectTimeout,
        [int]$ScenarioRemoteCommandTimeout,
        [bool]$ScenarioSkipDirect,
        [bool]$ScenarioRespectPreflightSkip,
        [bool]$ScenarioIncludeSshControlMaster,
        [string]$ScenarioSshControlMasterBaseline,
        [int]$ScenarioSshControlPersistSecs,
        [int]$ScenarioStabilityDurationSecs,
        [int]$ScenarioStabilitySmallIntervalSecs,
        [bool]$ScenarioStabilityInjectRemoteDaemonRestart,
        [bool]$ScenarioKeepRemote,
        [string]$ScenarioCleanupStamp,
        [string]$ScenarioResumeFromResults,
        [string]$ScenarioUrl
    )
    $parts = New-Object System.Collections.Generic.List[string]
    $parts.Add("pwsh")
    $parts.Add("-NoProfile")
    $parts.Add("-File")
    $parts.Add((Quote-CommandArg $PSCommandPath))
    if (-not [string]::IsNullOrWhiteSpace($ScenarioName)) {
        $parts.Add("-Scenario")
        $parts.Add((Quote-CommandArg $ScenarioName))
    }
    if (-not [string]::IsNullOrWhiteSpace($ScenarioRunLevel)) {
        $parts.Add("-RunLevel")
        $parts.Add((Quote-CommandArg $ScenarioRunLevel))
    }
    $parts.Add("-Targets")
    $parts.Add((Format-ArrayLiteral -Values $ScenarioTargets))
    $parts.Add("-Url")
    $parts.Add((Quote-CommandArg $ScenarioUrl))
    if ($ScenarioUseRemotePayload) {
        $parts.Add("-UseRemotePayload")
    }
    $parts.Add("-PayloadMiB")
    $parts.Add([string]$ScenarioPayloadMiB)
    $parts.Add("-Concurrency")
    $parts.Add([string]$ScenarioConcurrency)
    $parts.Add("-TransportPoolSize")
    $parts.Add([string]$ScenarioTransportPoolSize)
    $parts.Add("-TransportPoolSizes")
    $parts.Add((Format-ArrayLiteral -Values $ScenarioTransportPoolSizes -Numeric))
    $parts.Add("-QuicMaxBidiStreams")
    $parts.Add([string]$ScenarioQuicMaxBidiStreams)
    $parts.Add("-QuicStreamReceiveWindow")
    $parts.Add([string]$ScenarioQuicStreamReceiveWindow)
    $parts.Add("-QuicReceiveWindow")
    $parts.Add([string]$ScenarioQuicReceiveWindow)
    $parts.Add("-QuicKeepAliveIntervalSecs")
    $parts.Add([string]$ScenarioQuicKeepAliveIntervalSecs)
    $parts.Add("-QuicIdleTimeoutSecs")
    $parts.Add([string]$ScenarioQuicIdleTimeoutSecs)
    if ($ScenarioQuicDebugLog) {
        $parts.Add("-QuicDebugLog")
    }
    if ($ScenarioQuicProfile) {
        $parts.Add("-QuicProfile")
    }
    if ($ScenarioSpxProfile) {
        $parts.Add("-SpxProfile")
    }
    $parts.Add("-SshConnectTimeout")
    $parts.Add([string]$ScenarioSshConnectTimeout)
    $parts.Add("-RemoteCommandTimeout")
    $parts.Add([string]$ScenarioRemoteCommandTimeout)
    if ($ScenarioSkipDirect) {
        $parts.Add("-SkipDirect")
    }
    if ($ScenarioRespectPreflightSkip) {
        $parts.Add("-RespectPreflightSkip")
    }
    if ($ScenarioIncludeSshControlMaster) {
        $parts.Add("-IncludeSshControlMaster")
    }
    if (-not [string]::IsNullOrWhiteSpace($ScenarioSshControlMasterBaseline)) {
        $parts.Add("-SshControlMasterBaseline")
        $parts.Add((Quote-CommandArg $ScenarioSshControlMasterBaseline))
    }
    $parts.Add("-SshControlPersistSecs")
    $parts.Add([string]$ScenarioSshControlPersistSecs)
    $parts.Add("-StabilityDurationSecs")
    $parts.Add([string]$ScenarioStabilityDurationSecs)
    $parts.Add("-StabilitySmallIntervalSecs")
    $parts.Add([string]$ScenarioStabilitySmallIntervalSecs)
    if ($ScenarioStabilityInjectRemoteDaemonRestart) {
        $parts.Add("-StabilityInjectRemoteDaemonRestart")
    }
    if ($ScenarioKeepRemote) {
        $parts.Add("-KeepRemote")
    }
    if (-not [string]::IsNullOrWhiteSpace($ScenarioCleanupStamp)) {
        $parts.Add("-CleanupStamp")
        $parts.Add((Quote-CommandArg $ScenarioCleanupStamp))
    }
    if (-not [string]::IsNullOrWhiteSpace($ScenarioResumeFromResults)) {
        $parts.Add("-ResumeFromResults")
        $parts.Add((Quote-CommandArg $ScenarioResumeFromResults))
    }
    return ($parts -join " ")
}

function Get-RunLevelPreset {
    param([string]$Level)
    switch ($Level.ToLowerInvariant()) {
        "smoke" {
            return [pscustomobject]@{
                payload_mib = 16
                concurrency = 2
                transport_pool_size = 1
                transport_pool_sizes = @("1", "2")
            }
        }
        "full" {
            return [pscustomobject]@{
                payload_mib = 100
                concurrency = 8
                transport_pool_size = 1
                transport_pool_sizes = @("1", "2", "4", "8")
            }
        }
        "stability" {
            return [pscustomobject]@{
                payload_mib = 100
                concurrency = 8
                transport_pool_size = 4
                transport_pool_sizes = @("4")
            }
        }
        default {
            throw "unknown benchmark run level '$Level'; supported values: smoke, full, stability"
        }
    }
}

function Get-SshOptions {
    @(
        "-o", "StrictHostKeyChecking=accept-new",
        "-o", "ConnectTimeout=$SshConnectTimeout",
        "-o", "ServerAliveInterval=15",
        "-o", "ServerAliveCountMax=2"
    )
}

function Get-QuicTuningArgs {
    @(
        "--quic-max-bidi-streams", $QuicMaxBidiStreams.ToString(),
        "--quic-stream-receive-window", $QuicStreamReceiveWindow.ToString(),
        "--quic-receive-window", $QuicReceiveWindow.ToString(),
        "--quic-keep-alive-interval-secs", $QuicKeepAliveIntervalSecs.ToString(),
        "--quic-idle-timeout-secs", $QuicIdleTimeoutSecs.ToString()
    )
}

function Get-CurrentQuicParameterSet {
    [pscustomobject]@{
        max_bidi_streams = $QuicMaxBidiStreams
        stream_receive_window = $QuicStreamReceiveWindow
        receive_window = $QuicReceiveWindow
        keep_alive_interval_secs = $QuicKeepAliveIntervalSecs
        idle_timeout_secs = $QuicIdleTimeoutSecs
    }
}

function New-BenchmarkCaseKey {
    param([string]$RunLevelValue, [string]$Target, [string]$Case, [int]$PoolSize)
    "$RunLevelValue|$Target|$Case|$PoolSize"
}

function Test-ResumableResultRow {
    param($Row)
    if ($null -eq $Row) {
        return $false
    }
    $errorKind = [string]$Row.error_kind
    if ([string]::IsNullOrWhiteSpace($errorKind)) {
        return $true
    }
    if ($errorKind -eq "preflight_skip") {
        return $true
    }
    return ([string]$Row.skipped_by_preflight).ToLowerInvariant() -eq "true"
}

function Copy-ResultRowForResume {
    param($Row, [string]$Source, [string]$Key)
    $copy = [pscustomobject]@{}
    foreach ($property in $Row.PSObject.Properties) {
        Add-Member -InputObject $copy -NotePropertyName $property.Name -NotePropertyValue $property.Value
    }
    $baselineMode = Get-BaselineMode -Case ([string]$copy.case) -OpenSshControlMasterMode ([string]$copy.openssh_control_master_mode)
    $baselineQuality = Get-BaselineQuality `
        -Case ([string]$copy.case) `
        -ErrorKind ([string]$copy.error_kind) `
        -Error ([string]$copy.error) `
        -OpenSshControlMasterMode ([string]$copy.openssh_control_master_mode)
    Add-Member -InputObject $copy -NotePropertyName "baseline_mode" -NotePropertyValue $baselineMode -Force
    Add-Member -InputObject $copy -NotePropertyName "baseline_quality" -NotePropertyValue $baselineQuality.quality -Force
    Add-Member -InputObject $copy -NotePropertyName "baseline_quality_reason" -NotePropertyValue $baselineQuality.reason -Force
    Add-Member -InputObject $copy -NotePropertyName "baseline_client_os" -NotePropertyValue $script:OpenSshCapability.os -Force
    Add-Member -InputObject $copy -NotePropertyName "baseline_client_arch" -NotePropertyValue $script:OpenSshCapability.arch -Force
    Add-Member -InputObject $copy -NotePropertyName "openssh_client_version" -NotePropertyValue $script:OpenSshCapability.client_version -Force
    Add-Member -InputObject $copy -NotePropertyName "openssh_controlmaster_supported" -NotePropertyValue $script:OpenSshCapability.control_master_supported -Force
    Add-Member -InputObject $copy -NotePropertyName "openssh_capability_notes" -NotePropertyValue $script:OpenSshCapability.capability_notes -Force
    Add-Member -InputObject $copy -NotePropertyName "resumed_from_results" -NotePropertyValue $Source -Force
    Add-Member -InputObject $copy -NotePropertyName "resumed_case_key" -NotePropertyValue $Key -Force
    Add-Member -InputObject $copy -NotePropertyName "resumed_at_unix" -NotePropertyValue ([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()) -Force
    return $copy
}

function Get-QuicLogFilter {
    if ($QuicDebugLog) {
        return "ssh_proxy::quic_native=trace,ssh_proxy::node_daemon::quic_transport=trace,warn"
    }
    return "warn"
}

function Test-QuicBenchmarkCase {
    param([string]$Case)
    return $Case -eq "spx-quic-direct" -or $Case -eq "quic-native-direct"
}

function Get-LocalPlatform {
    [pscustomobject]@{
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    }
}

function Get-OpenSshCapability {
    $platform = Get-LocalPlatform
    $command = Get-Command ssh.exe -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        return [pscustomobject]@{
            client_version = $null
            client_path = $null
            os = $platform.os
            arch = $platform.arch
            control_master_supported = $false
            capability_notes = "ssh.exe was not found on PATH; OpenSSH baselines are environment-limited"
        }
    }

    $version = $null
    try {
        $version = (& ssh.exe -V 2>&1 | Out-String).Trim()
    }
    catch {
        $version = $_.Exception.Message
    }

    $isWindowsPlatform = $platform.os -match "Windows"
    [pscustomobject]@{
        client_version = if ([string]::IsNullOrWhiteSpace($version)) { "unknown" } else { $version }
        client_path = $command.Source
        os = $platform.os
        arch = $platform.arch
        control_master_supported = -not $isWindowsPlatform
        capability_notes = if ($isWindowsPlatform) {
            "ControlMaster is not assumed usable on Windows OpenSSH; per-case rows mark successful probes valid and socket/client failures environment-limited"
        } else {
            "ControlMaster is expected to be usable when the local OpenSSH client supports control sockets; per-case rows still carry final quality"
        }
    }
}

function Get-BaselineMode {
    param([string]$Case, [string]$OpenSshControlMasterMode = $null)
    switch ($Case) {
        "sshd-D" { return "openssh-socks-fresh" }
        "sshd-D-controlmaster-fresh" { return "openssh-controlmaster-fresh" }
        "sshd-D-controlmaster" { return "openssh-controlmaster-reused" }
        "ssh-native-direct" { return "russh-ssh-native" }
        "spx-ssh-direct" { return "spx-over-ssh" }
        default { return $null }
    }
}

function Get-BaselineQuality {
    param(
        [string]$Case,
        [string]$ErrorKind = $null,
        [string]$Error = $null,
        [string]$OpenSshControlMasterMode = $null,
        $Capability = $script:OpenSshCapability
    )
    $mode = Get-BaselineMode -Case $Case -OpenSshControlMasterMode $OpenSshControlMasterMode
    if ([string]::IsNullOrWhiteSpace([string]$mode)) {
        return [pscustomobject]@{ quality = $null; reason = $null }
    }

    if ($ErrorKind -eq "preflight_skip") {
        return [pscustomobject]@{
            quality = "skipped"
            reason = "baseline row was skipped by route preflight and is not a performance point"
        }
    }

    $hasError = -not [string]::IsNullOrWhiteSpace([string]$ErrorKind) -or -not [string]::IsNullOrWhiteSpace([string]$Error)
    if (-not $hasError) {
        return [pscustomobject]@{
            quality = "valid"
            reason = if ($mode -like "openssh-controlmaster*") {
                "OpenSSH ControlMaster baseline completed and can be compared despite platform capability assumptions"
            } else {
                "baseline completed without runtime error"
            }
        }
    }

    $message = "$ErrorKind $Error"
    if ($mode -like "openssh-controlmaster*" -and $message -match "ControlMaster|control socket|Not a socket|platform|client|not usable") {
        return [pscustomobject]@{
            quality = "environment-limited"
            reason = "local OpenSSH ControlMaster capability prevented a valid baseline"
        }
    }
    if ($mode -eq "openssh-socks-fresh" -and $message -match "ssh -D|OpenSSH|ssh\.exe|not found|not recognized|did not listen|ExitOnForwardFailure") {
        return [pscustomobject]@{
            quality = "environment-limited"
            reason = "local OpenSSH SOCKS baseline did not produce a usable listener in this environment"
        }
    }

    [pscustomobject]@{
        quality = "degraded"
        reason = "baseline ran in this environment but failed or produced an incomplete result"
    }
}

function Test-BaselineRowValid {
    param($Row)
    if ($null -eq $Row) {
        return $false
    }
    if ($null -ne $Row.PSObject.Properties["baseline_quality"] -and -not [string]::IsNullOrWhiteSpace([string]$Row.baseline_quality)) {
        return [string]$Row.baseline_quality -eq "valid"
    }
    return [string]::IsNullOrWhiteSpace([string]$Row.error)
}

function Get-BaselineComparisonQuality {
    param($Baseline, $Candidate)
    if ((Test-BaselineRowValid -Row $Baseline) -and (Test-BaselineRowValid -Row $Candidate)) {
        return "valid"
    }
    $qualities = @($Baseline, $Candidate) | ForEach-Object {
        if ($null -ne $_ -and $null -ne $_.PSObject.Properties["baseline_quality"]) {
            [string]$_.baseline_quality
        }
    }
    if ($qualities -contains "environment-limited") {
        return "environment-limited"
    }
    if ($qualities -contains "skipped") {
        return "skipped"
    }
    return "degraded"
}

function Get-BaselineQualitySummary {
    param([object[]]$Rows)
    @($Rows | Where-Object {
        $null -ne $_.PSObject.Properties["baseline_mode"] -and
        -not [string]::IsNullOrWhiteSpace([string]$_.baseline_mode)
    } | Group-Object baseline_mode, baseline_quality | ForEach-Object {
        $first = @($_.Group | Select-Object -First 1)[0]
        [pscustomobject]@{
            baseline_mode = $first.baseline_mode
            quality = $first.baseline_quality
            count = $_.Count
            example_reason = $first.baseline_quality_reason
        }
    })
}

function Get-RemotePlatform {
    param([string]$Target)
    $text = Invoke-RemoteCapture $Target "printf 'os='; uname -s 2>/dev/null || printf 'unknown\n'; printf 'arch='; uname -m 2>/dev/null || printf 'unknown\n'"
    $os = $null
    $arch = $null
    foreach ($line in ($text -split "`r?`n")) {
        if ($line -match '^os=(.*)$') {
            $os = $Matches[1].Trim()
        }
        if ($line -match '^arch=(.*)$') {
            $arch = $Matches[1].Trim()
        }
    }
    [pscustomobject]@{
        os = if ([string]::IsNullOrWhiteSpace($os)) { "unknown" } else { $os }
        arch = if ([string]::IsNullOrWhiteSpace($arch)) { "unknown" } else { $arch }
    }
}

function Get-SafeTargetName {
    param([string]$Target)
    return ($Target -replace '[^A-Za-z0-9_.-]', '_')
}

function Quote-RemoteShellValue {
    param([string]$Value)
    return "'" + ($Value -replace "'", "'\''") + "'"
}

function New-RemoteBenchPlan {
    param(
        [string]$Target,
        [string]$Stamp,
        [string]$DirectHost = $null,
        [int]$Control = 0,
        [int]$Plain = 0,
        [int]$Tls = 0,
        [int]$Quic = 0,
        [int]$Http = 0
    )
    $safeTarget = Get-SafeTargetName -Target $Target
    $remoteDir = "/tmp/transport-bench-$Stamp-$safeTarget"
    [pscustomobject]@{
        target = $Target
        direct_host = $DirectHost
        stamp = $Stamp
        remote_dir = $remoteDir
        control = $Control
        plain = $Plain
        tls = $Tls
        quic = $Quic
        http = $Http
        paths = [pscustomobject]@{
            home = "$remoteDir/home"
            binary = "$remoteDir/ssh_proxy"
            cert = "$remoteDir/cert.pem"
            key = "$remoteDir/key.pem"
            range_server = "$remoteDir/range_server.py"
            payload = "$remoteDir/payload.bin"
            routes = "$remoteDir/routes.json"
            daemon_log = "$remoteDir/daemon.log"
            http_log = "$remoteDir/http.log"
        }
        pid_files = [pscustomobject]@{
            daemon = "$remoteDir/daemon.pid"
            http = "$remoteDir/http.pid"
        }
    }
}

function Get-BenchmarkFailureClassification {
    param(
        [string]$Message,
        $PlanCapture = $null,
        [string]$ForcedKind = $null,
        [string]$ForcedStage = $null
    )
    if (-not [string]::IsNullOrWhiteSpace($ForcedKind)) {
        $stage = if (-not [string]::IsNullOrWhiteSpace($ForcedStage)) { $ForcedStage } else { $ForcedKind }
        return [pscustomobject]@{
            error_kind = $ForcedKind
            primary_failure_stage = $stage
            human_summary = if ([string]::IsNullOrWhiteSpace($Message)) { $ForcedKind } else { $Message }
        }
    }
    $topologyFailure = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_topology_failure"
    $candidateFailures = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_candidate_failures"
    $candidateEndpoints = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_candidate_failure_endpoints"
    if (-not [string]::IsNullOrWhiteSpace([string]$topologyFailure)) {
        $detailParts = @($candidateFailures, $candidateEndpoints) | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) }
        $detail = if ($detailParts.Count -gt 0) { ": " + ($detailParts -join "; ") } else { "" }
        return [pscustomobject]@{
            error_kind = "connect_timeout"
            primary_failure_stage = "preflight"
            human_summary = "direct endpoint unreachable$detail"
        }
    }
    $text = [string]$Message
    $lower = $text.ToLowerInvariant()
    if ($lower -match "preflight") {
        return [pscustomobject]@{
            error_kind = "preflight_skip"
            primary_failure_stage = "preflight"
            human_summary = if ([string]::IsNullOrWhiteSpace($text)) { "skipped by route preflight" } else { $text }
        }
    }
    if ($lower -match "permission denied|auth|authentication|host key verification|known_host|known host") {
        return [pscustomobject]@{
            error_kind = "auth_failed"
            primary_failure_stage = "auth"
            human_summary = $text
        }
    }
    if ($lower -match "handshake|certificate|cert|tls|quic.*crypto|crypto.*quic") {
        return [pscustomobject]@{
            error_kind = "handshake_timeout"
            primary_failure_stage = "handshake"
            human_summary = $text
        }
    }
    if ($lower -match "did not listen|bind|address already in use|listener") {
        return [pscustomobject]@{
            error_kind = "listener_start_failed"
            primary_failure_stage = "listener_start"
            human_summary = $text
        }
    }
    if ($lower -match "control.*degraded|control connect timed out|empty control response|control request") {
        return [pscustomobject]@{
            error_kind = "control_degraded"
            primary_failure_stage = "control"
            human_summary = $text
        }
    }
    if ($lower -match "curl failed|copy|broken pipe|connection reset|reset by peer|write failed|read failed") {
        return [pscustomobject]@{
            error_kind = "copy_failed"
            primary_failure_stage = "copy"
            human_summary = $text
        }
    }
    if ($lower -match "timed out|timeout|connect failed|connection refused|unreachable|no route to host") {
        return [pscustomobject]@{
            error_kind = "connect_timeout"
            primary_failure_stage = "connect"
            human_summary = $text
        }
    }
    [pscustomobject]@{
        error_kind = "copy_failed"
        primary_failure_stage = "runtime"
        human_summary = if ([string]::IsNullOrWhiteSpace($text)) { "runtime failure" } else { $text }
    }
}

function Invoke-RemoteCleanup {
    param($Plan)
    $pidFiles = @($Plan.pid_files.PSObject.Properties | ForEach-Object { $_.Value })
    $pidList = ($pidFiles | ForEach-Object { Quote-RemoteShellValue $_ }) -join " "
    $command = 'for f in ' + $pidList + '; do if [ -f "$f" ]; then pid=$(cat "$f"); kill "$pid" 2>/dev/null || true; sleep 1; kill -9 "$pid" 2>/dev/null || true; fi; done; rm -rf ' + (Quote-RemoteShellValue $Plan.remote_dir)
    $output = & ssh.exe @((Get-SshOptions) + @($Plan.target, $command)) 2>&1
    $exitCode = $LASTEXITCODE
    $text = ($output | Out-String).Trim()
    $failure = if ($exitCode -eq 0) {
        $null
    } else {
        Get-BenchmarkFailureClassification -Message $text -ForcedKind "cleanup_failed" -ForcedStage "cleanup"
    }
    [pscustomobject]@{
        target = $Plan.target
        remote_dir = $Plan.remote_dir
        command = $command
        exit_code = $exitCode
        removed = $exitCode -eq 0
        kept = $false
        paths = $Plan.paths
        pid_files = $Plan.pid_files
        output = if ([string]::IsNullOrWhiteSpace($text)) { $null } else { $text }
        primary_failure_stage = if ($null -eq $failure) { $null } else { $failure.primary_failure_stage }
        human_summary = if ($null -eq $failure) { $null } else { $failure.human_summary }
        error_kind = if ($null -eq $failure) { $null } else { $failure.error_kind }
        error = if ($exitCode -eq 0) { $null } else { $text }
    }
}

function Convert-PoolSizeList {
    param(
        [string[]]$Values,
        [int]$DefaultValue
    )

    $rawValues = New-Object System.Collections.Generic.List[string]
    foreach ($value in $Values) {
        if ($null -eq $value) {
            continue
        }
        foreach ($part in ([string]$value -split ',')) {
            $trimmed = $part.Trim()
            if (-not [string]::IsNullOrWhiteSpace($trimmed)) {
                $rawValues.Add($trimmed)
            }
        }
    }
    if ($rawValues.Count -eq 0) {
        $rawValues.Add([string]$DefaultValue)
    }

    $poolSizes = New-Object System.Collections.Generic.List[int]
    foreach ($raw in $rawValues) {
        if ($raw -notmatch '^\d+$') {
            throw "invalid transport pool size '$raw'; use values like -TransportPoolSizes '1,2,4,8'"
        }
        $poolSize = [int]::Parse($raw, [Globalization.CultureInfo]::InvariantCulture)
        if ($poolSize -lt 1) {
            throw "transport pool sizes must be >= 1; got $poolSize"
        }
        if ($poolSize -gt 64) {
            throw "transport pool size $poolSize is unusually high; if this came from -TransportPoolSizes 1,2,4,8 with -File, quote it as -TransportPoolSizes '1,2,4,8' or use pwsh -Command with @(1,2,4,8)"
        }
        $poolSizes.Add($poolSize)
    }

    return @($poolSizes | Sort-Object -Unique)
}

Import-LocalBenchEnv
if ($Targets.Count -eq 0) {
    $Targets = Split-EnvList $env:SSH_PROXY_BENCH_TARGETS
}
if (-not $PSBoundParameters.ContainsKey("Url") -and -not [string]::IsNullOrWhiteSpace($env:SSH_PROXY_BENCH_URL)) {
    $Url = $env:SSH_PROXY_BENCH_URL
}

if ($Targets.Count -eq 1 -and $Targets[0].Contains(",")) {
    throw "ambiguous -Targets value '$($Targets[0])'; use -Targets @('ssh-only-peer','direct-peer') when dot-sourcing, or run separate -Targets calls with powershell -File"
}
$scenarioName = $Scenario
$runLevelName = $RunLevel.ToLowerInvariant()
$runLevelPreset = Get-RunLevelPreset -Level $runLevelName
if (-not $PSBoundParameters.ContainsKey("PayloadMiB")) {
    $PayloadMiB = $runLevelPreset.payload_mib
}
if (-not $PSBoundParameters.ContainsKey("Concurrency")) {
    $Concurrency = $runLevelPreset.concurrency
}
if (-not $PSBoundParameters.ContainsKey("TransportPoolSize")) {
    $TransportPoolSize = $runLevelPreset.transport_pool_size
}
if (-not $PSBoundParameters.ContainsKey("TransportPoolSizes") -and -not $PSBoundParameters.ContainsKey("TransportPoolSize")) {
    $TransportPoolSizes = $runLevelPreset.transport_pool_sizes
}
if ($runLevelName -eq "stability" -and -not $PSBoundParameters.ContainsKey("RespectPreflightSkip")) {
    $RespectPreflightSkip = $true
}
if (-not [string]::IsNullOrWhiteSpace($scenarioName)) {
    switch ($scenarioName) {
        "two-peer-transport-matrix" {
            $UseRemotePayload = $true
            if ($Targets.Count -eq 0) {
                $Targets = Split-EnvList $env:SSH_PROXY_BENCH_TARGETS
            }
        }
        default {
            throw "unknown benchmark scenario '$scenarioName'; supported values: two-peer-transport-matrix"
        }
    }
}
if ($Targets.Count -eq 0) {
    throw "missing -Targets; pass one or more SSH target aliases, for example -Targets @('ssh-only-peer','direct-peer')"
}
if ($IncludeSshControlMaster -and -not $PSBoundParameters.ContainsKey("SshControlMasterBaseline")) {
    $SshControlMasterBaseline = "reused"
}
if ($SshControlMasterBaseline -ne "none") {
    $IncludeSshControlMaster = $true
}
$TransportPoolSizes = Convert-PoolSizeList -Values $TransportPoolSizes -DefaultValue $TransportPoolSize
if (-not [string]::IsNullOrWhiteSpace($CleanupStamp)) {
    $cleanupOnlyResults = New-Object System.Collections.Generic.List[object]
    foreach ($target in $Targets) {
        $plan = New-RemoteBenchPlan -Target $target -Stamp $CleanupStamp
        try {
            $cleanupOnlyResults.Add((Invoke-RemoteCleanup -Plan $plan))
        }
        catch {
            $failure = Get-BenchmarkFailureClassification -Message $_.Exception.Message -ForcedKind "cleanup_failed" -ForcedStage "cleanup"
            $cleanupOnlyResults.Add([pscustomobject]@{
                target = $target
                remote_dir = $plan.remote_dir
                command = $null
                exit_code = $null
                removed = $false
                kept = $false
                paths = $plan.paths
                pid_files = $plan.pid_files
                output = $null
                primary_failure_stage = $failure.primary_failure_stage
                human_summary = $failure.human_summary
                error_kind = $failure.error_kind
                error = $_.Exception.Message
            })
        }
    }
    $cleanupOnlySummary = [pscustomobject]@{
        mode = "cleanup"
        stamp = $CleanupStamp
        targets = $Targets
        cleanup = $cleanupOnlyResults
    }
    $cleanupOnlySummary | ConvertTo-Json -Depth 8
    return
}
Write-Host "targets=$($Targets -join ',')"
Write-Host "run_level=$runLevelName"
Write-Host "transport_pool_sizes=$($TransportPoolSizes -join ',')"
Write-Host "quic_max_bidi_streams=$QuicMaxBidiStreams"
Write-Host "quic_stream_receive_window=$QuicStreamReceiveWindow"
Write-Host "quic_receive_window=$QuicReceiveWindow"
Write-Host "quic_keep_alive_interval_secs=$QuicKeepAliveIntervalSecs"
Write-Host "quic_idle_timeout_secs=$QuicIdleTimeoutSecs"
Write-Host "quic_debug_log=$([bool]$QuicDebugLog)"
Write-Host "quic_profile=$([bool]$QuicProfile)"
Write-Host "spx_profile=$([bool]$SpxProfile)"
Write-Host "ssh_connect_timeout=$SshConnectTimeout"
Write-Host "remote_command_timeout=$RemoteCommandTimeout"
Write-Host "respect_preflight_skip=$([bool]$RespectPreflightSkip)"
Write-Host "ssh_control_master_baseline=$SshControlMasterBaseline"
Write-Host "stability_duration_secs=$StabilityDurationSecs"
Write-Host "stability_small_interval_secs=$StabilitySmallIntervalSecs"
Write-Host "stability_inject_remote_daemon_restart=$([bool]$StabilityInjectRemoteDaemonRestart)"
Write-Host "resume_from_results=$ResumeFromResults"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$localBin = Join-Path $root "target\release\ssh_proxy.exe"
$remoteBinSource = Join-Path $root "target\x86_64-unknown-linux-musl\release\ssh_proxy"
if (-not (Test-Path -LiteralPath $localBin)) {
    throw "missing release binary: $localBin"
}
if (-not (Test-Path -LiteralPath $remoteBinSource)) {
    throw "missing Linux musl sidecar: $remoteBinSource"
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$localWork = Join-Path $env:TEMP "transport-bench-$stamp"
New-Item -ItemType Directory -Force -Path $localWork | Out-Null
$cert = Join-Path $localWork "bench-cert.pem"
$key = Join-Path $localWork "bench-key.pem"
$rangeServer = Join-Path $localWork "range_server.py"
$opensslErr = Join-Path $localWork "openssl.err.log"
$opensslArgs = @(
    "req", "-x509", "-newkey", "rsa:2048", "-nodes",
    "-keyout", $key, "-out", $cert, "-days", "2",
    "-subj", "/CN=ssh-proxy-bench",
    "-addext", "basicConstraints=critical,CA:FALSE",
    "-addext", "keyUsage=critical,digitalSignature,keyEncipherment",
    "-addext", "extendedKeyUsage=serverAuth",
    "-addext", "subjectAltName=DNS:ssh-proxy-bench,DNS:localhost,IP:127.0.0.1"
)
$openssl = Start-Process -FilePath "openssl.exe" -ArgumentList $opensslArgs -Wait -PassThru -WindowStyle Hidden -RedirectStandardError $opensslErr
if ($openssl.ExitCode -ne 0) {
    $detail = Get-Content -LiteralPath $opensslErr -Raw -ErrorAction SilentlyContinue
    throw "openssl failed to generate benchmark certificate: $detail"
}
Set-Content -Encoding UTF8 -LiteralPath $rangeServer -Value @'
import os
import re
import shutil
import sys
from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler


class RangeHandler(SimpleHTTPRequestHandler):
    def send_head(self):
        path = self.translate_path(self.path)
        if os.path.isdir(path):
            self.send_error(404)
            return None
        try:
            f = open(path, 'rb')
        except OSError:
            self.send_error(404)
            return None
        size = os.fstat(f.fileno()).st_size
        self.range = None
        value = self.headers.get('Range')
        if value:
            match = re.match(r'bytes=(\d*)-(\d*)$', value)
            if match:
                start = int(match.group(1) or 0)
                end = int(match.group(2) or size - 1)
                end = min(end, size - 1)
                if start <= end:
                    self.range = (start, end)
                    self.send_response(206)
                    self.send_header('Content-type', 'application/octet-stream')
                    self.send_header('Accept-Ranges', 'bytes')
                    self.send_header('Content-Range', f'bytes {start}-{end}/{size}')
                    self.send_header('Content-Length', str(end - start + 1))
                    self.end_headers()
                    return f
        self.send_response(200)
        self.send_header('Content-type', 'application/octet-stream')
        self.send_header('Accept-Ranges', 'bytes')
        self.send_header('Content-Length', str(size))
        self.end_headers()
        return f

    def copyfile(self, source, outputfile):
        if self.range is None:
            shutil.copyfileobj(source, outputfile)
            return
        start, end = self.range
        source.seek(start)
        remaining = end - start + 1
        while remaining > 0:
            chunk = source.read(min(1024 * 1024, remaining))
            if not chunk:
                break
            outputfile.write(chunk)
            remaining -= len(chunk)


if __name__ == '__main__':
    os.chdir(os.path.dirname(__file__))
    ThreadingHTTPServer((sys.argv[1], int(sys.argv[2])), RangeHandler).serve_forever()
'@

function Invoke-Checked {
    param(
        [string]$File,
        [string[]]$ArgumentList,
        [string]$Label,
        [int]$TimeoutSeconds = 0
    )
    $out = Join-Path $localWork "$($Label -replace '[^A-Za-z0-9_.-]', '_')-$(Get-Random).out.log"
    $err = Join-Path $localWork "$($Label -replace '[^A-Za-z0-9_.-]', '_')-$(Get-Random).err.log"
    $proc = Start-Process -FilePath $File -ArgumentList $ArgumentList -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $out -RedirectStandardError $err
    $finished = if ($TimeoutSeconds -gt 0) {
        $proc.WaitForExit($TimeoutSeconds * 1000)
    } else {
        $proc.WaitForExit()
        $true
    }
    if (-not $finished) {
        Stop-ProcessQuiet $proc
        throw "$Label timed out after ${TimeoutSeconds}s"
    }
    $proc.Refresh()
    if ($proc.ExitCode -ne 0) {
        $stderr = Get-Content -LiteralPath $err -Raw -ErrorAction SilentlyContinue
        $stdout = Get-Content -LiteralPath $out -Raw -ErrorAction SilentlyContinue
        $detail = (($stderr, $stdout) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }) -join "`n"
        if ([string]::IsNullOrWhiteSpace($detail)) {
            $detail = "no output"
        }
        throw "$Label failed with exit code $($proc.ExitCode): $detail"
    }
}

function Invoke-LocalCapture {
    param(
        [string[]]$ArgumentList,
        [string]$Label,
        [int]$TimeoutSeconds = 30
    )
    $out = Join-Path $localWork "$($Label -replace '[^A-Za-z0-9_.-]', '_')-$(Get-Random).out.log"
    $err = Join-Path $localWork "$($Label -replace '[^A-Za-z0-9_.-]', '_')-$(Get-Random).err.log"
    $cleanArgs = @($ArgumentList | Where-Object { $_ -ne $null -and $_ -ne "" })
    $argLine = ($cleanArgs | ForEach-Object {
        if ($_ -match '\s|\"') {
            '"' + ($_ -replace '"', '\"') + '"'
        } else {
            $_
        }
    }) -join ' '
    $proc = Start-Process -FilePath $localBin -ArgumentList $argLine -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $out -RedirectStandardError $err
    $finished = $proc.WaitForExit($TimeoutSeconds * 1000)
    if (-not $finished) {
        Stop-ProcessQuiet $proc
        throw "$Label timed out after ${TimeoutSeconds}s"
    }
    $proc.Refresh()
    $stdout = Get-Content -LiteralPath $out -Raw -ErrorAction SilentlyContinue
    $stderr = Get-Content -LiteralPath $err -Raw -ErrorAction SilentlyContinue
    if ($proc.ExitCode -ne 0) {
        $detail = (($stderr, $stdout) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }) -join "`n"
        if ([string]::IsNullOrWhiteSpace($detail)) {
            $detail = "no output"
        }
        throw "$Label failed with exit code $($proc.ExitCode): $detail"
    }
    return $stdout
}

function Invoke-Remote {
    param([string]$Target, [string]$Command)
    Invoke-Checked -File "ssh.exe" -ArgumentList ((Get-SshOptions) + @($Target, $Command)) -Label "ssh $Target" -TimeoutSeconds $RemoteCommandTimeout
}

function Invoke-RemoteCapture {
    param([string]$Target, [string]$Command)
    $output = & ssh.exe @((Get-SshOptions) + @($Target, $Command)) 2>&1
    if ($LASTEXITCODE -ne 0) {
        $message = ($output | Out-String).Trim()
        if ([string]::IsNullOrWhiteSpace($message)) {
            $message = "no output"
        }
        throw "ssh ${Target} capture failed with exit code ${LASTEXITCODE}: $message"
    }
    return ($output | Out-String).Trim()
}

function Copy-Remote {
    param([string]$Target, [string]$Local, [string]$Remote)
    Invoke-Checked -File "scp.exe" -ArgumentList ((Get-SshOptions) + @($Local, "${Target}:$Remote")) -Label "scp $Target" -TimeoutSeconds $RemoteCommandTimeout
}

function Get-DirectHost {
    param([string]$Target)
    $line = & ssh.exe -G $Target 2>$null | Where-Object { $_ -match '^hostname\s+' } | Select-Object -First 1
    if ($line -match '^hostname\s+(.+)$') {
        return $Matches[1].Trim()
    }
    return $Target
}

function Wait-Tcp {
    param([string]$HostName, [int]$Port, [int]$TimeoutSeconds = 20)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $client = [System.Net.Sockets.TcpClient]::new()
        try {
            $task = $client.ConnectAsync($HostName, $Port)
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

function Get-FreePort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    return $port
}

function Start-LocalProcess {
    param([string[]]$ProcArgs, [string]$Name)
    $out = Join-Path $localWork "$Name.out.log"
    $err = Join-Path $localWork "$Name.err.log"
    $cleanArgs = @($ProcArgs | Where-Object { $_ -ne $null -and $_ -ne "" })
    $argLine = ($cleanArgs | ForEach-Object {
        if ($_ -match '\s|\"') {
            '"' + ($_ -replace '"', '\"') + '"'
        } else {
            $_
        }
    }) -join ' '
    return Start-Process -FilePath $localBin -ArgumentList $argLine -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $out -RedirectStandardError $err
}

function Get-ProcessLogSummary {
    param([string]$Name)
    $parts = New-Object System.Collections.Generic.List[string]
    foreach ($kind in @("err", "out")) {
        $path = Join-Path $localWork "$Name.$kind.log"
        if (-not (Test-Path -LiteralPath $path)) {
            continue
        }
        $text = (Get-Content -LiteralPath $path -Tail 12 -ErrorAction SilentlyContinue) -join "`n"
        if (-not [string]::IsNullOrWhiteSpace($text)) {
            $parts.Add("${kind}: $text")
        }
    }
    return ($parts.ToArray() -join "`n")
}

function Start-SshSocks {
    param(
        [string]$Target,
        [int]$Port,
        [string]$Name,
        [string]$ControlPath = $null
    )
    $out = Join-Path $localWork "$Name.out.log"
    $err = Join-Path $localWork "$Name.err.log"
    $cleanArgs = @("-N", "-D", "127.0.0.1:$Port", "-o", "ExitOnForwardFailure=yes")
    if (-not [string]::IsNullOrWhiteSpace($ControlPath)) {
        $cleanArgs += @("-S", $ControlPath, "-o", "ControlMaster=no", "-o", "ControlPersist=no")
    }
    $cleanArgs += (Get-SshOptions) + @($Target)
    $cleanArgs = $cleanArgs | Where-Object { $_ -ne $null -and $_ -ne "" }
    return Start-Process -FilePath "ssh.exe" `
        -ArgumentList $cleanArgs `
        -PassThru -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError $err
}

function Get-SshControlPath {
    param([string]$Target, [string]$Mode)
    $safeTarget = Get-SafeTargetName -Target $Target
    return (Join-Path $localWork "sshcm-$safeTarget-$Mode.sock")
}

function Invoke-SshControlCommand {
    param([string]$Target, [string]$ControlPath, [string]$Command)
    $args = @("-S", $ControlPath, "-O", $Command) + (Get-SshOptions) + @($Target)
    & ssh.exe @args 2>$null | Out-Null
    return $LASTEXITCODE
}

function Start-SshControlMaster {
    param([string]$Target, [string]$ControlPath, [string]$Name)
    if (Test-Path -LiteralPath $ControlPath) {
        Remove-Item -LiteralPath $ControlPath -Force -ErrorAction SilentlyContinue
    }
    $out = Join-Path $localWork "$Name.out.log"
    $err = Join-Path $localWork "$Name.err.log"
    $args = @(
        "-N",
        "-M",
        "-S", $ControlPath,
        "-o", "ControlMaster=yes",
        "-o", "ControlPersist=$SshControlPersistSecs",
        "-o", "ExitOnForwardFailure=yes"
    ) + (Get-SshOptions) + @($Target)
    $proc = Start-Process -FilePath "ssh.exe" `
        -ArgumentList $args `
        -PassThru -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError $err
    for ($i = 0; $i -lt 40; $i++) {
        if ($proc.HasExited) {
            $detail = Get-ProcessLogSummary -Name $Name
            if ($detail -match "getsockname failed: Not a socket") {
                throw "local OpenSSH ControlMaster is not usable on this platform/client; $detail"
            }
            throw "ssh ControlMaster exited before ready; $detail"
        }
        if ((Invoke-SshControlCommand -Target $Target -ControlPath $ControlPath -Command "check") -eq 0) {
            return $proc
        }
        Start-Sleep -Milliseconds 500
    }
    $detail = Get-ProcessLogSummary -Name $Name
    Stop-ProcessQuiet $proc
    if ($detail -match "getsockname failed: Not a socket") {
        throw "local OpenSSH ControlMaster is not usable on this platform/client; $detail"
    }
    throw "ssh ControlMaster did not become ready at $ControlPath; $detail"
}

function Stop-SshControlMaster {
    param([string]$Target, [string]$ControlPath, $Process)
    if (-not [string]::IsNullOrWhiteSpace($ControlPath)) {
        Invoke-SshControlCommand -Target $Target -ControlPath $ControlPath -Command "exit" | Out-Null
    }
    Stop-ProcessQuiet $Process
    if (-not [string]::IsNullOrWhiteSpace($ControlPath) -and (Test-Path -LiteralPath $ControlPath)) {
        Remove-Item -LiteralPath $ControlPath -Force -ErrorAction SilentlyContinue
    }
}

function Stop-ProcessQuiet {
    param($Process)
    if ($null -ne $Process -and -not $Process.HasExited) {
        Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
        $Process.WaitForExit(5000) | Out-Null
    }
}

function Invoke-CurlDownload {
    param([int]$Port, [string]$Url, [string[]]$Extra = @(), [int]$MaxTime = 240)
    & curl.exe -fsSL --proxy "socks5h://127.0.0.1:$Port" --max-time $MaxTime @Extra -o NUL $Url
    return $LASTEXITCODE
}

function Get-ProxyStatus {
    param([int]$ControlPort)
    if ($ControlPort -le 0) {
        return $null
    }
    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $task = $client.ConnectAsync("127.0.0.1", $ControlPort)
        if (-not $task.Wait(1000) -or -not $client.Connected) {
            return @{ ok = $false; error = "control connect timed out" }
        }
        $stream = $client.GetStream()
        $bytes = [System.Text.Encoding]::UTF8.GetBytes("status`n")
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush()
        $reader = [System.IO.StreamReader]::new($stream, [System.Text.Encoding]::UTF8)
        $text = $reader.ReadToEnd()
        if ([string]::IsNullOrWhiteSpace($text)) {
            return @{ ok = $false; error = "empty control response" }
        }
        return $text | ConvertFrom-Json
    }
    catch {
        return @{ ok = $false; error = $_.Exception.Message }
    }
    finally {
        $client.Dispose()
    }
}

function Get-StatusValue {
    param($Status, [string]$Name)
    if ($null -eq $Status) {
        return $null
    }
    if ($Status -is [hashtable] -and $Status.ContainsKey("ok") -and -not $Status.ok) {
        return $null
    }
    $property = $Status.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Get-StatusLinkHealthValue {
    param($Status, [string]$Name)
    $link = Get-StatusValue -Status $Status -Name "link"
    if ($null -eq $link) {
        return $null
    }
    $healthProperty = $link.PSObject.Properties["health"]
    if ($null -eq $healthProperty -or $null -eq $healthProperty.Value) {
        return $null
    }
    $property = $healthProperty.Value.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Get-ProcessProfileSample {
    param($Process)
    if ($null -eq $Process) {
        return $null
    }
    try {
        $Process.Refresh()
        return [pscustomobject]@{
            pid = $Process.Id
            total_cpu_ms = [Math]::Round($Process.TotalProcessorTime.TotalMilliseconds, 3)
            user_cpu_ms = [Math]::Round($Process.UserProcessorTime.TotalMilliseconds, 3)
            privileged_cpu_ms = [Math]::Round($Process.PrivilegedProcessorTime.TotalMilliseconds, 3)
            working_set_bytes = $Process.WorkingSet64
            peak_working_set_bytes = $Process.PeakWorkingSet64
            private_memory_bytes = $Process.PrivateMemorySize64
        }
    }
    catch {
        return [pscustomobject]@{
            pid = $null
            total_cpu_ms = $null
            user_cpu_ms = $null
            privileged_cpu_ms = $null
            working_set_bytes = $null
            peak_working_set_bytes = $null
            private_memory_bytes = $null
            error = $_.Exception.Message
        }
    }
}

function Get-ProfileDelta {
    param($Before, $After, [string]$Name)
    if ($null -eq $Before -or $null -eq $After) {
        return $null
    }
    $beforeProperty = $Before.PSObject.Properties[$Name]
    $afterProperty = $After.PSObject.Properties[$Name]
    if ($null -eq $beforeProperty -or $null -eq $afterProperty) {
        return $null
    }
    if ($null -eq $beforeProperty.Value -or $null -eq $afterProperty.Value) {
        return $null
    }
    return [Math]::Round(([double]$afterProperty.Value - [double]$beforeProperty.Value), 3)
}

function Convert-StatusJson {
    param($Value)
    if ($null -eq $Value) {
        return $null
    }
    return $Value | ConvertTo-Json -Depth 8 -Compress
}

function Join-StatusValues {
    param($Values)
    $items = @($Values | Where-Object { $null -ne $_ } | ForEach-Object { [string]$_ })
    if ($items.Count -eq 0) {
        return $null
    }
    return $items -join ";"
}

function Get-QuicConnectionSummary {
    param($Status, [string]$PropertyName)
    $connections = Get-StatusValue -Status $Status -Name "quic_connections"
    if ($null -eq $connections) {
        return $null
    }
    return Join-StatusValues (@($connections | ForEach-Object {
        $property = $_.PSObject.Properties[$PropertyName]
        if ($null -eq $property) {
            return $null
        }
        $property.Value
    }))
}

function Get-QuicProfileObject {
    param(
        [string]$Target,
        [string]$Case,
        $Status,
        $ProcessBefore,
        $ProcessAfter
    )
    if (-not $QuicProfile -or -not (Test-QuicBenchmarkCase -Case $Case)) {
        return $null
    }
    $statusProfile = Get-StatusValue -Status $Status -Name "quic_profile"
    $transport = [pscustomobject]@{
        max_bidi_streams = Get-StatusValue -Status $Status -Name "quic_max_bidi_streams"
        stream_receive_window = Get-StatusValue -Status $Status -Name "quic_stream_receive_window"
        receive_window = Get-StatusValue -Status $Status -Name "quic_receive_window"
        keep_alive_interval_secs = Get-StatusValue -Status $Status -Name "quic_keep_alive_interval_secs"
        idle_timeout_secs = Get-StatusValue -Status $Status -Name "quic_idle_timeout_secs"
    }
    $udp = [pscustomobject]@{
        runtime = Get-StatusValue -Status $Status -Name "quic_udp_runtime"
        gso = Get-StatusValue -Status $Status -Name "quic_udp_gso"
        gso_source = Get-StatusValue -Status $Status -Name "quic_udp_gso_source"
        packetization = Get-StatusValue -Status $Status -Name "quic_packetization"
        max_datagram_size = $null
        max_datagram_size_source = "unavailable: Quinn endpoint API is not exposed through ssh_proxy status"
        packet_loss = $null
        packet_loss_source = "unavailable: Quinn connection loss counters are not exposed through ssh_proxy status"
    }
    $process = [pscustomobject]@{
        before = $ProcessBefore
        after = $ProcessAfter
        total_cpu_delta_ms = Get-ProfileDelta -Before $ProcessBefore -After $ProcessAfter -Name "total_cpu_ms"
        user_cpu_delta_ms = Get-ProfileDelta -Before $ProcessBefore -After $ProcessAfter -Name "user_cpu_ms"
        privileged_cpu_delta_ms = Get-ProfileDelta -Before $ProcessBefore -After $ProcessAfter -Name "privileged_cpu_ms"
        working_set_delta_bytes = Get-ProfileDelta -Before $ProcessBefore -After $ProcessAfter -Name "working_set_bytes"
        private_memory_delta_bytes = Get-ProfileDelta -Before $ProcessBefore -After $ProcessAfter -Name "private_memory_bytes"
    }
    [pscustomobject]@{
        enabled = $true
        target = $Target
        case = $Case
        profile_scope = "local-proxy-process-and-route-status"
        local_platform = $script:LocalPlatform
        remote_platform = [pscustomobject]@{
            os = $script:CurrentRemoteOs
            arch = $script:CurrentRemoteArch
        }
        route_status = $statusProfile
        socket = [pscustomobject]@{
            local_proxy = "127.0.0.1"
            remote_quic = Get-StatusValue -Status $Status -Name "remote_quic"
            selected_protocol = Get-StatusProtocolValue -Status $Status
        }
        transport = if ($null -ne $statusProfile -and $null -ne $statusProfile.transport) { $statusProfile.transport } else { $transport }
        runtime = if ($null -ne $statusProfile) { $statusProfile.runtime } else { $null }
        udp = if ($null -ne $statusProfile -and $null -ne $statusProfile.udp) { $statusProfile.udp } else { $udp }
        connections = if ($null -ne $statusProfile) { $statusProfile.connections } else { $null }
        flow = if ($null -ne $statusProfile) { $statusProfile.flow } else { $null }
        control = if ($null -ne $statusProfile) { $statusProfile.control } else { $null }
        signals = if ($null -ne $statusProfile) { $statusProfile.signals } else { $null }
        process = $process
    }
}

function Get-StabilityReconnectSignal {
    param($Status)
    if ($null -eq $Status) {
        return 0
    }
    $connectAttempts = Convert-BenchInt (Get-StatusValue -Status $Status -Name "connect_attempts") 0
    $sshSessionAttempts = Convert-BenchInt (Get-StatusValue -Status $Status -Name "ssh_session_connect_attempts") 0
    $quicStreamFailures = Convert-BenchInt (Get-StatusValue -Status $Status -Name "quic_stream_open_failures") 0
    $quicCopyFailures = Convert-BenchInt (Get-StatusValue -Status $Status -Name "quic_copy_failures") 0
    return $connectAttempts + $sshSessionAttempts + $quicStreamFailures + $quicCopyFailures
}

function Test-StabilityStatusDegraded {
    param($Status)
    if ($null -eq $Status) {
        return $true
    }
    if ($Status -is [hashtable] -and $Status.ContainsKey("ok") -and -not $Status.ok) {
        return $true
    }
    $connected = Get-StatusLinkHealthValue -Status $Status -Name "connected"
    if ($null -ne $connected -and ([string]$connected).ToLowerInvariant() -eq "false") {
        return $true
    }
    $degradedReason = Get-StatusLinkHealthValue -Status $Status -Name "degraded_reason"
    if (-not [string]::IsNullOrWhiteSpace([string]$degradedReason)) {
        return $true
    }
    $controlHealth = Get-StatusLinkHealthValue -Status $Status -Name "control_health"
    if (-not [string]::IsNullOrWhiteSpace([string]$controlHealth) -and [string]$controlHealth -ne "healthy") {
        return $true
    }
    $controlDegraded = Get-StatusValue -Status $Status -Name "control_degraded"
    return ($null -ne $controlDegraded -and ([string]$controlDegraded).ToLowerInvariant() -eq "true")
}

function Start-StabilityLargeDownload {
    param([int]$Port, [string]$CaseUrl, [int]$MaxTimeSeconds)
    $args = @(
        "-fsSL",
        "--proxy", "socks5h://127.0.0.1:$Port",
        "--max-time", [string]$MaxTimeSeconds,
        "-o", "NUL",
        $CaseUrl
    )
    Start-Process -FilePath "curl.exe" -ArgumentList $args -PassThru -WindowStyle Hidden
}

function Restart-RemoteBenchmarkDaemon {
    param($Plan)
    if ($null -eq $Plan) {
        throw "remote daemon restart requested without a remote plan"
    }
    $token = "bench-$($Plan.stamp)-$(Get-SafeTargetName -Target $Plan.target)"
    $binary = Quote-RemoteShellValue $Plan.paths.binary
    $home = Quote-RemoteShellValue $Plan.paths.home
    $cert = Quote-RemoteShellValue $Plan.paths.cert
    $key = Quote-RemoteShellValue $Plan.paths.key
    $routes = Quote-RemoteShellValue $Plan.paths.routes
    $log = Quote-RemoteShellValue $Plan.paths.daemon_log
    $pid = Quote-RemoteShellValue $Plan.pid_files.daemon
    $command = @"
if [ -f $pid ]; then oldpid=`$(cat $pid); kill "`$oldpid" 2>/dev/null || true; sleep 1; kill -9 "`$oldpid" 2>/dev/null || true; fi
chmod 700 $binary
SSH_PROXY_HOME=$home nohup $binary --log '$(Get-QuicLogFilter)' node daemon --control tcp://127.0.0.1:$($Plan.control) --transport 0.0.0.0:$($Plan.plain) --tls-transport 0.0.0.0:$($Plan.tls) --quic-transport 0.0.0.0:$($Plan.quic) --quic-max-bidi-streams $QuicMaxBidiStreams --quic-stream-receive-window $QuicStreamReceiveWindow --quic-receive-window $QuicReceiveWindow --quic-keep-alive-interval-secs $QuicKeepAliveIntervalSecs --quic-idle-timeout-secs $QuicIdleTimeoutSecs --tls-cert $cert --tls-key $key --token '$token' --routes-path $routes --no-route-autostart >> $log 2>&1 < /dev/null & echo `$! > $pid
for i in 1 2 3 4 5 6 7 8 9 10; do $binary --log warn node control --endpoint tcp://127.0.0.1:$($Plan.control) --token '$token' status >/dev/null 2>&1 && exit 0; sleep 1; done
exit 1
"@
    Invoke-Remote $Plan.target $command
}

function Get-StatusProtocolValue {
    param($Status)
    $protocol = Get-StatusLinkHealthValue -Status $Status -Name "selected_protocol"
    if ($null -ne $protocol -and -not [string]::IsNullOrWhiteSpace([string]$protocol)) {
        return [string]$protocol
    }
    $protocol = Get-StatusValue -Status $Status -Name "selected_protocol"
    if ($null -ne $protocol -and -not [string]::IsNullOrWhiteSpace([string]$protocol)) {
        return [string]$protocol
    }
    $workers = Get-StatusValue -Status $Status -Name "workers"
    if ($null -ne $workers) {
        $protocols = @($workers | Where-Object { $_.selected_protocol } | ForEach-Object { [string]$_.selected_protocol } | Select-Object -Unique)
        if ($protocols.Count -eq 1) {
            return $protocols[0]
        }
        if ($protocols.Count -gt 1) {
            return "mixed"
        }
    }
    return $null
}

function Get-PlanCaptureValue {
    param($PlanCapture, [string]$Name)
    if ($null -eq $PlanCapture) {
        return $null
    }
    $property = $PlanCapture.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Get-PlanValue {
    param($Plan, [string]$Name)
    if ($null -eq $Plan) {
        return $null
    }
    $property = $Plan.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Get-PlanDecisionValue {
    param($Plan, [string]$Name)
    $chain = Get-PlanValue -Plan $Plan -Name "decision_chain"
    if ($null -eq $chain) {
        return $null
    }
    return Get-PlanValue -Plan $chain -Name $Name
}

function Get-PlanDecisionPathValue {
    param($Plan, [string]$Path)
    $chain = Get-PlanValue -Plan $Plan -Name "decision_chain"
    if ($null -eq $chain) {
        return $null
    }
    $current = $chain
    foreach ($part in ($Path -split '\.')) {
        if ($null -eq $current) {
            return $null
        }
        $current = Get-PlanValue -Plan $current -Name $part
    }
    return $current
}

function Get-PreflightRecommendedFallback {
    param($Plan)
    $preflight = Get-PlanValue -Plan $Plan -Name "preflight"
    if ($null -eq $preflight) {
        return $null
    }
    return Get-PlanValue -Plan $preflight -Name "recommended_fallback"
}

function Get-PreflightFailureSummary {
    param($Plan)
    $preflight = Get-PlanValue -Plan $Plan -Name "preflight"
    if ($null -eq $preflight) {
        return @()
    }
    $results = Get-PlanValue -Plan $preflight -Name "results"
    if ($null -eq $results) {
        return @()
    }
    return @($results | Where-Object { $_.reachable -eq $false } | ForEach-Object {
        $protocol = Get-PlanValue -Plan $_ -Name "protocol"
        $status = Get-PlanValue -Plan $_ -Name "status"
        $endpoint = Get-PlanValue -Plan $_ -Name "endpoint"
        "${protocol}:${status}:${endpoint}"
    })
}

function Get-PreflightFailureEndpoints {
    param($Plan)
    $preflight = Get-PlanValue -Plan $Plan -Name "preflight"
    if ($null -eq $preflight) {
        return @()
    }
    $results = Get-PlanValue -Plan $preflight -Name "results"
    if ($null -eq $results) {
        return @()
    }
    return @($results | Where-Object { $_.reachable -eq $false } | ForEach-Object {
        Get-PlanValue -Plan $_ -Name "endpoint"
    } | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) })
}

function Get-ProxyArgValue {
    param([string[]]$ProxyArgs, [string]$Name)
    for ($i = 0; $i -lt ($ProxyArgs.Count - 1); $i++) {
        if ($ProxyArgs[$i] -eq $Name) {
            return [string]$ProxyArgs[$i + 1]
        }
    }
    return $null
}

function Get-PreflightNegativeCacheKey {
    param([string]$Target, [string]$Case, [string[]]$ProxyArgs)
    $protocol = Get-CaseDirectProtocol -Case $Case
    if ([string]::IsNullOrWhiteSpace([string]$protocol)) {
        return $null
    }
    $endpoint = switch ($protocol) {
        "plain-tcp" { Get-ProxyArgValue -ProxyArgs $ProxyArgs -Name "--remote-tcp" }
        "tls-tcp" { Get-ProxyArgValue -ProxyArgs $ProxyArgs -Name "--remote-tls" }
        "quic-framed" { Get-ProxyArgValue -ProxyArgs $ProxyArgs -Name "--remote-quic" }
        "quic-native" { Get-ProxyArgValue -ProxyArgs $ProxyArgs -Name "--remote-quic" }
        default { $null }
    }
    if ([string]::IsNullOrWhiteSpace([string]$endpoint)) {
        return $null
    }
    return "$Target|$protocol|$endpoint"
}

function New-CachedRoutePlanCapture {
    param([string]$CacheKey, $CachedCapture)
    $parts = $CacheKey -split '\|', 3
    [pscustomobject]@{
        ok = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "ok"
        plan_before = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_before"
        plan_selected_transport = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_selected_transport"
        plan_recommended_fallback = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_recommended_fallback"
        plan_fallback_reason = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_fallback_reason"
        plan_next_action = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_next_action"
        plan_ssh_data_plane_reason = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_ssh_data_plane_reason"
        plan_decision_chain = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_decision_chain"
        plan_decision_topology_class = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_decision_topology_class"
        plan_decision_selected_reason = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_decision_selected_reason"
        plan_decision_repair_hint = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_decision_repair_hint"
        plan_decision_explicit_user_override = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_decision_explicit_user_override"
        plan_candidate_failures = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_candidate_failures"
        plan_candidate_failure_endpoints = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_candidate_failure_endpoints"
        plan_topology_failure = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_topology_failure"
        plan_error = Get-PlanCaptureValue -PlanCapture $CachedCapture -Name "plan_error"
        preflight_cache_hit = $true
        preflight_cache_key = $CacheKey
        preflight_cache_protocol = if ($parts.Count -ge 2) { $parts[1] } else { $null }
        preflight_cache_endpoint = if ($parts.Count -ge 3) { $parts[2] } else { $null }
    }
}

function Get-RoutePlanCapture {
    param([string]$Target, [string]$Case, [string[]]$ProxyArgs, [int]$Port)
    $routeProxyArgs = @($ProxyArgs)
    for ($i = 0; $i -lt ($routeProxyArgs.Count - 1); $i++) {
        if ($routeProxyArgs[$i] -eq "--remote-transport" -and $routeProxyArgs[$i + 1] -eq "plain-tcp" -and -not ($routeProxyArgs -contains "--allow-plain-tcp")) {
            $routeProxyArgs += "--allow-plain-tcp"
            break
        }
    }
    $planArgs = @(
        "--log", "warn",
        "route", $Target,
        "--direction", "local-uses-remote",
        "--port", $Port.ToString(),
        "--bind", "127.0.0.1",
        "--deploy", "never",
        "--no-reconnect",
        "--connect-timeout-secs", "20",
        "--transport-pool-size", $script:CurrentTransportPoolSize.ToString(),
        "--explain"
    ) + $routeProxyArgs
    try {
        $text = Invoke-LocalCapture -ArgumentList $planArgs -Label "$Target-$Case-plan" -TimeoutSeconds 30
        $plan = $text | ConvertFrom-Json
        $failures = @(Get-PreflightFailureSummary -Plan $plan)
        $failureEndpoints = @(Get-PreflightFailureEndpoints -Plan $plan)
        $topologyFailure = if ($failures.Count -gt 0) { "direct_endpoint_unreachable" } else { $null }
        return [pscustomobject]@{
            ok = $true
            plan_before = $plan | ConvertTo-Json -Depth 10 -Compress
            plan_selected_transport = Get-PlanValue -Plan $plan -Name "selected_transport"
            plan_recommended_fallback = Get-PreflightRecommendedFallback -Plan $plan
            plan_fallback_reason = Get-PlanValue -Plan $plan -Name "fallback_reason"
            plan_next_action = Get-PlanValue -Plan $plan -Name "next_action"
            plan_ssh_data_plane_reason = Get-PlanValue -Plan $plan -Name "ssh_data_plane_reason"
            plan_decision_chain = (Get-PlanValue -Plan $plan -Name "decision_chain") | ConvertTo-Json -Depth 10 -Compress
            plan_decision_topology_class = Get-PlanDecisionPathValue -Plan $plan -Path "topology.class"
            plan_decision_selected_reason = Get-PlanDecisionValue -Plan $plan -Name "selected_reason"
            plan_decision_repair_hint = Get-PlanDecisionPathValue -Plan $plan -Path "preflight.repair_hint"
            plan_decision_explicit_user_override = Get-PlanDecisionPathValue -Plan $plan -Path "policy.explicit_user_override"
            plan_candidate_failures = if ($failures.Count -gt 0) { $failures -join ";" } else { $null }
            plan_candidate_failure_endpoints = if ($failureEndpoints.Count -gt 0) { $failureEndpoints -join ";" } else { $null }
            plan_topology_failure = $topologyFailure
            plan_error = $null
            preflight_cache_hit = $false
            preflight_cache_key = $null
            preflight_cache_protocol = $null
            preflight_cache_endpoint = $null
        }
    }
    catch {
        return [pscustomobject]@{
            ok = $false
            plan_before = $null
            plan_selected_transport = $null
            plan_recommended_fallback = $null
            plan_fallback_reason = $null
            plan_next_action = $null
            plan_ssh_data_plane_reason = $null
            plan_decision_chain = $null
            plan_decision_topology_class = $null
            plan_decision_selected_reason = $null
            plan_decision_repair_hint = $null
            plan_decision_explicit_user_override = $null
            plan_candidate_failures = $null
            plan_candidate_failure_endpoints = $null
            plan_topology_failure = $null
            plan_error = $_.Exception.Message
            preflight_cache_hit = $false
            preflight_cache_key = $null
            preflight_cache_protocol = $null
            preflight_cache_endpoint = $null
        }
    }
}

function Get-CachedOrFreshRoutePlanCapture {
    param([string]$Target, [string]$Case, [string[]]$ProxyArgs, [int]$Port)
    $cacheKey = Get-PreflightNegativeCacheKey -Target $Target -Case $Case -ProxyArgs $ProxyArgs
    if ($RespectPreflightSkip -and -not [string]::IsNullOrWhiteSpace([string]$cacheKey) -and $script:PreflightNegativeCache.ContainsKey($cacheKey)) {
        return New-CachedRoutePlanCapture -CacheKey $cacheKey -CachedCapture $script:PreflightNegativeCache[$cacheKey]
    }

    $capture = Get-RoutePlanCapture -Target $Target -Case $Case -ProxyArgs $ProxyArgs -Port $Port
    if ($RespectPreflightSkip `
        -and -not [string]::IsNullOrWhiteSpace([string]$cacheKey) `
        -and ((Get-PlanCaptureValue -PlanCapture $capture -Name "plan_topology_failure") -eq "direct_endpoint_unreachable")) {
        $script:PreflightNegativeCache[$cacheKey] = $capture
    }
    return $capture
}

function Measure-Case {
    param(
        [string]$Target,
        [string]$Case,
        [int]$Port,
        [string]$CaseUrl,
        [int]$ControlPort = 0,
        $PlanCapture = $null,
        [string]$OpenSshControlMasterMode = $null,
        [string]$OpenSshControlPath = $null,
        [int]$OpenSshControlPersistSecs = 0,
        [bool]$OpenSshControlMasterReused = $false,
        $ProfileProcess = $null
    )
    $quicProfileEnabled = [bool]($QuicProfile -and (Test-QuicBenchmarkCase -Case $Case))
    $statusBefore = Get-ProxyStatus -ControlPort $ControlPort
    $processBefore = if ($quicProfileEnabled) { Get-ProcessProfileSample -Process $ProfileProcess } else { $null }
    $warm = Invoke-CurlDownload -Port $Port -Url $CaseUrl -Extra @("-r", "0-65535") -MaxTime 60
    if ($warm -ne 0) {
        throw "warmup curl failed with exit code $warm"
    }

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $largeExit = Invoke-CurlDownload -Port $Port -Url $CaseUrl -MaxTime 300
    $sw.Stop()
    $largeSeconds = [Math]::Round($sw.Elapsed.TotalSeconds, 3)
    $largeMibps = if ($largeExit -eq 0 -and $largeSeconds -gt 0) {
        [Math]::Round(100.0 / $largeSeconds, 3)
    } else {
        0
    }

    $rangeArgs = @("-fsSL", "--proxy", "socks5h://127.0.0.1:$Port", "--max-time", "120", "-r", "0-1048575", "-o", "NUL", $CaseUrl)
    $sw.Restart()
    $procs = for ($i = 0; $i -lt $Concurrency; $i++) {
        Start-Process -FilePath "curl.exe" -ArgumentList $rangeArgs -PassThru -WindowStyle Hidden
    }
    Wait-Process -InputObject $procs
    foreach ($proc in $procs) {
        $proc.Refresh()
    }
    $sw.Stop()
    $ok = @($procs | Where-Object { $_.ExitCode -eq 0 }).Count
    $concurrentSeconds = [Math]::Round($sw.Elapsed.TotalSeconds, 3)
    $concurrentMibps = if ($concurrentSeconds -gt 0) {
        [Math]::Round($ok / $concurrentSeconds, 3)
    } else {
        0
    }
    $statusAfter = Get-ProxyStatus -ControlPort $ControlPort
    $processAfter = if ($quicProfileEnabled) { Get-ProcessProfileSample -Process $ProfileProcess } else { $null }
    $quicProfileValue = Get-QuicProfileObject -Target $Target -Case $Case -Status $statusAfter -ProcessBefore $processBefore -ProcessAfter $processAfter
    $runtimeFailure = $null
    if ($largeExit -ne 0) {
        $runtimeFailure = Get-BenchmarkFailureClassification -Message "large curl failed with exit code $largeExit"
    } elseif ($ok -lt $Concurrency) {
        $runtimeFailure = Get-BenchmarkFailureClassification -Message "concurrent curl completed $ok/$Concurrency requests"
    }
    $baselineMode = Get-BaselineMode -Case $Case -OpenSshControlMasterMode $OpenSshControlMasterMode
    $baselineQuality = Get-BaselineQuality `
        -Case $Case `
        -ErrorKind $(if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.error_kind }) `
        -Error $(if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.human_summary }) `
        -OpenSshControlMasterMode $OpenSshControlMasterMode

    [pscustomobject]@{
        target = $Target
        case = $Case
        run_level = $script:CurrentRunLevel
        selected_protocol = Get-StatusProtocolValue -Status $statusAfter
        link_selected_protocol = Get-StatusLinkHealthValue -Status $statusAfter -Name "selected_protocol"
        link_active_connections = Get-StatusLinkHealthValue -Status $statusAfter -Name "active_connections"
        link_active_streams = Get-StatusLinkHealthValue -Status $statusAfter -Name "active_streams"
        link_active_channels = Get-StatusLinkHealthValue -Status $statusAfter -Name "active_channels"
        link_open_attempts = Get-StatusLinkHealthValue -Status $statusAfter -Name "open_attempts"
        link_open_successes = Get-StatusLinkHealthValue -Status $statusAfter -Name "open_successes"
        link_open_failures = Get-StatusLinkHealthValue -Status $statusAfter -Name "open_failures"
        link_open_latency_ms = Get-StatusLinkHealthValue -Status $statusAfter -Name "open_latency_ms"
        link_bytes_client_to_remote = Get-StatusLinkHealthValue -Status $statusAfter -Name "bytes_client_to_remote"
        link_bytes_remote_to_client = Get-StatusLinkHealthValue -Status $statusAfter -Name "bytes_remote_to_client"
        link_first_byte_samples = Get-StatusLinkHealthValue -Status $statusAfter -Name "first_byte_samples"
        link_first_byte_latency_ms = Get-StatusLinkHealthValue -Status $statusAfter -Name "first_byte_latency_ms"
        link_max_first_byte_latency_ms = Get-StatusLinkHealthValue -Status $statusAfter -Name "max_first_byte_latency_ms"
        link_last_close_reason = Get-StatusLinkHealthValue -Status $statusAfter -Name "last_close_reason"
        link_degraded_reason = Get-StatusLinkHealthValue -Status $statusAfter -Name "degraded_reason"
        link_healthy_workers = Get-StatusLinkHealthValue -Status $statusAfter -Name "healthy_workers"
        link_degraded_workers = Get-StatusLinkHealthValue -Status $statusAfter -Name "degraded_workers"
        link_reconnecting_workers = Get-StatusLinkHealthValue -Status $statusAfter -Name "reconnecting_workers"
        link_control_health = Get-StatusLinkHealthValue -Status $statusAfter -Name "control_health"
        link_connected = Get-StatusLinkHealthValue -Status $statusAfter -Name "connected"
        local_os = $script:LocalPlatform.os
        local_arch = $script:LocalPlatform.arch
        baseline_mode = $baselineMode
        baseline_quality = $baselineQuality.quality
        baseline_quality_reason = $baselineQuality.reason
        baseline_client_os = $script:OpenSshCapability.os
        baseline_client_arch = $script:OpenSshCapability.arch
        openssh_client_version = $script:OpenSshCapability.client_version
        openssh_controlmaster_supported = $script:OpenSshCapability.control_master_supported
        openssh_capability_notes = $script:OpenSshCapability.capability_notes
        remote_os = $script:CurrentRemoteOs
        remote_arch = $script:CurrentRemoteArch
        spx_frame_write_batches = Get-StatusValue -Status $statusAfter -Name "spx_frame_write_batches"
        spx_frame_write_flushes = Get-StatusValue -Status $statusAfter -Name "spx_frame_write_flushes"
        spx_frame_write_frames = Get-StatusValue -Status $statusAfter -Name "spx_frame_write_frames"
        spx_frame_write_data_frames = Get-StatusValue -Status $statusAfter -Name "spx_frame_write_data_frames"
        spx_frame_write_data_bytes = Get-StatusValue -Status $statusAfter -Name "spx_frame_write_data_bytes"
        spx_frame_write_vectored_writes = Get-StatusValue -Status $statusAfter -Name "spx_frame_write_vectored_writes"
        spx_frame_write_failures = Get-StatusValue -Status $statusAfter -Name "spx_frame_write_failures"
        spx_frame_read_frames = Get-StatusValue -Status $statusAfter -Name "spx_frame_read_frames"
        spx_frame_read_data_frames = Get-StatusValue -Status $statusAfter -Name "spx_frame_read_data_frames"
        spx_frame_read_data_bytes = Get-StatusValue -Status $statusAfter -Name "spx_frame_read_data_bytes"
        spx_tcp_stream_backpressure_timeouts = Get-StatusValue -Status $statusAfter -Name "spx_tcp_stream_backpressure_timeouts"
        spx_udp_assoc_backpressure_timeouts = Get-StatusValue -Status $statusAfter -Name "spx_udp_assoc_backpressure_timeouts"
        spx_healthy_workers = Get-StatusValue -Status $statusAfter -Name "healthy_workers"
        spx_degraded_workers = Get-StatusValue -Status $statusAfter -Name "degraded_workers"
        spx_reconnecting_workers = Get-StatusValue -Status $statusAfter -Name "reconnecting_workers"
        spx_pool_degraded_reason = Get-StatusValue -Status $statusAfter -Name "pool_degraded_reason"
        quic_profile_enabled = $quicProfileEnabled
        quic_profile_json = Convert-StatusJson $quicProfileValue
        quic_profile_parameter_set = if (Test-QuicBenchmarkCase -Case $Case) { Get-CurrentQuicParameterSet } else { $null }
        quic_profile_selected_protocol = if (Test-QuicBenchmarkCase -Case $Case) { Get-StatusProtocolValue -Status $statusAfter } else { $null }
        quic_profile_pool_size = if (Test-QuicBenchmarkCase -Case $Case) { $script:CurrentTransportPoolSize } else { $null }
        quic_profile_large_mibps = if (Test-QuicBenchmarkCase -Case $Case) { $largeMibps } else { $null }
        quic_profile_concurrent_mibps = if (Test-QuicBenchmarkCase -Case $Case) { $concurrentMibps } else { $null }
        quic_profile_failure_kind = if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.error_kind }
        quic_profile_control_health = if (Test-QuicBenchmarkCase -Case $Case) { Get-StatusLinkHealthValue -Status $statusAfter -Name "control_health" } else { $null }
        quic_profile_next_bottleneck = if ($null -ne $quicProfileValue) { $quicProfileValue.signals.next_bottleneck } else { $null }
        quic_profile_window_sizing_suspected = if ($null -ne $quicProfileValue) { $quicProfileValue.signals.window_sizing.suspected } else { $null }
        quic_profile_udp_path_suspected = if ($null -ne $quicProfileValue) { $quicProfileValue.signals.udp_path.suspected } else { $null }
        quic_profile_application_copy_suspected = if ($null -ne $quicProfileValue) { $quicProfileValue.signals.application_copy.suspected } else { $null }
        quic_profile_slow_consumers_suspected = if ($null -ne $quicProfileValue) { $quicProfileValue.signals.slow_consumers.suspected } else { $null }
        quic_profile_congestion_suspected = if ($null -ne $quicProfileValue) { $quicProfileValue.signals.congestion.suspected } else { $null }
        quic_profile_process_total_cpu_delta_ms = if ($null -ne $quicProfileValue) { $quicProfileValue.process.total_cpu_delta_ms } else { $null }
        quic_profile_process_user_cpu_delta_ms = if ($null -ne $quicProfileValue) { $quicProfileValue.process.user_cpu_delta_ms } else { $null }
        quic_profile_process_privileged_cpu_delta_ms = if ($null -ne $quicProfileValue) { $quicProfileValue.process.privileged_cpu_delta_ms } else { $null }
        quic_profile_process_working_set_bytes = if ($null -ne $processAfter) { $processAfter.working_set_bytes } else { $null }
        quic_profile_process_peak_working_set_bytes = if ($null -ne $processAfter) { $processAfter.peak_working_set_bytes } else { $null }
        quic_profile_process_private_memory_bytes = if ($null -ne $processAfter) { $processAfter.private_memory_bytes } else { $null }
        quic_profile_process_working_set_delta_bytes = if ($null -ne $quicProfileValue) { $quicProfileValue.process.working_set_delta_bytes } else { $null }
        quic_profile_process_private_memory_delta_bytes = if ($null -ne $quicProfileValue) { $quicProfileValue.process.private_memory_delta_bytes } else { $null }
        quic_profile_packet_loss = if ($null -ne $quicProfileValue) { $quicProfileValue.udp.packet_loss } else { $null }
        quic_profile_packet_loss_source = if ($null -ne $quicProfileValue) { $quicProfileValue.udp.packet_loss_source } else { $null }
        quic_profile_max_datagram_size = if ($null -ne $quicProfileValue) { $quicProfileValue.udp.max_datagram_size } else { $null }
        quic_profile_max_datagram_size_source = if ($null -ne $quicProfileValue) { $quicProfileValue.udp.max_datagram_size_source } else { $null }
        spx_profile_enabled = [bool]$SpxProfile
        spx_write_frames_per_batch = if ($SpxProfile) { Divide-BenchNumber (Get-StatusValue -Status $statusAfter -Name "spx_frame_write_frames") (Get-StatusValue -Status $statusAfter -Name "spx_frame_write_batches") } else { $null }
        spx_write_data_bytes_per_frame = if ($SpxProfile) { Divide-BenchNumber (Get-StatusValue -Status $statusAfter -Name "spx_frame_write_data_bytes") (Get-StatusValue -Status $statusAfter -Name "spx_frame_write_data_frames") } else { $null }
        spx_read_data_bytes_per_frame = if ($SpxProfile) { Divide-BenchNumber (Get-StatusValue -Status $statusAfter -Name "spx_frame_read_data_bytes") (Get-StatusValue -Status $statusAfter -Name "spx_frame_read_data_frames") } else { $null }
        spx_frame_flushes_per_batch = if ($SpxProfile) { Divide-BenchNumber (Get-StatusValue -Status $statusAfter -Name "spx_frame_write_flushes") (Get-StatusValue -Status $statusAfter -Name "spx_frame_write_batches") } else { $null }
        spx_vectored_writes_per_frame = if ($SpxProfile) { Divide-BenchNumber (Get-StatusValue -Status $statusAfter -Name "spx_frame_write_vectored_writes") (Get-StatusValue -Status $statusAfter -Name "spx_frame_write_frames") } else { $null }
        spx_relay_remote_to_client_mibps = if ($SpxProfile) { Divide-MibPerSecond (Get-StatusValue -Status $statusAfter -Name "last_spx_tcp_remote_to_client_bytes") (Get-StatusValue -Status $statusAfter -Name "last_spx_tcp_relay_duration_ms") } else { $null }
        spx_relay_client_to_remote_mibps = if ($SpxProfile) { Divide-MibPerSecond (Get-StatusValue -Status $statusAfter -Name "last_spx_tcp_client_to_remote_bytes") (Get-StatusValue -Status $statusAfter -Name "last_spx_tcp_relay_duration_ms") } else { $null }
        spx_tcp_relay_samples = Get-StatusValue -Status $statusAfter -Name "spx_tcp_relay_samples"
        last_spx_tcp_relay_duration_ms = Get-StatusValue -Status $statusAfter -Name "last_spx_tcp_relay_duration_ms"
        last_spx_tcp_client_to_remote_bytes = Get-StatusValue -Status $statusAfter -Name "last_spx_tcp_client_to_remote_bytes"
        last_spx_tcp_remote_to_client_bytes = Get-StatusValue -Status $statusAfter -Name "last_spx_tcp_remote_to_client_bytes"
        last_spx_tcp_relay_close_reason = Get-StatusValue -Status $statusAfter -Name "last_spx_tcp_relay_close_reason"
        ssh_direct_channel_open_samples = Get-StatusValue -Status $statusAfter -Name "ssh_direct_channel_open_samples"
        last_ssh_direct_channel_open_latency_ms = Get-StatusValue -Status $statusAfter -Name "last_ssh_direct_channel_open_latency_ms"
        spx_peer_handshake_samples = Get-StatusValue -Status $statusAfter -Name "spx_peer_handshake_samples"
        last_spx_peer_handshake_latency_ms = Get-StatusValue -Status $statusAfter -Name "last_spx_peer_handshake_latency_ms"
        direct_transport_policy = Get-CaseTransportPolicy -Case $Case
        direct_transport_policy_reason = Get-CaseTransportPolicyReason -Case $Case
        tls_peer_auth_mode = Get-StatusValue -Status $statusAfter -Name "tls_peer_auth_mode"
        openssh_control_master_mode = $OpenSshControlMasterMode
        openssh_control_path = $OpenSshControlPath
        openssh_control_persist_secs = if ($OpenSshControlPersistSecs -gt 0) { $OpenSshControlPersistSecs } else { $null }
        openssh_control_master_reused = $OpenSshControlMasterReused
        large_exit = $largeExit
        large_seconds = $largeSeconds
        large_mibps = $largeMibps
        concurrent = $Concurrency
        transport_pool_size = $script:CurrentTransportPoolSize
        concurrent_ok = $ok
        concurrent_seconds = $concurrentSeconds
        concurrent_mibps = $concurrentMibps
        plan_selected_transport = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_selected_transport"
        plan_recommended_fallback = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_recommended_fallback"
        plan_fallback_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_fallback_reason"
        plan_next_action = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_next_action"
        plan_ssh_data_plane_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_ssh_data_plane_reason"
        plan_decision_chain = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_chain"
        plan_decision_topology_class = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_topology_class"
        plan_decision_selected_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_selected_reason"
        plan_decision_repair_hint = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_repair_hint"
        plan_decision_explicit_user_override = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_explicit_user_override"
        plan_candidate_failures = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_candidate_failures"
        plan_candidate_failure_endpoints = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_candidate_failure_endpoints"
        plan_topology_failure = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_topology_failure"
        plan_error = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_error"
        active_quic_flows = Get-StatusValue -Status $statusAfter -Name "active_quic_flows"
        quic_stream_open_samples = Get-StatusValue -Status $statusAfter -Name "quic_stream_open_samples"
        last_quic_stream_open_latency_ms = Get-StatusValue -Status $statusAfter -Name "last_quic_stream_open_latency_ms"
        max_quic_stream_open_latency_ms = Get-StatusValue -Status $statusAfter -Name "max_quic_stream_open_latency_ms"
        quic_stream_open_failures = Get-StatusValue -Status $statusAfter -Name "quic_stream_open_failures"
        quic_stream_open_timeout_secs = Get-StatusValue -Status $statusAfter -Name "quic_stream_open_timeout_secs"
        quic_first_byte_timeout_secs = Get-StatusValue -Status $statusAfter -Name "quic_first_byte_timeout_secs"
        quic_header_write_samples = Get-StatusValue -Status $statusAfter -Name "quic_header_write_samples"
        last_quic_header_write_latency_ms = Get-StatusValue -Status $statusAfter -Name "last_quic_header_write_latency_ms"
        max_quic_header_write_latency_ms = Get-StatusValue -Status $statusAfter -Name "max_quic_header_write_latency_ms"
        quic_header_write_failures = Get-StatusValue -Status $statusAfter -Name "quic_header_write_failures"
        quic_copy_buffer_size = Get-StatusValue -Status $statusAfter -Name "quic_copy_buffer_size"
        quic_backpressure_timeout_secs = Get-StatusValue -Status $statusAfter -Name "quic_backpressure_timeout_secs"
        quic_backpressure_timeouts = Get-StatusValue -Status $statusAfter -Name "quic_backpressure_timeouts"
        quic_connection_pool_size = Get-StatusValue -Status $statusAfter -Name "quic_connection_pool_size"
        quic_connection_pool_policy = Get-StatusValue -Status $statusAfter -Name "quic_connection_pool_policy"
        quic_connection_pool_workload_hint = Get-StatusValue -Status $statusAfter -Name "quic_connection_pool_workload_hint"
        quic_connection_pool_reason = Get-StatusValue -Status $statusAfter -Name "quic_connection_pool_reason"
        quic_connection_pool_mode = Get-StatusValue -Status $statusAfter -Name "quic_connection_pool_mode"
        quic_connection_pool_selection_policy = Get-StatusValue -Status $statusAfter -Name "quic_connection_pool_selection_policy"
        active_quic_connections = Get-StatusValue -Status $statusAfter -Name "active_quic_connections"
        quic_flow_graceful_closes = Get-StatusValue -Status $statusAfter -Name "quic_flow_graceful_closes"
        quic_flow_resets = Get-StatusValue -Status $statusAfter -Name "quic_flow_resets"
        quic_flow_first_byte_samples = Get-StatusValue -Status $statusAfter -Name "quic_flow_first_byte_samples"
        last_quic_flow_first_byte_latency_ms = Get-StatusValue -Status $statusAfter -Name "last_quic_flow_first_byte_latency_ms"
        max_quic_flow_first_byte_latency_ms = Get-StatusValue -Status $statusAfter -Name "max_quic_flow_first_byte_latency_ms"
        quic_copy_duration_samples = Get-StatusValue -Status $statusAfter -Name "quic_copy_duration_samples"
        last_quic_copy_duration_ms = Get-StatusValue -Status $statusAfter -Name "last_quic_copy_duration_ms"
        max_quic_copy_duration_ms = Get-StatusValue -Status $statusAfter -Name "max_quic_copy_duration_ms"
        quic_copy_failures = Get-StatusValue -Status $statusAfter -Name "quic_copy_failures"
        last_quic_copy_client_to_remote_bytes = Get-StatusValue -Status $statusAfter -Name "last_quic_copy_client_to_remote_bytes"
        last_quic_copy_remote_to_client_bytes = Get-StatusValue -Status $statusAfter -Name "last_quic_copy_remote_to_client_bytes"
        max_quic_copy_client_to_remote_bytes = Get-StatusValue -Status $statusAfter -Name "max_quic_copy_client_to_remote_bytes"
        max_quic_copy_remote_to_client_bytes = Get-StatusValue -Status $statusAfter -Name "max_quic_copy_remote_to_client_bytes"
        last_quic_flow_close_reason = Get-StatusValue -Status $statusAfter -Name "last_quic_flow_close_reason"
        quic_max_bidi_streams = Get-StatusValue -Status $statusAfter -Name "quic_max_bidi_streams"
        quic_stream_receive_window = Get-StatusValue -Status $statusAfter -Name "quic_stream_receive_window"
        quic_receive_window = Get-StatusValue -Status $statusAfter -Name "quic_receive_window"
        quic_keep_alive_interval_secs = Get-StatusValue -Status $statusAfter -Name "quic_keep_alive_interval_secs"
        quic_idle_timeout_secs = Get-StatusValue -Status $statusAfter -Name "quic_idle_timeout_secs"
        quic_udp_runtime = Get-StatusValue -Status $statusAfter -Name "quic_udp_runtime"
        quic_udp_gso = Get-StatusValue -Status $statusAfter -Name "quic_udp_gso"
        quic_udp_gso_source = Get-StatusValue -Status $statusAfter -Name "quic_udp_gso_source"
        quic_packetization = Get-StatusValue -Status $statusAfter -Name "quic_packetization"
        quic_connections = Convert-StatusJson (Get-StatusValue -Status $statusAfter -Name "quic_connections")
        quic_connection_active_flows = Get-QuicConnectionSummary -Status $statusAfter -PropertyName "active_quic_flows"
        quic_connection_opened_flows = Get-QuicConnectionSummary -Status $statusAfter -PropertyName "opened_quic_flows"
        quic_connection_resets = Get-QuicConnectionSummary -Status $statusAfter -PropertyName "quic_flow_resets"
        quic_connection_control_states = Get-QuicConnectionSummary -Status $statusAfter -PropertyName "control_state"
        quic_connection_control_degraded = Get-QuicConnectionSummary -Status $statusAfter -PropertyName "control_degraded"
        control_state = Get-StatusValue -Status $statusAfter -Name "control_state"
        control_degraded = Get-StatusValue -Status $statusAfter -Name "control_degraded"
        control_pings_sent = Get-StatusValue -Status $statusAfter -Name "control_pings_sent"
        control_pongs_received = Get-StatusValue -Status $statusAfter -Name "control_pongs_received"
        last_control_pong_latency_ms = Get-StatusValue -Status $statusAfter -Name "last_control_pong_latency_ms"
        last_control_error = Get-StatusValue -Status $statusAfter -Name "last_control_error"
        ssh_mode = Get-StatusValue -Status $statusAfter -Name "ssh_mode"
        ssh_data_plane_reason = Get-StatusValue -Status $statusAfter -Name "ssh_data_plane_reason"
        ssh_session_pool_size = Get-StatusValue -Status $statusAfter -Name "ssh_session_pool_size"
        ssh_session_pool_source = Get-StatusValue -Status $statusAfter -Name "ssh_session_pool_source"
        ssh_session_pool_reason = Get-StatusValue -Status $statusAfter -Name "ssh_session_pool_reason"
        ssh_session_pool_warning = Get-StatusValue -Status $statusAfter -Name "ssh_session_pool_warning"
        ssh_session_growth_active_threshold = Get-StatusValue -Status $statusAfter -Name "ssh_session_growth_active_threshold"
        ssh_session_growth_events = Get-StatusValue -Status $statusAfter -Name "ssh_session_growth_events"
        ssh_session_growth_suppressed = Get-StatusValue -Status $statusAfter -Name "ssh_session_growth_suppressed"
        ssh_session_scheduler = Convert-StatusJson (Get-StatusValue -Status $statusAfter -Name "ssh_session_scheduler")
        ssh_sessions = Convert-StatusJson (Get-StatusValue -Status $statusAfter -Name "workers")
        active_ssh_sessions = Get-StatusValue -Status $statusAfter -Name "active_ssh_sessions"
        active_ssh_channels = Get-StatusValue -Status $statusAfter -Name "active_ssh_channels"
        ssh_session_connect_attempts = Get-StatusValue -Status $statusAfter -Name "ssh_session_connect_attempts"
        ssh_session_connect_failures = Get-StatusValue -Status $statusAfter -Name "ssh_session_connect_failures"
        ssh_channel_open_attempts = Get-StatusValue -Status $statusAfter -Name "ssh_channel_open_attempts"
        ssh_channel_open_failures = Get-StatusValue -Status $statusAfter -Name "ssh_channel_open_failures"
        ssh_first_byte_samples = Get-StatusValue -Status $statusAfter -Name "first_byte_samples"
        ssh_avg_first_byte_latency_ms = Get-StatusValue -Status $statusAfter -Name "avg_first_byte_latency_ms"
        last_ssh_first_byte_latency_ms = Get-StatusValue -Status $statusAfter -Name "last_first_byte_latency_ms"
        max_ssh_first_byte_latency_ms = Get-StatusValue -Status $statusAfter -Name "max_first_byte_latency_ms"
        p50_ssh_first_byte_latency_ms = Get-StatusValue -Status $statusAfter -Name "p50_first_byte_latency_ms"
        p95_ssh_first_byte_latency_ms = Get-StatusValue -Status $statusAfter -Name "p95_first_byte_latency_ms"
        ssh_last_channel_queue_depth = Get-StatusValue -Status $statusAfter -Name "last_channel_queue_depth"
        ssh_max_channel_queue_depth = Get-StatusValue -Status $statusAfter -Name "max_channel_queue_depth"
        ssh_graceful_closes = Get-StatusValue -Status $statusAfter -Name "graceful_closes"
        ssh_error_closes = Get-StatusValue -Status $statusAfter -Name "error_closes"
        last_ssh_close_reason = Get-StatusValue -Status $statusAfter -Name "last_close_reason"
        status_before = if ($null -eq $statusBefore) { $null } else { $statusBefore | ConvertTo-Json -Depth 8 -Compress }
        status_after = if ($null -eq $statusAfter) { $null } else { $statusAfter | ConvertTo-Json -Depth 8 -Compress }
        plan_before = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_before"
        skipped_by_preflight = $false
        skip_reason = $null
        skip_endpoint = $null
        ssh_vs_spx_overhead = $null
        primary_failure_stage = if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.primary_failure_stage }
        human_summary = if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.human_summary }
        error_kind = if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.error_kind }
        error = if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.human_summary }
    }
}

function Measure-StabilityCase {
    param(
        [string]$Target,
        [string]$Case,
        [int]$Port,
        [string]$CaseUrl,
        [int]$ControlPort,
        $PlanCapture = $null,
        $RemotePlan = $null
    )
    $statusBefore = Get-ProxyStatus -ControlPort $ControlPort
    $reconnectSignalBefore = Get-StabilityReconnectSignal -Status $statusBefore
    $smallRequests = 0
    $lostRequests = 0
    $latencyTotalMs = 0.0
    $maxLatencyMs = 0.0
    $degradedSamples = 0
    $degradedIntervals = 0
    $wasDegraded = $false
    $largeAttempts = 0
    $largeFailures = 0
    $restartInjected = $false
    $restartError = $null
    $largeProc = $null
    $started = [DateTime]::UtcNow
    $deadline = $started.AddSeconds($StabilityDurationSecs)
    $restartAt = $started.AddSeconds([Math]::Max(1, [int]($StabilityDurationSecs / 2)))

    try {
        $largeProc = Start-StabilityLargeDownload -Port $Port -CaseUrl $CaseUrl -MaxTimeSeconds ($StabilityDurationSecs + 120)
        $largeAttempts++
        while ([DateTime]::UtcNow -lt $deadline) {
            if ($null -ne $largeProc -and $largeProc.HasExited) {
                $largeProc.Refresh()
                if ($largeProc.ExitCode -ne 0) {
                    $largeFailures++
                }
                $largeProc.Dispose()
                $largeProc = Start-StabilityLargeDownload -Port $Port -CaseUrl $CaseUrl -MaxTimeSeconds ($StabilityDurationSecs + 120)
                $largeAttempts++
            }
            if ($StabilityInjectRemoteDaemonRestart -and -not $restartInjected -and [DateTime]::UtcNow -ge $restartAt) {
                try {
                    Restart-RemoteBenchmarkDaemon -Plan $RemotePlan
                }
                catch {
                    $restartError = $_.Exception.Message
                }
                $restartInjected = $true
            }
            $statusNow = Get-ProxyStatus -ControlPort $ControlPort
            $isDegraded = Test-StabilityStatusDegraded -Status $statusNow
            if ($isDegraded) {
                $degradedSamples++
                if (-not $wasDegraded) {
                    $degradedIntervals++
                }
            }
            $wasDegraded = $isDegraded

            $smallRequests++
            $requestSw = [System.Diagnostics.Stopwatch]::StartNew()
            $exit = Invoke-CurlDownload -Port $Port -Url $CaseUrl -Extra @("-r", "0-65535") -MaxTime ([Math]::Max(10, $StabilitySmallIntervalSecs + 5))
            $requestSw.Stop()
            $latencyMs = [Math]::Round($requestSw.Elapsed.TotalMilliseconds, 3)
            $latencyTotalMs += $latencyMs
            if ($latencyMs -gt $maxLatencyMs) {
                $maxLatencyMs = $latencyMs
            }
            if ($exit -ne 0) {
                $lostRequests++
            }
            $sleepMs = [Math]::Max(0, ($StabilitySmallIntervalSecs * 1000) - [int]$requestSw.Elapsed.TotalMilliseconds)
            if ($sleepMs -gt 0 -and [DateTime]::UtcNow.AddMilliseconds($sleepMs) -lt $deadline) {
                Start-Sleep -Milliseconds $sleepMs
            }
        }
    }
    finally {
        if ($null -ne $largeProc) {
            if ($largeProc.HasExited) {
                $largeProc.Refresh()
                if ($largeProc.ExitCode -ne 0) {
                    $largeFailures++
                }
            } else {
                Stop-ProcessQuiet $largeProc
            }
            $largeProc.Dispose()
        }
    }

    $statusAfter = Get-ProxyStatus -ControlPort $ControlPort
    $reconnectSignalAfter = Get-StabilityReconnectSignal -Status $statusAfter
    $runtimeFailure = $null
    if ($lostRequests -gt 0) {
        $runtimeFailure = Get-BenchmarkFailureClassification -Message "stability lost $lostRequests/$smallRequests small requests"
    } elseif ($largeFailures -gt 0) {
        $runtimeFailure = Get-BenchmarkFailureClassification -Message "stability rolling large transfer failed $largeFailures times"
    } elseif (-not [string]::IsNullOrWhiteSpace([string]$restartError)) {
        $runtimeFailure = Get-BenchmarkFailureClassification -Message $restartError -ForcedKind "control_degraded" -ForcedStage "control"
    }
    $actualDuration = [Math]::Round(([DateTime]::UtcNow - $started).TotalSeconds, 3)
    $baselineMode = Get-BaselineMode -Case $Case
    $baselineQuality = Get-BaselineQuality `
        -Case $Case `
        -ErrorKind $(if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.error_kind }) `
        -Error $(if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.human_summary })
    [pscustomobject]@{
        target = $Target
        case = $Case
        run_level = $script:CurrentRunLevel
        selected_protocol = Get-StatusProtocolValue -Status $statusAfter
        link_selected_protocol = Get-StatusLinkHealthValue -Status $statusAfter -Name "selected_protocol"
        link_active_connections = Get-StatusLinkHealthValue -Status $statusAfter -Name "active_connections"
        link_active_streams = Get-StatusLinkHealthValue -Status $statusAfter -Name "active_streams"
        link_active_channels = Get-StatusLinkHealthValue -Status $statusAfter -Name "active_channels"
        link_open_attempts = Get-StatusLinkHealthValue -Status $statusAfter -Name "open_attempts"
        link_open_successes = Get-StatusLinkHealthValue -Status $statusAfter -Name "open_successes"
        link_open_failures = Get-StatusLinkHealthValue -Status $statusAfter -Name "open_failures"
        link_open_latency_ms = Get-StatusLinkHealthValue -Status $statusAfter -Name "open_latency_ms"
        link_bytes_client_to_remote = Get-StatusLinkHealthValue -Status $statusAfter -Name "bytes_client_to_remote"
        link_bytes_remote_to_client = Get-StatusLinkHealthValue -Status $statusAfter -Name "bytes_remote_to_client"
        link_first_byte_samples = Get-StatusLinkHealthValue -Status $statusAfter -Name "first_byte_samples"
        link_first_byte_latency_ms = Get-StatusLinkHealthValue -Status $statusAfter -Name "first_byte_latency_ms"
        link_max_first_byte_latency_ms = Get-StatusLinkHealthValue -Status $statusAfter -Name "max_first_byte_latency_ms"
        link_last_close_reason = Get-StatusLinkHealthValue -Status $statusAfter -Name "last_close_reason"
        link_degraded_reason = Get-StatusLinkHealthValue -Status $statusAfter -Name "degraded_reason"
        link_control_health = Get-StatusLinkHealthValue -Status $statusAfter -Name "control_health"
        link_connected = Get-StatusLinkHealthValue -Status $statusAfter -Name "connected"
        local_os = $script:LocalPlatform.os
        local_arch = $script:LocalPlatform.arch
        baseline_mode = $baselineMode
        baseline_quality = $baselineQuality.quality
        baseline_quality_reason = $baselineQuality.reason
        baseline_client_os = $script:OpenSshCapability.os
        baseline_client_arch = $script:OpenSshCapability.arch
        openssh_client_version = $script:OpenSshCapability.client_version
        openssh_controlmaster_supported = $script:OpenSshCapability.control_master_supported
        openssh_capability_notes = $script:OpenSshCapability.capability_notes
        remote_os = $script:CurrentRemoteOs
        remote_arch = $script:CurrentRemoteArch
        large_exit = $null
        large_seconds = $null
        large_mibps = $null
        concurrent = $Concurrency
        transport_pool_size = $script:CurrentTransportPoolSize
        concurrent_ok = $null
        concurrent_seconds = $null
        concurrent_mibps = $null
        stability_duration_secs = $StabilityDurationSecs
        stability_actual_duration_secs = $actualDuration
        stability_small_interval_secs = $StabilitySmallIntervalSecs
        stability_small_requests = $smallRequests
        stability_lost_requests = $lostRequests
        stability_avg_latency_ms = if ($smallRequests -gt 0) { [Math]::Round($latencyTotalMs / $smallRequests, 3) } else { $null }
        stability_max_latency_ms = [Math]::Round($maxLatencyMs, 3)
        stability_degraded_samples = $degradedSamples
        stability_degraded_intervals = $degradedIntervals
        stability_reconnect_count = [Math]::Max(0, $reconnectSignalAfter - $reconnectSignalBefore)
        stability_large_attempts = $largeAttempts
        stability_large_failures = $largeFailures
        stability_remote_restart_requested = [bool]$StabilityInjectRemoteDaemonRestart
        stability_remote_restart_injected = $restartInjected
        stability_remote_restart_error = $restartError
        plan_selected_transport = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_selected_transport"
        plan_recommended_fallback = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_recommended_fallback"
        plan_fallback_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_fallback_reason"
        plan_next_action = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_next_action"
        plan_ssh_data_plane_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_ssh_data_plane_reason"
        plan_decision_chain = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_chain"
        plan_decision_topology_class = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_topology_class"
        plan_decision_selected_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_selected_reason"
        plan_decision_repair_hint = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_repair_hint"
        plan_decision_explicit_user_override = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_explicit_user_override"
        plan_candidate_failures = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_candidate_failures"
        plan_candidate_failure_endpoints = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_candidate_failure_endpoints"
        plan_topology_failure = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_topology_failure"
        plan_error = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_error"
        status_before = if ($null -eq $statusBefore) { $null } else { $statusBefore | ConvertTo-Json -Depth 8 -Compress }
        status_after = if ($null -eq $statusAfter) { $null } else { $statusAfter | ConvertTo-Json -Depth 8 -Compress }
        plan_before = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_before"
        skipped_by_preflight = $false
        skip_reason = $null
        skip_endpoint = $null
        primary_failure_stage = if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.primary_failure_stage }
        human_summary = if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.human_summary }
        error_kind = if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.error_kind }
        error = if ($null -eq $runtimeFailure) { $null } else { $runtimeFailure.human_summary }
    }
}

function Measure-StabilityProxyCase {
    param([string]$Target, [string]$Case, [string[]]$ProxyArgs, [string]$CaseUrl, $RemotePlan = $null)
    $port = Get-FreePort
    $controlPort = Get-FreePort
    $args = @("--log", (Get-QuicLogFilter), "proxy", $Target, "--listen", "127.0.0.1:$port", "--control-listen", "127.0.0.1:$controlPort", "--deploy", "never", "--connect-timeout-secs", "20", "--transport-pool-size", $script:CurrentTransportPoolSize.ToString()) + (Get-QuicTuningArgs) + $ProxyArgs
    $proc = $null
    $name = "$Target-stability-$Case"
    $planCapture = Get-CachedOrFreshRoutePlanCapture -Target $Target -Case $Case -ProxyArgs $ProxyArgs -Port $port
    if ($RespectPreflightSkip -and -not [string]::IsNullOrWhiteSpace([string](Get-PlanCaptureValue -PlanCapture $planCapture -Name "plan_topology_failure"))) {
        return New-PreflightSkippedResult -Target $Target -Case $Case -PlanCapture $planCapture
    }
    try {
        $proc = Start-LocalProcess -ProcArgs $args -Name $name
        if (-not (Wait-Tcp "127.0.0.1" $port 25)) {
            $detail = Get-ProcessLogSummary -Name $name
            if ([string]::IsNullOrWhiteSpace($detail)) {
                throw "local proxy did not listen on $port"
            }
            throw "local proxy did not listen on $port; $detail"
        }
        Measure-StabilityCase -Target $Target -Case $Case -Port $port -CaseUrl $CaseUrl -ControlPort $controlPort -PlanCapture $planCapture -RemotePlan $RemotePlan
    }
    catch {
        New-ErrorResult -Target $Target -Case $Case -Message $_.Exception.Message -PlanCapture $planCapture
    }
    finally {
        Stop-ProcessQuiet $proc
    }
}

function New-ErrorResult {
    param(
        [string]$Target,
        [string]$Case,
        [string]$Message,
        $PlanCapture = $null,
        [string]$OpenSshControlMasterMode = $null,
        [string]$OpenSshControlPath = $null,
        [int]$OpenSshControlPersistSecs = 0,
        [bool]$OpenSshControlMasterReused = $false
    )
    $topologyFailure = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_topology_failure"
    $errorMessage = if (-not [string]::IsNullOrWhiteSpace([string]$topologyFailure)) {
        "${topologyFailure}; $Message"
    } else {
        $Message
    }
    $failure = Get-BenchmarkFailureClassification -Message $Message -PlanCapture $PlanCapture
    $baselineMode = Get-BaselineMode -Case $Case -OpenSshControlMasterMode $OpenSshControlMasterMode
    $baselineQuality = Get-BaselineQuality `
        -Case $Case `
        -ErrorKind $failure.error_kind `
        -Error $errorMessage `
        -OpenSshControlMasterMode $OpenSshControlMasterMode
    [pscustomobject]@{
        target = $Target
        case = $Case
        run_level = $script:CurrentRunLevel
        selected_protocol = $null
        link_selected_protocol = $null
        link_active_connections = $null
        link_active_streams = $null
        link_active_channels = $null
        link_open_attempts = $null
        link_open_successes = $null
        link_open_failures = $null
        link_open_latency_ms = $null
        link_bytes_client_to_remote = $null
        link_bytes_remote_to_client = $null
        link_first_byte_samples = $null
        link_first_byte_latency_ms = $null
        link_max_first_byte_latency_ms = $null
        link_last_close_reason = $null
        link_degraded_reason = $null
        link_healthy_workers = $null
        link_degraded_workers = $null
        link_reconnecting_workers = $null
        link_control_health = $null
        link_connected = $null
        local_os = $script:LocalPlatform.os
        local_arch = $script:LocalPlatform.arch
        baseline_mode = $baselineMode
        baseline_quality = $baselineQuality.quality
        baseline_quality_reason = $baselineQuality.reason
        baseline_client_os = $script:OpenSshCapability.os
        baseline_client_arch = $script:OpenSshCapability.arch
        openssh_client_version = $script:OpenSshCapability.client_version
        openssh_controlmaster_supported = $script:OpenSshCapability.control_master_supported
        openssh_capability_notes = $script:OpenSshCapability.capability_notes
        remote_os = $script:CurrentRemoteOs
        remote_arch = $script:CurrentRemoteArch
        large_exit = $null
        large_seconds = $null
        large_mibps = $null
        concurrent = $Concurrency
        transport_pool_size = $script:CurrentTransportPoolSize
        concurrent_ok = 0
        concurrent_seconds = $null
        concurrent_mibps = $null
        ssh_direct_channel_open_samples = $null
        last_ssh_direct_channel_open_latency_ms = $null
        spx_peer_handshake_samples = $null
        last_spx_peer_handshake_latency_ms = $null
        quic_profile_enabled = [bool]($QuicProfile -and (Test-QuicBenchmarkCase -Case $Case))
        quic_profile_json = $null
        quic_profile_parameter_set = if (Test-QuicBenchmarkCase -Case $Case) { Get-CurrentQuicParameterSet } else { $null }
        quic_profile_selected_protocol = $null
        quic_profile_pool_size = if (Test-QuicBenchmarkCase -Case $Case) { $script:CurrentTransportPoolSize } else { $null }
        quic_profile_large_mibps = $null
        quic_profile_concurrent_mibps = $null
        quic_profile_failure_kind = if ($null -eq $failure) { $null } else { $failure.error_kind }
        quic_profile_control_health = $null
        quic_profile_next_bottleneck = $null
        quic_profile_window_sizing_suspected = $null
        quic_profile_udp_path_suspected = $null
        quic_profile_application_copy_suspected = $null
        quic_profile_slow_consumers_suspected = $null
        quic_profile_congestion_suspected = $null
        quic_profile_process_total_cpu_delta_ms = $null
        quic_profile_process_user_cpu_delta_ms = $null
        quic_profile_process_privileged_cpu_delta_ms = $null
        quic_profile_process_working_set_bytes = $null
        quic_profile_process_peak_working_set_bytes = $null
        quic_profile_process_private_memory_bytes = $null
        quic_profile_process_working_set_delta_bytes = $null
        quic_profile_process_private_memory_delta_bytes = $null
        quic_profile_packet_loss = $null
        quic_profile_packet_loss_source = if (Test-QuicBenchmarkCase -Case $Case) { "not captured: case failed before QUIC profile status was available" } else { $null }
        quic_profile_max_datagram_size = $null
        quic_profile_max_datagram_size_source = if (Test-QuicBenchmarkCase -Case $Case) { "not captured: case failed before QUIC profile status was available" } else { $null }
        spx_profile_enabled = [bool]$SpxProfile
        spx_write_frames_per_batch = $null
        spx_write_data_bytes_per_frame = $null
        spx_read_data_bytes_per_frame = $null
        spx_frame_flushes_per_batch = $null
        spx_vectored_writes_per_frame = $null
        spx_relay_remote_to_client_mibps = $null
        spx_relay_client_to_remote_mibps = $null
        spx_healthy_workers = $null
        spx_degraded_workers = $null
        spx_reconnecting_workers = $null
        spx_pool_degraded_reason = $null
        direct_transport_policy = Get-CaseTransportPolicy -Case $Case
        direct_transport_policy_reason = Get-CaseTransportPolicyReason -Case $Case
        tls_peer_auth_mode = $null
        openssh_control_master_mode = $OpenSshControlMasterMode
        openssh_control_path = $OpenSshControlPath
        openssh_control_persist_secs = if ($OpenSshControlPersistSecs -gt 0) { $OpenSshControlPersistSecs } else { $null }
        openssh_control_master_reused = $OpenSshControlMasterReused
        plan_selected_transport = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_selected_transport"
        plan_recommended_fallback = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_recommended_fallback"
        plan_fallback_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_fallback_reason"
        plan_next_action = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_next_action"
        plan_ssh_data_plane_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_ssh_data_plane_reason"
        plan_decision_chain = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_chain"
        plan_decision_topology_class = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_topology_class"
        plan_decision_selected_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_selected_reason"
        plan_decision_repair_hint = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_repair_hint"
        plan_decision_explicit_user_override = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_explicit_user_override"
        plan_candidate_failures = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_candidate_failures"
        plan_candidate_failure_endpoints = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_candidate_failure_endpoints"
        plan_topology_failure = $topologyFailure
        plan_error = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_error"
        active_quic_flows = $null
        quic_stream_open_samples = $null
        last_quic_stream_open_latency_ms = $null
        max_quic_stream_open_latency_ms = $null
        quic_stream_open_failures = $null
        quic_stream_open_timeout_secs = $null
        quic_first_byte_timeout_secs = $null
        quic_header_write_samples = $null
        last_quic_header_write_latency_ms = $null
        max_quic_header_write_latency_ms = $null
        quic_header_write_failures = $null
        quic_copy_buffer_size = $null
        quic_backpressure_timeout_secs = $null
        quic_backpressure_timeouts = $null
        quic_flow_graceful_closes = $null
        quic_flow_resets = $null
        quic_flow_first_byte_samples = $null
        last_quic_flow_first_byte_latency_ms = $null
        max_quic_flow_first_byte_latency_ms = $null
        quic_copy_duration_samples = $null
        last_quic_copy_duration_ms = $null
        max_quic_copy_duration_ms = $null
        quic_copy_failures = $null
        last_quic_copy_client_to_remote_bytes = $null
        last_quic_copy_remote_to_client_bytes = $null
        max_quic_copy_client_to_remote_bytes = $null
        max_quic_copy_remote_to_client_bytes = $null
        last_quic_flow_close_reason = $null
        quic_max_bidi_streams = $null
        quic_stream_receive_window = $null
        quic_receive_window = $null
        quic_keep_alive_interval_secs = $null
        quic_idle_timeout_secs = $null
        quic_connection_pool_size = $null
        quic_connection_pool_policy = $null
        quic_connection_pool_workload_hint = $null
        quic_connection_pool_reason = $null
        quic_connection_pool_mode = $null
        quic_connection_pool_selection_policy = $null
        quic_udp_runtime = $null
        quic_udp_gso = $null
        quic_udp_gso_source = $null
        quic_packetization = $null
        quic_connections = $null
        quic_connection_active_flows = $null
        quic_connection_opened_flows = $null
        quic_connection_resets = $null
        quic_connection_control_states = $null
        quic_connection_control_degraded = $null
        control_state = $null
        control_degraded = $null
        control_pings_sent = $null
        control_pongs_received = $null
        last_control_pong_latency_ms = $null
        last_control_error = $null
        ssh_mode = $null
        ssh_session_pool_size = $null
        ssh_session_pool_source = $null
        ssh_session_pool_reason = $null
        ssh_session_pool_warning = $null
        ssh_session_growth_active_threshold = $null
        ssh_session_growth_events = $null
        ssh_session_growth_suppressed = $null
        active_ssh_sessions = $null
        active_ssh_channels = $null
        ssh_session_connect_attempts = $null
        ssh_session_connect_failures = $null
        ssh_channel_open_attempts = $null
        ssh_channel_open_failures = $null
        ssh_first_byte_samples = $null
        ssh_avg_first_byte_latency_ms = $null
        last_ssh_first_byte_latency_ms = $null
        max_ssh_first_byte_latency_ms = $null
        p50_ssh_first_byte_latency_ms = $null
        p95_ssh_first_byte_latency_ms = $null
        ssh_last_channel_queue_depth = $null
        ssh_max_channel_queue_depth = $null
        ssh_graceful_closes = $null
        ssh_error_closes = $null
        last_ssh_close_reason = $null
        status_before = $null
        status_after = $null
        plan_before = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_before"
        skipped_by_preflight = $false
        skip_reason = $null
        skip_endpoint = $null
        ssh_vs_spx_overhead = $null
        primary_failure_stage = $failure.primary_failure_stage
        human_summary = $failure.human_summary
        error_kind = $failure.error_kind
        error = $errorMessage
    }
}

function New-PreflightSkippedResult {
    param([string]$Target, [string]$Case, $PlanCapture = $null)
    $topologyFailure = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_topology_failure"
    $endpoint = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_candidate_failure_endpoints"
    $reason = if (-not [string]::IsNullOrWhiteSpace([string]$topologyFailure)) {
        $topologyFailure
    } else {
        "preflight_skip"
    }
    $failure = Get-BenchmarkFailureClassification `
        -Message "skipped by route preflight: $reason" `
        -PlanCapture $PlanCapture `
        -ForcedKind "preflight_skip" `
        -ForcedStage "preflight"
    $baselineMode = Get-BaselineMode -Case $Case
    $baselineQuality = Get-BaselineQuality `
        -Case $Case `
        -ErrorKind $failure.error_kind `
        -Error "skipped by route preflight: $reason"
    [pscustomobject]@{
        target = $Target
        case = $Case
        run_level = $script:CurrentRunLevel
        selected_protocol = $null
        link_selected_protocol = $null
        link_active_connections = $null
        link_active_streams = $null
        link_active_channels = $null
        link_open_attempts = $null
        link_open_successes = $null
        link_open_failures = $null
        link_open_latency_ms = $null
        link_bytes_client_to_remote = $null
        link_bytes_remote_to_client = $null
        link_first_byte_samples = $null
        link_first_byte_latency_ms = $null
        link_max_first_byte_latency_ms = $null
        link_last_close_reason = $null
        link_degraded_reason = $null
        link_control_health = $null
        link_connected = $null
        local_os = $script:LocalPlatform.os
        local_arch = $script:LocalPlatform.arch
        baseline_mode = $baselineMode
        baseline_quality = $baselineQuality.quality
        baseline_quality_reason = $baselineQuality.reason
        baseline_client_os = $script:OpenSshCapability.os
        baseline_client_arch = $script:OpenSshCapability.arch
        openssh_client_version = $script:OpenSshCapability.client_version
        openssh_controlmaster_supported = $script:OpenSshCapability.control_master_supported
        openssh_capability_notes = $script:OpenSshCapability.capability_notes
        remote_os = $script:CurrentRemoteOs
        remote_arch = $script:CurrentRemoteArch
        large_exit = $null
        large_seconds = $null
        large_mibps = $null
        concurrent = $Concurrency
        transport_pool_size = $script:CurrentTransportPoolSize
        concurrent_ok = 0
        concurrent_seconds = $null
        concurrent_mibps = $null
        plan_selected_transport = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_selected_transport"
        plan_recommended_fallback = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_recommended_fallback"
        plan_fallback_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_fallback_reason"
        plan_next_action = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_next_action"
        plan_ssh_data_plane_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_ssh_data_plane_reason"
        plan_decision_chain = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_chain"
        plan_decision_topology_class = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_topology_class"
        plan_decision_selected_reason = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_selected_reason"
        plan_decision_repair_hint = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_repair_hint"
        plan_decision_explicit_user_override = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_decision_explicit_user_override"
        plan_candidate_failures = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_candidate_failures"
        plan_candidate_failure_endpoints = $endpoint
        plan_topology_failure = $topologyFailure
        plan_error = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_error"
        status_before = $null
        status_after = $null
        plan_before = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "plan_before"
        skipped_by_preflight = $true
        skip_reason = $reason
        skip_endpoint = $endpoint
        preflight_cache_hit = [bool](Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "preflight_cache_hit")
        preflight_cache_key = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "preflight_cache_key"
        preflight_cache_protocol = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "preflight_cache_protocol"
        preflight_cache_endpoint = Get-PlanCaptureValue -PlanCapture $PlanCapture -Name "preflight_cache_endpoint"
        quic_profile_enabled = [bool]($QuicProfile -and (Test-QuicBenchmarkCase -Case $Case))
        quic_profile_json = $null
        quic_profile_parameter_set = if (Test-QuicBenchmarkCase -Case $Case) { Get-CurrentQuicParameterSet } else { $null }
        quic_profile_selected_protocol = $null
        quic_profile_pool_size = if (Test-QuicBenchmarkCase -Case $Case) { $script:CurrentTransportPoolSize } else { $null }
        quic_profile_large_mibps = $null
        quic_profile_concurrent_mibps = $null
        quic_profile_failure_kind = if ($null -eq $failure) { $null } else { $failure.error_kind }
        quic_profile_control_health = $null
        quic_profile_next_bottleneck = $null
        quic_profile_window_sizing_suspected = $null
        quic_profile_udp_path_suspected = $null
        quic_profile_application_copy_suspected = $null
        quic_profile_slow_consumers_suspected = $null
        quic_profile_congestion_suspected = $null
        quic_profile_process_total_cpu_delta_ms = $null
        quic_profile_process_user_cpu_delta_ms = $null
        quic_profile_process_privileged_cpu_delta_ms = $null
        quic_profile_process_working_set_bytes = $null
        quic_profile_process_peak_working_set_bytes = $null
        quic_profile_process_private_memory_bytes = $null
        quic_profile_process_working_set_delta_bytes = $null
        quic_profile_process_private_memory_delta_bytes = $null
        quic_profile_packet_loss = $null
        quic_profile_packet_loss_source = if (Test-QuicBenchmarkCase -Case $Case) { "not captured: skipped by route preflight" } else { $null }
        quic_profile_max_datagram_size = $null
        quic_profile_max_datagram_size_source = if (Test-QuicBenchmarkCase -Case $Case) { "not captured: skipped by route preflight" } else { $null }
        spx_profile_enabled = [bool]$SpxProfile
        spx_write_frames_per_batch = $null
        spx_write_data_bytes_per_frame = $null
        spx_read_data_bytes_per_frame = $null
        spx_frame_flushes_per_batch = $null
        spx_vectored_writes_per_frame = $null
        spx_relay_remote_to_client_mibps = $null
        spx_relay_client_to_remote_mibps = $null
        direct_transport_policy = Get-CaseTransportPolicy -Case $Case
        direct_transport_policy_reason = Get-CaseTransportPolicyReason -Case $Case
        tls_peer_auth_mode = $null
        openssh_control_master_mode = $null
        openssh_control_path = $null
        openssh_control_persist_secs = $null
        openssh_control_master_reused = $false
        ssh_vs_spx_overhead = $null
        primary_failure_stage = $failure.primary_failure_stage
        human_summary = $failure.human_summary
        error_kind = $failure.error_kind
        error = "skipped by route preflight: $reason"
    }
}

function Measure-ProxyCase {
    param([string]$Target, [string]$Case, [string[]]$ProxyArgs, [string]$CaseUrl)
    $port = Get-FreePort
    $controlPort = Get-FreePort
    $args = @("--log", (Get-QuicLogFilter), "proxy", $Target, "--listen", "127.0.0.1:$port", "--control-listen", "127.0.0.1:$controlPort", "--deploy", "never", "--no-reconnect", "--connect-timeout-secs", "20", "--transport-pool-size", $script:CurrentTransportPoolSize.ToString()) + (Get-QuicTuningArgs) + $ProxyArgs
    $proc = $null
    $name = "$Target-$Case"
    $planCapture = Get-CachedOrFreshRoutePlanCapture -Target $Target -Case $Case -ProxyArgs $ProxyArgs -Port $port
    if ($RespectPreflightSkip -and -not [string]::IsNullOrWhiteSpace([string](Get-PlanCaptureValue -PlanCapture $planCapture -Name "plan_topology_failure"))) {
        return New-PreflightSkippedResult -Target $Target -Case $Case -PlanCapture $planCapture
    }
    try {
        $proc = Start-LocalProcess -ProcArgs $args -Name $name
        if (-not (Wait-Tcp "127.0.0.1" $port 25)) {
            $detail = Get-ProcessLogSummary -Name $name
            if ([string]::IsNullOrWhiteSpace($detail)) {
                throw "local proxy did not listen on $port"
            }
            throw "local proxy did not listen on $port; $detail"
        }
        Measure-Case -Target $Target -Case $Case -Port $port -CaseUrl $CaseUrl -ControlPort $controlPort -PlanCapture $planCapture -ProfileProcess $proc
    }
    catch {
        New-ErrorResult -Target $Target -Case $Case -Message $_.Exception.Message -PlanCapture $planCapture
    }
    finally {
        Stop-ProcessQuiet $proc
    }
}

function Measure-SshdBaseline {
    param([string]$Target, [string]$CaseUrl)
    $port = Get-FreePort
    $proc = $null
    $name = "$Target-sshd-D"
    try {
        $proc = Start-SshSocks -Target $Target -Port $port -Name $name
        if (-not (Wait-Tcp "127.0.0.1" $port 25)) {
            $detail = Get-ProcessLogSummary -Name $name
            if ([string]::IsNullOrWhiteSpace($detail)) {
                throw "ssh -D did not listen on $port"
            }
            throw "ssh -D did not listen on $port; $detail"
        }
        Measure-Case -Target $Target -Case "sshd-D" -Port $port -CaseUrl $CaseUrl -OpenSshControlMasterMode "disabled"
    }
    catch {
        New-ErrorResult -Target $Target -Case "sshd-D" -Message $_.Exception.Message -OpenSshControlMasterMode "disabled"
    }
    finally {
        Stop-ProcessQuiet $proc
    }
}

function Measure-SshdControlMasterBaseline {
    param([string]$Target, [string]$CaseUrl, [string]$Mode)
    $case = if ($Mode -eq "fresh") { "sshd-D-controlmaster-fresh" } else { "sshd-D-controlmaster" }
    $controlMode = if ($Mode -eq "fresh") { "fresh_master" } else { "reused_master" }
    $port = Get-FreePort
    $controlPath = Get-SshControlPath -Target $Target -Mode $Mode
    $masterProc = $null
    $socksProc = $null
    $masterName = "$Target-$case-master"
    $socksName = "$Target-$case"
    try {
        $masterProc = Start-SshControlMaster -Target $Target -ControlPath $controlPath -Name $masterName
        $socksProc = Start-SshSocks -Target $Target -Port $port -Name $socksName -ControlPath $controlPath
        if (-not (Wait-Tcp "127.0.0.1" $port 25)) {
            $detail = Get-ProcessLogSummary -Name $socksName
            if ([string]::IsNullOrWhiteSpace($detail)) {
                throw "ssh -D ControlMaster baseline did not listen on $port"
            }
            throw "ssh -D ControlMaster baseline did not listen on $port; $detail"
        }
        Measure-Case `
            -Target $Target `
            -Case $case `
            -Port $port `
            -CaseUrl $CaseUrl `
            -OpenSshControlMasterMode $controlMode `
            -OpenSshControlPath $controlPath `
            -OpenSshControlPersistSecs $SshControlPersistSecs `
            -OpenSshControlMasterReused ($Mode -ne "fresh")
    }
    catch {
        New-ErrorResult `
            -Target $Target `
            -Case $case `
            -Message $_.Exception.Message `
            -OpenSshControlMasterMode $controlMode `
            -OpenSshControlPath $controlPath `
            -OpenSshControlPersistSecs $SshControlPersistSecs `
            -OpenSshControlMasterReused ($Mode -ne "fresh")
    }
    finally {
        Stop-ProcessQuiet $socksProc
        Stop-SshControlMaster -Target $Target -ControlPath $controlPath -Process $masterProc
    }
}

function Convert-BenchNumber {
    param($Value)
    if ($null -eq $Value -or $Value -eq "") {
        return 0.0
    }
    return [double]$Value
}

function Divide-BenchNumber {
    param($Numerator, $Denominator)
    $num = Convert-BenchNumber $Numerator
    $den = Convert-BenchNumber $Denominator
    if ($den -le 0) {
        return $null
    }
    return [Math]::Round($num / $den, 3)
}

function Divide-MibPerSecond {
    param($Bytes, $DurationMs)
    $bytesValue = Convert-BenchNumber $Bytes
    $durationValue = Convert-BenchNumber $DurationMs
    if ($bytesValue -le 0 -or $durationValue -le 0) {
        return $null
    }
    return [Math]::Round(($bytesValue / 1048576.0) / ($durationValue / 1000.0), 3)
}

function Convert-BenchInt {
    param($Value, [int]$Default = 0)
    if ($null -eq $Value -or [string]::IsNullOrWhiteSpace([string]$Value)) {
        return $Default
    }
    $parsed = 0
    if ([int]::TryParse([string]$Value, [ref]$parsed)) {
        return $parsed
    }
    return $Default
}

function Get-BenchPoolSize {
    param($Value)
    $poolSize = Convert-BenchInt $Value 0
    if ($poolSize -gt 0) {
        return $poolSize
    }
    return $null
}

function Get-CaseDirectProtocol {
    param([string]$Case)
    switch ($Case) {
        "spx-plain-direct" { return "plain-tcp" }
        "spx-tls-direct" { return "tls-tcp" }
        "spx-quic-direct" { return "quic-framed" }
        "quic-native-direct" { return "quic-native" }
        default { return $null }
    }
}

function Get-CaseTransportPolicy {
    param([string]$Case)
    switch ($Case) {
        "spx-plain-direct" { return "lab_baseline" }
        "spx-tls-direct" { return "production_direct" }
        "spx-quic-direct" { return "experimental" }
        "quic-native-direct" { return "experimental" }
        default { return $null }
    }
}

function Get-CaseTransportPolicyReason {
    param([string]$Case)
    switch ($Case) {
        "spx-plain-direct" { return "Plain TCP SPX is a lab or explicitly trusted baseline only; it is not the production default because the data path is not encrypted." }
        "spx-tls-direct" { return "TLS/TCP SPX is the production direct baseline because it keeps the stable SPX data plane while adding peer encryption and certificate identity." }
        "spx-quic-direct" { return "Framed QUIC remains experimental until throughput and recovery behavior close the gap with TLS/TCP SPX." }
        "quic-native-direct" { return "QUIC-native remains experimental until throughput and recovery behavior close the gap with TLS/TCP SPX." }
        default { return $null }
    }
}

function Get-PlanTopology {
    param($Row)
    if ($null -eq $Row.plan_before -or [string]::IsNullOrWhiteSpace([string]$Row.plan_before)) {
        return $null
    }
    try {
        $plan = [string]$Row.plan_before | ConvertFrom-Json
        if ($null -ne $plan.topology) {
            return $plan.topology
        }
    }
    catch {
    }
    return $null
}

function Get-StatusProtocol {
    param($Row)
    if ($null -eq $Row.status_after -or [string]::IsNullOrWhiteSpace([string]$Row.status_after)) {
        return $null
    }
    try {
        $status = [string]$Row.status_after | ConvertFrom-Json
        if ($null -ne $status.link -and $null -ne $status.link.health -and $null -ne $status.link.health.selected_protocol) {
            return [string]$status.link.health.selected_protocol
        }
        if ($null -ne $status.selected_protocol) {
            return [string]$status.selected_protocol
        }
        if ($null -ne $status.workers) {
            $protocols = @($status.workers | Where-Object { $_.selected_protocol } | ForEach-Object { [string]$_.selected_protocol } | Select-Object -Unique)
            if ($protocols.Count -eq 1) {
                return $protocols[0]
            }
            if ($protocols.Count -gt 1) {
                return "mixed"
            }
        }
    }
    catch {
    }
    return $null
}

function Get-TargetTopologies {
    param([object[]]$Rows)
    $topologies = New-Object System.Collections.Generic.List[object]
    foreach ($group in ($Rows | Group-Object target)) {
        $target = [string]$group.Name
        $targetRows = @($group.Group)
        $reachableDirect = @($targetRows | Where-Object {
            $null -ne (Get-CaseDirectProtocol $_.case) -and
            [string]::IsNullOrWhiteSpace([string]$_.error) -and
            [string]::IsNullOrWhiteSpace([string]$_.error_kind)
        } | ForEach-Object { Get-CaseDirectProtocol $_.case } | Where-Object { $_ } | Sort-Object -Unique)
        $failedDirect = @($targetRows | Where-Object {
            $null -ne (Get-CaseDirectProtocol $_.case) -and
            (-not [string]::IsNullOrWhiteSpace([string]$_.error_kind) -or -not [string]::IsNullOrWhiteSpace([string]$_.plan_topology_failure))
        } | ForEach-Object { Get-CaseDirectProtocol $_.case } | Where-Object { $_ } | Sort-Object -Unique)
        $sshProtocols = @($targetRows | Where-Object {
            [string]::IsNullOrWhiteSpace([string]$_.error) -and
            ($_.case -like "sshd-D*" -or $_.case -eq "spx-ssh-direct" -or $_.case -eq "ssh-native-direct")
        } | ForEach-Object {
            if ($_.case -like "sshd-D*") { "openssh-socks" } else { [string]$_.selected_protocol }
        } | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } | Sort-Object -Unique)

        $sshJumpChain = @()
        $sshTarget = $null
        $directCandidates = @()
        foreach ($row in $targetRows) {
            $topology = Get-PlanTopology -Row $row
            if ($null -eq $topology) {
                continue
            }
            if ($sshJumpChain.Count -eq 0 -and $null -ne $topology.ssh_jump_chain) {
                $sshJumpChain = @($topology.ssh_jump_chain | ForEach-Object { [string]$_ } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            }
            if ([string]::IsNullOrWhiteSpace([string]$sshTarget) -and $null -ne $topology.ssh_target) {
                $sshTarget = [string]$topology.ssh_target
            }
            if ($directCandidates.Count -eq 0 -and $null -ne $topology.direct_private_candidates) {
                $directCandidates = @($topology.direct_private_candidates | ForEach-Object {
                    if ($_.protocol -and $_.endpoint) {
                        "$($_.protocol):$($_.endpoint)"
                    } elseif ($_.endpoint) {
                        [string]$_.endpoint
                    }
                } | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) })
            }
        }

        $topologyClass = if ($reachableDirect.Count -gt 0) {
            "direct-reachable"
        } elseif ($failedDirect.Count -gt 0 -and $sshProtocols.Count -gt 0) {
            "ssh-only"
        } elseif ($sshProtocols.Count -gt 0) {
            "ssh-reachable"
        } else {
            "unknown"
        }

        $recommended = "manual-review"
        $reason = "insufficient benchmark evidence"
        if ($topologyClass -eq "ssh-only") {
            $recommended = "ssh-fallback"
            $reason = "direct peer endpoints failed preflight/runtime checks while SSH paths succeeded"
        } elseif ($reachableDirect -contains "tls-tcp") {
            $recommended = "tls-tcp"
            $reason = "direct TLS/TCP peer transport is reachable and is the production direct default"
        } elseif ($reachableDirect -contains "plain-tcp") {
            $recommended = "plain-tcp-lab"
            $reason = "only trusted/lab plain TCP direct transport was observed as reachable"
        } elseif ($reachableDirect -contains "quic-native") {
            $recommended = "quic-native-experimental"
            $reason = "QUIC-native is reachable but remains an experimental data plane"
        } elseif ($reachableDirect -contains "quic-framed") {
            $recommended = "quic-framed-experimental"
            $reason = "framed QUIC is reachable but remains behind TCP/TLS baselines"
        }

        $topologies.Add([pscustomobject]@{
            target = $target
            topology_class = $topologyClass
            ssh_target = $sshTarget
            ssh_jump_chain = $sshJumpChain
            direct_private_candidates = $directCandidates
            reachable_direct_protocols = $reachableDirect
            failed_direct_protocols = $failedDirect
            observed_ssh_protocols = $sshProtocols
            recommended_default_transport = $recommended
            recommendation_reason = $reason
        })
    }
    return $topologies
}

function Get-PoolRecommendations {
    param([object[]]$Rows, [int]$ExpectedConcurrency)
    $recommendations = New-Object System.Collections.Generic.List[object]
    $pooledRows = @($Rows | Where-Object {
        ($_.case -like "spx-*" -or $_.case -eq "ssh-native-direct") -and
        $null -ne (Get-BenchPoolSize $_.transport_pool_size)
    })
    if ($pooledRows.Count -eq 0) {
        return $recommendations
    }
    foreach ($group in ($pooledRows | Group-Object target, case)) {
        $parts = $group.Name -split ', '
        $target = $parts[0]
        $case = $parts[1]
        $rows = @($group.Group)
        $testedPools = @($rows | ForEach-Object { Get-BenchPoolSize $_.transport_pool_size } | Where-Object { $null -ne $_ } | Sort-Object -Unique)
        $successful = @($rows | Where-Object {
            [string]::IsNullOrWhiteSpace([string]$_.error) -and
            (Convert-BenchInt $_.large_exit -1) -eq 0 -and
            (Convert-BenchInt $_.concurrent_ok 0) -gt 0
        })
        if ($successful.Count -eq 0) {
            $recommendations.Add([pscustomobject]@{
                target = $target
                case = $case
                transport_policy = Get-CaseTransportPolicy -Case $case
                recommended_pool_size = $null
                recommendation_basis = "no successful pool run"
                pools_tested = $testedPools
                successful_pools = @()
                selected_protocols = @()
                best_large_pool_size = $null
                best_concurrent_pool_size = $null
            })
            continue
        }

        $successfulPools = @($successful | ForEach-Object { Get-BenchPoolSize $_.transport_pool_size } | Where-Object { $null -ne $_ } | Sort-Object -Unique)
        $stable = @($successful | Where-Object { (Convert-BenchInt $_.concurrent_ok 0) -ge $ExpectedConcurrency })
        $largeBest = @($successful | Sort-Object `
            @{ Expression = { Convert-BenchNumber $_.large_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchNumber $_.concurrent_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchInt $_.transport_pool_size 0 }; Descending = $false } |
            Select-Object -First 1)[0]
        $concurrentCandidates = if ($stable.Count -gt 0) { $stable } else { $successful }
        $concurrentBest = @($concurrentCandidates | Sort-Object `
            @{ Expression = { Convert-BenchInt $_.concurrent_ok 0 }; Descending = $true },
            @{ Expression = { Convert-BenchNumber $_.concurrent_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchNumber $_.large_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchInt $_.transport_pool_size 0 }; Descending = $false } |
            Select-Object -First 1)[0]

        $recommended = if ($ExpectedConcurrency -gt 1) { $concurrentBest } else { $largeBest }
        $basis = if ($ExpectedConcurrency -gt 1) {
            if ($stable.Count -gt 0) {
                "highest concurrent throughput among pools with all concurrent requests successful"
            } else {
                "highest concurrent success count/throughput; no pool completed all concurrent requests"
            }
        } else {
            "highest large-transfer throughput"
        }
        $protocols = @($successful | ForEach-Object { Get-StatusProtocol $_ } | Where-Object { $_ } | Select-Object -Unique)
        $recommendations.Add([pscustomobject]@{
            target = $target
            case = $case
            transport_policy = Get-CaseTransportPolicy -Case $case
            recommended_pool_size = Get-BenchPoolSize $recommended.transport_pool_size
            recommendation_basis = $basis
            pools_tested = $testedPools
            successful_pools = $successfulPools
            selected_protocols = $protocols
            best_large_pool_size = Get-BenchPoolSize $largeBest.transport_pool_size
            best_large_mibps = Convert-BenchNumber $largeBest.large_mibps
            best_concurrent_pool_size = Get-BenchPoolSize $concurrentBest.transport_pool_size
            best_concurrent_ok = Convert-BenchInt $concurrentBest.concurrent_ok 0
            best_concurrent_mibps = Convert-BenchNumber $concurrentBest.concurrent_mibps
        })
    }
    return $recommendations
}

function Get-PoolRecommendationsByWorkload {
    param([object[]]$Rows, [int]$ExpectedConcurrency)
    $recommendations = New-Object System.Collections.Generic.List[object]
    $workloads = @("large_transfer", "concurrent_small_flows", "mixed")
    $pooledRows = @($Rows | Where-Object {
        ($_.case -like "spx-*" -or $_.case -eq "ssh-native-direct") -and
        $null -ne (Get-BenchPoolSize $_.transport_pool_size)
    })
    if ($pooledRows.Count -eq 0) {
        return $recommendations
    }
    foreach ($group in ($pooledRows | Group-Object target, case)) {
        $parts = $group.Name -split ', '
        $target = $parts[0]
        $case = $parts[1]
        $rows = @($group.Group)
        $testedPools = @($rows | ForEach-Object { Get-BenchPoolSize $_.transport_pool_size } | Where-Object { $null -ne $_ } | Sort-Object -Unique)
        $successful = @($rows | Where-Object {
            [string]::IsNullOrWhiteSpace([string]$_.error) -and
            (Convert-BenchInt $_.large_exit -1) -eq 0 -and
            (Convert-BenchInt $_.concurrent_ok 0) -gt 0
        })
        if ($successful.Count -eq 0) {
            foreach ($workload in $workloads) {
                $recommendations.Add([pscustomobject]@{
                    target = $target
                    case = $case
                    workload_shape = $workload
                    transport_policy = Get-CaseTransportPolicy -Case $case
                    recommended_pool_size = $null
                    recommendation_basis = "no successful pool run"
                    pools_tested = $testedPools
                    successful_pools = @()
                    selected_protocols = @()
                    best_pool_size = $null
                    best_large_mibps = $null
                    best_concurrent_ok = $null
                    best_concurrent_mibps = $null
                })
            }
            continue
        }

        $successfulPools = @($successful | ForEach-Object { Get-BenchPoolSize $_.transport_pool_size } | Where-Object { $null -ne $_ } | Sort-Object -Unique)
        $stable = @($successful | Where-Object { (Convert-BenchInt $_.concurrent_ok 0) -ge $ExpectedConcurrency })
        $protocols = @($successful | ForEach-Object { Get-StatusProtocol $_ } | Where-Object { $_ } | Select-Object -Unique)

        $largeBest = @($successful | Sort-Object `
            @{ Expression = { Convert-BenchNumber $_.large_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchNumber $_.concurrent_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchInt $_.transport_pool_size 0 }; Descending = $false } |
            Select-Object -First 1)[0]

        $concurrentCandidates = if ($stable.Count -gt 0) { $stable } else { $successful }
        $concurrentBest = @($concurrentCandidates | Sort-Object `
            @{ Expression = { Convert-BenchInt $_.concurrent_ok 0 }; Descending = $true },
            @{ Expression = { Convert-BenchNumber $_.concurrent_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchNumber $_.large_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchInt $_.transport_pool_size 0 }; Descending = $false } |
            Select-Object -First 1)[0]

        $mixedCandidates = if ($stable.Count -gt 0) { $stable } else { $successful }
        $mixedBest = @($mixedCandidates | Sort-Object `
            @{ Expression = { [Math]::Min((Convert-BenchNumber $_.large_mibps), (Convert-BenchNumber $_.concurrent_mibps)) }; Descending = $true },
            @{ Expression = { (Convert-BenchNumber $_.large_mibps) + (Convert-BenchNumber $_.concurrent_mibps) }; Descending = $true },
            @{ Expression = { Convert-BenchInt $_.concurrent_ok 0 }; Descending = $true },
            @{ Expression = { Convert-BenchInt $_.transport_pool_size 0 }; Descending = $false } |
            Select-Object -First 1)[0]

        $workloadRows = @(
            [pscustomobject]@{
                workload_shape = "large_transfer"
                row = $largeBest
                basis = "highest large-transfer throughput"
            },
            [pscustomobject]@{
                workload_shape = "concurrent_small_flows"
                row = $concurrentBest
                basis = if ($stable.Count -gt 0) {
                    "highest concurrent throughput among pools with all concurrent requests successful"
                } else {
                    "highest concurrent success count/throughput; no pool completed all concurrent requests"
                }
            },
            [pscustomobject]@{
                workload_shape = "mixed"
                row = $mixedBest
                basis = if ($stable.Count -gt 0) {
                    "best balanced large/concurrent throughput among pools with all concurrent requests successful"
                } else {
                    "best balanced large/concurrent throughput; no pool completed all concurrent requests"
                }
            }
        )

        foreach ($workloadRow in $workloadRows) {
            $row = $workloadRow.row
            $recommendations.Add([pscustomobject]@{
                target = $target
                case = $case
                workload_shape = $workloadRow.workload_shape
                transport_policy = Get-CaseTransportPolicy -Case $case
                recommended_pool_size = Get-BenchPoolSize $row.transport_pool_size
                recommendation_basis = $workloadRow.basis
                pools_tested = $testedPools
                successful_pools = $successfulPools
                selected_protocols = $protocols
                best_pool_size = Get-BenchPoolSize $row.transport_pool_size
                best_large_mibps = Convert-BenchNumber $row.large_mibps
                best_concurrent_ok = Convert-BenchInt $row.concurrent_ok 0
                best_concurrent_mibps = Convert-BenchNumber $row.concurrent_mibps
            })
        }
    }
    return $recommendations
}

function Get-SshNativePoolDiagnostics {
    param([object[]]$Rows)
    $diagnostics = New-Object System.Collections.Generic.List[object]
    $sshRows = @($Rows | Where-Object {
        $_.case -eq "ssh-native-direct" -and $null -ne (Get-BenchPoolSize $_.transport_pool_size)
    })
    foreach ($group in ($sshRows | Group-Object target)) {
        $target = [string]$group.Name
        $rows = @($group.Group)
        $successful = @($rows | Where-Object {
            [string]::IsNullOrWhiteSpace([string]$_.error) -and
            (Convert-BenchInt $_.large_exit -1) -eq 0 -and
            (Convert-BenchInt $_.concurrent_ok 0) -gt 0
        })
        $testedPools = @($rows | ForEach-Object { Get-BenchPoolSize $_.transport_pool_size } | Where-Object { $null -ne $_ } | Sort-Object -Unique)
        $largeBest = if ($successful.Count -gt 0) {
            @($successful | Sort-Object `
                @{ Expression = { Convert-BenchNumber $_.large_mibps }; Descending = $true },
                @{ Expression = { Convert-BenchNumber $_.concurrent_mibps }; Descending = $true },
                @{ Expression = { Convert-BenchInt $_.transport_pool_size 0 }; Descending = $false } |
                Select-Object -First 1)[0]
        } else {
            $null
        }
        $concurrentBest = if ($successful.Count -gt 0) {
            @($successful | Sort-Object `
                @{ Expression = { Convert-BenchInt $_.concurrent_ok 0 }; Descending = $true },
                @{ Expression = { Convert-BenchNumber $_.concurrent_mibps }; Descending = $true },
                @{ Expression = { Convert-BenchNumber $_.large_mibps }; Descending = $true },
                @{ Expression = { Convert-BenchInt $_.transport_pool_size 0 }; Descending = $false } |
                Select-Object -First 1)[0]
        } else {
            $null
        }
        $highPools = @($testedPools | Where-Object { $_ -gt 2 })
        $diagnostics.Add([pscustomobject]@{
            target = $target
            implicit_default_max_pool = 2
            tested_pools = $testedPools
            explicit_high_pools = $highPools
            best_large_pool_size = if ($null -ne $largeBest) { Get-BenchPoolSize $largeBest.transport_pool_size } else { $null }
            best_large_mibps = if ($null -ne $largeBest) { Convert-BenchNumber $largeBest.large_mibps } else { $null }
            best_concurrent_pool_size = if ($null -ne $concurrentBest) { Get-BenchPoolSize $concurrentBest.transport_pool_size } else { $null }
            best_concurrent_mibps = if ($null -ne $concurrentBest) { Convert-BenchNumber $concurrentBest.concurrent_mibps } else { $null }
            best_concurrent_ok = if ($null -ne $concurrentBest) { Convert-BenchInt $concurrentBest.concurrent_ok 0 } else { $null }
            high_pool_policy = if ($highPools.Count -gt 0) {
                "pool sizes above 2 are benchmark-only explicit experiments; implicit ssh-native defaults stay at 1/2"
            } else {
                "only implicit-safe ssh-native pools were tested"
            }
        })
    }
    return $diagnostics
}

function Set-SshVsSpxOverheadComparisons {
    param([object[]]$Rows)
    $comparisons = New-Object System.Collections.Generic.List[object]
    foreach ($group in ($Rows | Group-Object target)) {
        $target = [string]$group.Name
        $targetRows = @($group.Group)
        $poolSizes = @($targetRows | Where-Object {
            $_.case -in @("spx-ssh-direct", "ssh-native-direct") -and
            $null -ne (Get-BenchPoolSize $_.transport_pool_size)
        } | ForEach-Object { Get-BenchPoolSize $_.transport_pool_size } | Sort-Object -Unique)
        foreach ($pool in $poolSizes) {
            $spx = @($targetRows | Where-Object {
                $_.case -eq "spx-ssh-direct" -and (Get-BenchPoolSize $_.transport_pool_size) -eq $pool
            } | Select-Object -First 1)[0]
            $native = @($targetRows | Where-Object {
                $_.case -eq "ssh-native-direct" -and (Get-BenchPoolSize $_.transport_pool_size) -eq $pool
            } | Select-Object -First 1)[0]
            if ($null -eq $spx -or $null -eq $native) {
                continue
            }
            $spxLarge = Convert-BenchNumber $spx.large_mibps
            $nativeLarge = Convert-BenchNumber $native.large_mibps
            $spxConcurrent = Convert-BenchNumber $spx.concurrent_mibps
            $nativeConcurrent = Convert-BenchNumber $native.concurrent_mibps
            $comparison = [pscustomobject]@{
                target = $target
                pool = $pool
                spx_protocol = $spx.selected_protocol
                native_protocol = $native.selected_protocol
                spx_large_mibps = $spxLarge
                native_large_mibps = $nativeLarge
                spx_concurrent_mibps = $spxConcurrent
                native_concurrent_mibps = $nativeConcurrent
                spx_large_speedup_over_native = if ($nativeLarge -gt 0) { [Math]::Round($spxLarge / $nativeLarge, 3) } else { $null }
                spx_concurrent_speedup_over_native = if ($nativeConcurrent -gt 0) { [Math]::Round($spxConcurrent / $nativeConcurrent, 3) } else { $null }
                ssh_direct_channel_open_latency_ms = $spx.last_ssh_direct_channel_open_latency_ms
                spx_peer_handshake_latency_ms = $spx.last_spx_peer_handshake_latency_ms
                spx_frame_write_batches = $spx.spx_frame_write_batches
                spx_frame_write_data_bytes = $spx.spx_frame_write_data_bytes
                spx_frame_write_vectored_writes = $spx.spx_frame_write_vectored_writes
                spx_frame_read_data_bytes = $spx.spx_frame_read_data_bytes
                spx_tcp_relay_duration_ms = $spx.last_spx_tcp_relay_duration_ms
                native_first_byte_latency_ms = $native.last_ssh_first_byte_latency_ms
                native_p95_first_byte_latency_ms = $native.p95_ssh_first_byte_latency_ms
                native_channel_queue_depth = $native.ssh_max_channel_queue_depth
                explanation = if ($spxConcurrent -gt $nativeConcurrent) {
                    "spx-over-ssh-direct won concurrent throughput at this pool; inspect SSH direct channel open latency, SPX frame counters, and relay duration for overhead attribution"
                } elseif ($nativeConcurrent -gt $spxConcurrent) {
                    "ssh-native won concurrent throughput at this pool; inspect native first-byte and queue-depth metrics for session scheduling behavior"
                } else {
                    "spx-over-ssh-direct and ssh-native concurrent throughput were tied or unavailable"
                }
            }
            $json = $comparison | ConvertTo-Json -Depth 6 -Compress
            $spx.ssh_vs_spx_overhead = $json
            $native.ssh_vs_spx_overhead = $json
            $comparisons.Add($comparison)
        }
    }
    return $comparisons
}

function Get-OpenSshControlMasterComparisons {
    param([object[]]$Rows)
    $comparisons = New-Object System.Collections.Generic.List[object]
    foreach ($group in ($Rows | Group-Object target)) {
        $target = [string]$group.Name
        $targetRows = @($group.Group)
        $baseline = @($targetRows | Where-Object { $_.case -eq "sshd-D" } | Select-Object -First 1)[0]
        if ($null -eq $baseline) {
            continue
        }
        $baselineValid = Test-BaselineRowValid -Row $baseline
        $bestNative = @($targetRows | Where-Object {
            $_.case -eq "ssh-native-direct" -and (Test-BaselineRowValid -Row $_)
        } | Sort-Object @{ Expression = { Convert-BenchNumber $_.concurrent_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchNumber $_.large_mibps }; Descending = $true } | Select-Object -First 1)[0]
        $bestSpx = @($targetRows | Where-Object {
            $_.case -eq "spx-ssh-direct" -and (Test-BaselineRowValid -Row $_)
        } | Sort-Object @{ Expression = { Convert-BenchNumber $_.concurrent_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchNumber $_.large_mibps }; Descending = $true } | Select-Object -First 1)[0]
        foreach ($cm in @($targetRows | Where-Object { $_.case -like "sshd-D-controlmaster*" })) {
            $cmValid = Test-BaselineRowValid -Row $cm
            $comparisonQuality = Get-BaselineComparisonQuality -Baseline $baseline -Candidate $cm
            $baselineLarge = Convert-BenchNumber $baseline.large_mibps
            $baselineConcurrent = Convert-BenchNumber $baseline.concurrent_mibps
            $cmLarge = Convert-BenchNumber $cm.large_mibps
            $cmConcurrent = Convert-BenchNumber $cm.concurrent_mibps
            $nativeLarge = if ($null -ne $bestNative) { Convert-BenchNumber $bestNative.large_mibps } else { 0.0 }
            $nativeConcurrent = if ($null -ne $bestNative) { Convert-BenchNumber $bestNative.concurrent_mibps } else { 0.0 }
            $spxLarge = if ($null -ne $bestSpx) { Convert-BenchNumber $bestSpx.large_mibps } else { 0.0 }
            $spxConcurrent = if ($null -ne $bestSpx) { Convert-BenchNumber $bestSpx.concurrent_mibps } else { 0.0 }
            $comparisons.Add([pscustomobject]@{
                target = $target
                case = $cm.case
                openssh_control_master_mode = $cm.openssh_control_master_mode
                openssh_control_master_reused = $cm.openssh_control_master_reused
                baseline_mode = $baseline.baseline_mode
                baseline_quality = $baseline.baseline_quality
                baseline_quality_reason = $baseline.baseline_quality_reason
                controlmaster_quality = $cm.baseline_quality
                controlmaster_quality_reason = $cm.baseline_quality_reason
                comparison_quality = $comparisonQuality
                baseline_large_mibps = if ($baselineValid -and $baselineLarge -gt 0) { $baselineLarge } else { $null }
                controlmaster_large_mibps = if ($cmValid -and $cmLarge -gt 0) { $cmLarge } else { $null }
                baseline_concurrent_mibps = if ($baselineValid -and $baselineConcurrent -gt 0) { $baselineConcurrent } else { $null }
                controlmaster_concurrent_mibps = if ($cmValid -and $cmConcurrent -gt 0) { $cmConcurrent } else { $null }
                best_ssh_native_pool = if ($null -ne $bestNative) { Get-BenchPoolSize $bestNative.transport_pool_size } else { $null }
                best_ssh_native_large_mibps = if ($nativeLarge -gt 0) { $nativeLarge } else { $null }
                best_ssh_native_concurrent_mibps = if ($nativeConcurrent -gt 0) { $nativeConcurrent } else { $null }
                best_spx_ssh_pool = if ($null -ne $bestSpx) { Get-BenchPoolSize $bestSpx.transport_pool_size } else { $null }
                best_spx_ssh_large_mibps = if ($spxLarge -gt 0) { $spxLarge } else { $null }
                best_spx_ssh_concurrent_mibps = if ($spxConcurrent -gt 0) { $spxConcurrent } else { $null }
                large_speedup = if ($baselineValid -and $cmValid -and $baselineLarge -gt 0 -and $cmLarge -gt 0) { [Math]::Round($cmLarge / $baselineLarge, 3) } else { $null }
                concurrent_speedup = if ($baselineValid -and $cmValid -and $baselineConcurrent -gt 0 -and $cmConcurrent -gt 0) { [Math]::Round($cmConcurrent / $baselineConcurrent, 3) } else { $null }
                controlmaster_vs_best_ssh_native_concurrent = if ($cmValid -and $nativeConcurrent -gt 0 -and $cmConcurrent -gt 0) { [Math]::Round($cmConcurrent / $nativeConcurrent, 3) } else { $null }
                controlmaster_vs_best_spx_ssh_concurrent = if ($cmValid -and $spxConcurrent -gt 0 -and $cmConcurrent -gt 0) { [Math]::Round($cmConcurrent / $spxConcurrent, 3) } else { $null }
                error = $cm.error
            })
        }
    }
    return $comparisons
}

function Get-TlsOverPlainRatios {
    param([object[]]$Rows)
    $comparisons = New-Object System.Collections.Generic.List[object]
    foreach ($group in ($Rows | Group-Object target)) {
        $target = [string]$group.Name
        $targetRows = @($group.Group)
        $poolSizes = @($targetRows | Where-Object {
            $_.case -in @("spx-plain-direct", "spx-tls-direct") -and
            $null -ne (Get-BenchPoolSize $_.transport_pool_size)
        } | ForEach-Object { Get-BenchPoolSize $_.transport_pool_size } | Sort-Object -Unique)
        foreach ($pool in $poolSizes) {
            $plain = @($targetRows | Where-Object {
                $_.case -eq "spx-plain-direct" -and (Get-BenchPoolSize $_.transport_pool_size) -eq $pool
            } | Select-Object -First 1)[0]
            $tls = @($targetRows | Where-Object {
                $_.case -eq "spx-tls-direct" -and (Get-BenchPoolSize $_.transport_pool_size) -eq $pool
            } | Select-Object -First 1)[0]
            if ($null -eq $plain -or $null -eq $tls) {
                continue
            }
            $plainLarge = Convert-BenchNumber $plain.large_mibps
            $tlsLarge = Convert-BenchNumber $tls.large_mibps
            $plainConcurrent = Convert-BenchNumber $plain.concurrent_mibps
            $tlsConcurrent = Convert-BenchNumber $tls.concurrent_mibps
            $comparison = [pscustomobject]@{
                target = $target
                pool = $pool
                plain_policy = $plain.direct_transport_policy
                tls_policy = $tls.direct_transport_policy
                plain_policy_reason = $plain.direct_transport_policy_reason
                tls_policy_reason = $tls.direct_transport_policy_reason
                tls_peer_auth_mode = $tls.tls_peer_auth_mode
                plain_large_mibps = $plainLarge
                tls_large_mibps = $tlsLarge
                tls_over_plain_large_ratio = if ($plainLarge -gt 0 -and $tlsLarge -gt 0) { [Math]::Round($tlsLarge / $plainLarge, 3) } else { $null }
                plain_concurrent_mibps = $plainConcurrent
                tls_concurrent_mibps = $tlsConcurrent
                tls_over_plain_concurrent_ratio = if ($plainConcurrent -gt 0 -and $tlsConcurrent -gt 0) { [Math]::Round($tlsConcurrent / $plainConcurrent, 3) } else { $null }
                plain_write_frames_per_batch = $plain.spx_write_frames_per_batch
                tls_write_frames_per_batch = $tls.spx_write_frames_per_batch
                plain_write_data_bytes_per_frame = $plain.spx_write_data_bytes_per_frame
                tls_write_data_bytes_per_frame = $tls.spx_write_data_bytes_per_frame
                plain_read_data_bytes_per_frame = $plain.spx_read_data_bytes_per_frame
                tls_read_data_bytes_per_frame = $tls.spx_read_data_bytes_per_frame
                plain_vectored_writes_per_frame = $plain.spx_vectored_writes_per_frame
                tls_vectored_writes_per_frame = $tls.spx_vectored_writes_per_frame
                plain_relay_remote_to_client_mibps = $plain.spx_relay_remote_to_client_mibps
                tls_relay_remote_to_client_mibps = $tls.spx_relay_remote_to_client_mibps
                plain_backpressure_timeouts = $plain.spx_tcp_stream_backpressure_timeouts
                tls_backpressure_timeouts = $tls.spx_tcp_stream_backpressure_timeouts
                explanation = if ($plainLarge -gt 0 -and $tlsLarge -gt 0 -and [Math]::Abs(1.0 - ($tlsLarge / $plainLarge)) -le 0.1) {
                    "TLS large-transfer throughput is within 10% of plain TCP; inspect SPX frame/copy counters before blaming TLS crypto"
                } elseif ($plainLarge -gt $tlsLarge) {
                    "Plain TCP is faster at this pool; compare frame batching and relay-copy counters to separate TLS cost from SPX overhead"
                } else {
                    "TLS/TCP matches or beats plain TCP at this pool while remaining the production direct policy"
                }
            }
            $comparisons.Add($comparison)
        }
    }
    return $comparisons
}

function Get-NativeQuicComparisons {
    param([object[]]$Rows)
    $comparisons = New-Object System.Collections.Generic.List[object]
    foreach ($targetGroup in ($Rows | Group-Object target)) {
        $target = $targetGroup.Name
        $framedRows = @($targetGroup.Group | Where-Object {
            $_.case -eq "spx-quic-direct" -and
            [string]::IsNullOrWhiteSpace([string]$_.error) -and
            [int]$_.large_exit -eq 0
        })
        $nativeRows = @($targetGroup.Group | Where-Object {
            $_.case -eq "quic-native-direct" -and
            [string]::IsNullOrWhiteSpace([string]$_.error) -and
            [int]$_.large_exit -eq 0
        })
        $allQuicPoolRows = @($framedRows) + @($nativeRows)
        $matchingPools = @($allQuicPoolRows | ForEach-Object { [int]$_.transport_pool_size } | Sort-Object -Unique)
        foreach ($pool in $matchingPools) {
            $framedPoolRows = @($framedRows | Where-Object { [int]$_.transport_pool_size -eq $pool })
            $nativePoolRows = @($nativeRows | Where-Object { [int]$_.transport_pool_size -eq $pool })
            if ($framedPoolRows.Count -eq 0 -or $nativePoolRows.Count -eq 0) {
                continue
            }
            $framed = $framedPoolRows[0]
            $native = $nativePoolRows[0]
            $comparisons.Add((New-NativeQuicComparison `
                -Target $target `
                -Scope "matching_pool" `
                -Framed $framed `
                -Native $native `
                -ReferenceFramedPool $framed))
        }
        $framedBest = @($framedRows | Sort-Object `
            @{ Expression = { Convert-BenchNumber $_.large_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchNumber $_.concurrent_mibps }; Descending = $true } |
            Select-Object -First 1)
        $framedPool8 = @($framedRows | Where-Object { [int]$_.transport_pool_size -eq 8 } | Select-Object -First 1)
        $nativeBest = @($nativeRows | Sort-Object `
            @{ Expression = { Convert-BenchNumber $_.large_mibps }; Descending = $true },
            @{ Expression = { Convert-BenchNumber $_.concurrent_mibps }; Descending = $true } |
            Select-Object -First 1)
        $nativePool8 = @($nativeRows | Where-Object { [int]$_.transport_pool_size -eq 8 } | Select-Object -First 1)
        if ($framedBest.Count -eq 0 -and $nativeBest.Count -eq 0) {
            continue
        }
        $framed = if ($framedBest.Count -gt 0) { $framedBest[0] } else { $null }
        $native = if ($nativeBest.Count -gt 0) { $nativeBest[0] } else { $null }
        $framed8 = if ($framedPool8.Count -gt 0) { $framedPool8[0] } else { $null }
        $native8 = if ($nativePool8.Count -gt 0) { $nativePool8[0] } else { $null }
        $comparisons.Add((New-NativeQuicComparison `
            -Target $target `
            -Scope "best_overall" `
            -Framed $framed `
            -Native $native `
            -ReferenceFramedPool $framed8 `
            -NativePoolSizes @($nativeRows | ForEach-Object { [int]$_.transport_pool_size } | Sort-Object -Unique)))
        if ($null -ne $framed8 -and $null -ne $native8) {
            $comparisons.Add((New-NativeQuicComparison `
                -Target $target `
                -Scope "pool8" `
                -Framed $framed8 `
                -Native $native8 `
                -ReferenceFramedPool $framed8))
        }
    }
    return $comparisons
}

function New-NativeQuicComparison {
    param(
        [string]$Target,
        [string]$Scope,
        $Framed,
        $Native,
        $ReferenceFramedPool = $null,
        [int[]]$NativePoolSizes = @()
    )
    $framedLarge = if ($null -ne $Framed) { Convert-BenchNumber $Framed.large_mibps } else { 0.0 }
    $nativeLarge = if ($null -ne $Native) { Convert-BenchNumber $Native.large_mibps } else { 0.0 }
    $framedConcurrent = if ($null -ne $Framed) { Convert-BenchNumber $Framed.concurrent_mibps } else { 0.0 }
    $nativeConcurrent = if ($null -ne $Native) { Convert-BenchNumber $Native.concurrent_mibps } else { 0.0 }
    $referenceLarge = if ($null -ne $ReferenceFramedPool) { Convert-BenchNumber $ReferenceFramedPool.large_mibps } else { 0.0 }
    $referenceConcurrent = if ($null -ne $ReferenceFramedPool) { Convert-BenchNumber $ReferenceFramedPool.concurrent_mibps } else { 0.0 }
    $largeSpeedup = if ($framedLarge -gt 0 -and $nativeLarge -gt 0) { [Math]::Round($nativeLarge / $framedLarge, 3) } else { $null }
    $concurrentSpeedup = if ($framedConcurrent -gt 0 -and $nativeConcurrent -gt 0) { [Math]::Round($nativeConcurrent / $framedConcurrent, 3) } else { $null }
    $suspects = Get-NativeQuicBottleneckSuspects -Native $Native -Framed $Framed -LargeSpeedup $largeSpeedup -ConcurrentSpeedup $concurrentSpeedup
    [pscustomobject]@{
        target = $Target
        comparison_scope = $Scope
        pool = if ($null -ne $Framed) { [int]$Framed.transport_pool_size } elseif ($null -ne $Native) { [int]$Native.transport_pool_size } else { $null }
        framed_pool_size = if ($null -ne $Framed) { [int]$Framed.transport_pool_size } else { $null }
        framed_large_mibps = $framedLarge
        framed_concurrent_mibps = $framedConcurrent
        reference_framed_pool_size = if ($null -ne $ReferenceFramedPool) { [int]$ReferenceFramedPool.transport_pool_size } else { $null }
        reference_framed_large_mibps = $referenceLarge
        reference_framed_concurrent_mibps = $referenceConcurrent
        native_pool_sizes_tested = if ($NativePoolSizes.Count -gt 0) { $NativePoolSizes -join "," } else { $null }
        native_pool_size = if ($null -ne $Native) { [int]$Native.transport_pool_size } else { $null }
        native_large_mibps = $nativeLarge
        native_concurrent_mibps = $nativeConcurrent
        native_first_byte_samples = if ($null -ne $Native) { $Native.quic_flow_first_byte_samples } else { $null }
        native_max_first_byte_latency_ms = if ($null -ne $Native) { $Native.max_quic_flow_first_byte_latency_ms } else { $null }
        native_stream_open_failures = if ($null -ne $Native) { $Native.quic_stream_open_failures } else { $null }
        native_header_write_failures = if ($null -ne $Native) { $Native.quic_header_write_failures } else { $null }
        native_copy_failures = if ($null -ne $Native) { $Native.quic_copy_failures } else { $null }
        native_backpressure_timeouts = if ($null -ne $Native) { $Native.quic_backpressure_timeouts } else { $null }
        native_max_copy_duration_ms = if ($null -ne $Native) { $Native.max_quic_copy_duration_ms } else { $null }
        native_resets = if ($null -ne $Native) { $Native.quic_flow_resets } else { $null }
        native_control_degraded = if ($null -ne $Native) { $Native.control_degraded } else { $null }
        native_control_state = if ($null -ne $Native) { $Native.control_state } else { $null }
        native_connection_pool_size = if ($null -ne $Native) { $Native.quic_connection_pool_size } else { $null }
        native_connection_pool_policy = if ($null -ne $Native) { $Native.quic_connection_pool_policy } else { $null }
        native_connection_pool_workload_hint = if ($null -ne $Native) { $Native.quic_connection_pool_workload_hint } else { $null }
        native_connection_pool_reason = if ($null -ne $Native) { $Native.quic_connection_pool_reason } else { $null }
        native_connection_pool_mode = if ($null -ne $Native) { $Native.quic_connection_pool_mode } else { $null }
        native_connection_pool_selection_policy = if ($null -ne $Native) { $Native.quic_connection_pool_selection_policy } else { $null }
        native_active_quic_connections = if ($null -ne $Native) { $Native.active_quic_connections } else { $null }
        native_connection_opened_flows = if ($null -ne $Native) { $Native.quic_connection_opened_flows } else { $null }
        native_connection_control_states = if ($null -ne $Native) { $Native.quic_connection_control_states } else { $null }
        native_connection_control_degraded = if ($null -ne $Native) { $Native.quic_connection_control_degraded } else { $null }
        framed_control_degraded = if ($null -ne $Framed) { $Framed.control_degraded } else { $null }
        framed_control_state = if ($null -ne $Framed) { $Framed.control_state } else { $null }
        framed_resets = if ($null -ne $Framed) { $Framed.quic_flow_resets } else { $null }
        framed_max_copy_duration_ms = if ($null -ne $Framed) { $Framed.max_quic_copy_duration_ms } else { $null }
        native_quic_udp_runtime = if ($null -ne $Native) { $Native.quic_udp_runtime } else { $null }
        native_quic_udp_gso = if ($null -ne $Native) { $Native.quic_udp_gso } else { $null }
        native_quic_packetization = if ($null -ne $Native) { $Native.quic_packetization } else { $null }
        large_speedup = $largeSpeedup
        concurrent_speedup = $concurrentSpeedup
        closes_gap_with_reference_large = if ($referenceLarge -gt 0 -and $nativeLarge -gt 0) { [Math]::Round($nativeLarge / $referenceLarge, 3) } else { $null }
        closes_gap_with_reference_concurrent = if ($referenceConcurrent -gt 0 -and $nativeConcurrent -gt 0) { [Math]::Round($nativeConcurrent / $referenceConcurrent, 3) } else { $null }
        bottleneck_suspects = $suspects
        explanation = Get-NativeQuicComparisonExplanation -Suspects $suspects -LargeSpeedup $largeSpeedup -ConcurrentSpeedup $concurrentSpeedup
    }
}

function Get-NativeQuicBottleneckSuspects {
    param($Native, $Framed, $LargeSpeedup, $ConcurrentSpeedup)
    $suspects = New-Object System.Collections.Generic.List[string]
    if ($null -eq $Native -or $null -eq $Framed) {
        $suspects.Add("missing_matching_result")
        return $suspects.ToArray()
    }
    $nativeConnections = Convert-BenchInt $Native.active_quic_connections 0
    $framedPool = Convert-BenchInt $Framed.transport_pool_size 0
    if ($framedPool -gt 0 -and $nativeConnections -gt 0 -and $nativeConnections -lt $framedPool) {
        $suspects.Add("connection_count")
    }
    if ((Convert-BenchInt $Native.quic_stream_open_failures 0) -gt 0 -or (Convert-BenchInt $Native.quic_header_write_failures 0) -gt 0) {
        $suspects.Add("stream_lifecycle")
    }
    if ((Convert-BenchInt $Native.quic_flow_resets 0) -gt 0 -or (Convert-BenchInt $Native.quic_copy_failures 0) -gt 0 -or (Convert-BenchInt $Native.quic_backpressure_timeouts 0) -gt 0) {
        $suspects.Add("data_copy")
    }
    if ([string]::IsNullOrWhiteSpace([string]$Native.quic_udp_gso) -or [string]::IsNullOrWhiteSpace([string]$Native.quic_packetization)) {
        $suspects.Add("udp_path")
    }
    if ([string]$Native.control_degraded -eq "True" -or [string]$Native.control_degraded -eq "true" -or [string]$Native.quic_connection_control_degraded -match "true") {
        $suspects.Add("control_degradation")
    }
    if (($null -ne $LargeSpeedup -and $LargeSpeedup -lt 1.0) -or ($null -ne $ConcurrentSpeedup -and $ConcurrentSpeedup -lt 1.0)) {
        if ($suspects.Count -eq 0) {
            $suspects.Add("unexplained_native_slowdown")
        }
    }
    if ($suspects.Count -eq 0) {
        $suspects.Add("native_not_slower_or_inconclusive")
    }
    return $suspects.ToArray()
}

function Get-NativeQuicComparisonExplanation {
    param($Suspects, $LargeSpeedup, $ConcurrentSpeedup)
    $speed = "large_speedup=$LargeSpeedup concurrent_speedup=$ConcurrentSpeedup"
    if ($Suspects -contains "connection_count") {
        return "$speed; native active connection count is lower than the framed pool for this comparison"
    }
    if ($Suspects -contains "stream_lifecycle") {
        return "$speed; stream-open or header-write failures point at native stream lifecycle overhead"
    }
    if ($Suspects -contains "data_copy") {
        return "$speed; reset/copy/backpressure counters point at native data-copy behavior"
    }
    if ($Suspects -contains "udp_path") {
        return "$speed; UDP/GSO/packetization diagnostics are incomplete, so the UDP path remains a suspect"
    }
    if ($Suspects -contains "control_degradation") {
        return "$speed; control health degraded during the native run"
    }
    return "$speed; no single native bottleneck was proven by available counters"
}

function Get-QuicProfileSummaries {
    param([object[]]$Rows)
    $summaries = New-Object System.Collections.Generic.List[object]
    foreach ($row in @($Rows | Where-Object { $_.quic_profile_enabled -eq $true })) {
        $suspects = New-Object System.Collections.Generic.List[string]
        if (-not [string]::IsNullOrWhiteSpace([string]$row.error_kind) -or -not [string]::IsNullOrWhiteSpace([string]$row.error)) {
            $suspects.Add("startup_or_connect_failure")
        }
        if ([string]::IsNullOrWhiteSpace([string]$row.quic_profile_packet_loss)) {
            $suspects.Add("packet_loss_unknown")
        }
        if ([string]::IsNullOrWhiteSpace([string]$row.quic_profile_max_datagram_size)) {
            $suspects.Add("packetization_unknown")
        }
        if ((Convert-BenchInt $row.quic_stream_open_failures 0) -gt 0 -or (Convert-BenchInt $row.quic_header_write_failures 0) -gt 0) {
            $suspects.Add("stream_lifecycle")
        }
        if ((Convert-BenchInt $row.quic_copy_failures 0) -gt 0 -or (Convert-BenchInt $row.quic_flow_resets 0) -gt 0) {
            $suspects.Add("copy_or_reset")
        }
        if ((Convert-BenchInt $row.quic_backpressure_timeouts 0) -gt 0) {
            $suspects.Add("backpressure_timeout")
        }
        if ([string]$row.control_degraded -eq "True" -or [string]$row.control_degraded -eq "true") {
            $suspects.Add("control_degraded")
        }
        $largeMibps = Convert-BenchNumber $row.large_mibps
        if ($largeMibps -gt 0 -and $largeMibps -lt 5.0) {
            $suspects.Add("low_single_connection_throughput")
        }
        if ((Convert-BenchNumber $row.quic_profile_process_total_cpu_delta_ms) -gt 0 -and (Convert-BenchNumber $row.large_seconds) -gt 0) {
            $cpuRatio = [Math]::Round((Convert-BenchNumber $row.quic_profile_process_total_cpu_delta_ms) / ((Convert-BenchNumber $row.large_seconds) * 1000.0), 3)
        } else {
            $cpuRatio = $null
        }
        $summaries.Add([pscustomobject]@{
            target = $row.target
            case = $row.case
            pool = $row.transport_pool_size
            large_mibps = $row.large_mibps
            concurrent_mibps = $row.concurrent_mibps
            quic_max_bidi_streams = $row.quic_max_bidi_streams
            quic_stream_receive_window = $row.quic_stream_receive_window
            quic_receive_window = $row.quic_receive_window
            quic_keep_alive_interval_secs = $row.quic_keep_alive_interval_secs
            quic_idle_timeout_secs = $row.quic_idle_timeout_secs
            quic_copy_buffer_size = $row.quic_copy_buffer_size
            quic_stream_open_timeout_secs = $row.quic_stream_open_timeout_secs
            quic_backpressure_timeout_secs = $row.quic_backpressure_timeout_secs
            quic_backpressure_timeouts = $row.quic_backpressure_timeouts
            quic_udp_runtime = $row.quic_udp_runtime
            quic_udp_gso = $row.quic_udp_gso
            quic_udp_gso_source = $row.quic_udp_gso_source
            quic_packetization = $row.quic_packetization
            quic_profile_next_bottleneck = $row.quic_profile_next_bottleneck
            packet_loss_source = $row.quic_profile_packet_loss_source
            max_datagram_size_source = $row.quic_profile_max_datagram_size_source
            process_cpu_delta_ms = $row.quic_profile_process_total_cpu_delta_ms
            process_cpu_ratio_to_large_wall = $cpuRatio
            process_peak_working_set_bytes = $row.quic_profile_process_peak_working_set_bytes
            bottleneck_suspects = if ($suspects.Count -gt 0) { $suspects.ToArray() } else { @("inconclusive") }
        })
    }
    return $summaries
}

function Get-QuicProfileRecommendations {
    param([object[]]$Rows)
    $profiles = @($Rows | Where-Object { $_.quic_profile_enabled -eq $true -and (Test-QuicBenchmarkCase -Case $_.case) -and $null -ne $_.large_mibps -and $null -ne $_.concurrent_mibps })
    if ($profiles.Count -eq 0) {
        return $null
    }
    $recommendationFor = {
        param([string]$Workload)
        $ranked = @(
            $profiles | Sort-Object @{
                Expression = {
                    $large = Convert-BenchNumber $_.large_mibps
                    $concurrent = Convert-BenchNumber $_.concurrent_mibps
                    switch ($Workload) {
                        "large_flow" { $large }
                        "high_concurrency" { $concurrent }
                        default {
                            if ($large -gt 0 -and $concurrent -gt 0) {
                                [Math]::Round(($large + $concurrent) / 2.0, 3)
                            } else {
                                0.0
                            }
                        }
                    }
                }
                Descending = $true
            },
            @{
                Expression = { Convert-BenchNumber $_.quic_backpressure_timeouts 0 }
                Descending = $false
            },
            @{
                Expression = { if ([string]::IsNullOrWhiteSpace([string]$_.control_degraded) -or [string]$_.control_degraded -eq "False" -or [string]$_.control_degraded -eq "false") { 0 } else { 1 } }
                Descending = $false
            }
        | Select-Object -First 1)
        if ($ranked.Count -eq 0) {
            return $null
        }
        $row = $ranked[0]
        [pscustomobject]@{
            workload = $Workload
            target = $row.target
            case = $row.case
            selected_protocol = $row.selected_protocol
            pool = $row.transport_pool_size
            large_mibps = $row.large_mibps
            concurrent_mibps = $row.concurrent_mibps
            control_health = $row.link_control_health
            failure_kind = if ([string]::IsNullOrWhiteSpace([string]$row.error_kind)) { $null } else { $row.error_kind }
            parameter_set = [pscustomobject]@{
                max_bidi_streams = $row.quic_max_bidi_streams
                stream_receive_window = $row.quic_stream_receive_window
                receive_window = $row.quic_receive_window
                keep_alive_interval_secs = $row.quic_keep_alive_interval_secs
                idle_timeout_secs = $row.quic_idle_timeout_secs
            }
            next_bottleneck = $row.quic_profile_next_bottleneck
            reason = switch ($Workload) {
                "large_flow" { "picked the profiled QUIC row with the highest large-transfer throughput while preferring healthy control and fewer backpressure signals" }
                "high_concurrency" { "picked the profiled QUIC row with the highest concurrent throughput while preferring healthy control and fewer backpressure signals" }
                default { "picked the profiled QUIC row with the best balanced large and concurrent throughput while preferring healthy control and fewer backpressure signals" }
            }
        }
    }
    [pscustomobject]@{
        large_flow = & $recommendationFor "large_flow"
        high_concurrency = & $recommendationFor "high_concurrency"
        mixed = & $recommendationFor "mixed"
    }
}

function Get-QuicProfileSweepPlan {
    [pscustomobject]@{
        target = "direct-peer"
        scope = "single-connection QUIC throughput"
        pool_size = 1
        max_bidi_streams = @($QuicMaxBidiStreams, [Math]::Min($QuicMaxBidiStreams * 2, 4096)) | Sort-Object -Unique
        stream_receive_window = @($QuicStreamReceiveWindow, [Math]::Min($QuicStreamReceiveWindow * 2, 67108864), [Math]::Min($QuicStreamReceiveWindow * 4, 67108864)) | Sort-Object -Unique
        receive_window = @($QuicReceiveWindow, [Math]::Min($QuicReceiveWindow * 2, 268435456), [Math]::Min($QuicReceiveWindow * 4, 268435456)) | Sort-Object -Unique
        keep_alive_interval_secs = @($QuicKeepAliveIntervalSecs, 5, 10, 20) | Sort-Object -Unique
        idle_timeout_secs = @($QuicIdleTimeoutSecs, 60, 120) | Sort-Object -Unique
        command_hint = "Run with -QuicProfile -Targets <direct-peer> -TransportPoolSizes 1 and vary the listed --quic-* values to isolate window, UDP, and copy bottlenecks."
    }
}

$results = New-Object System.Collections.Generic.List[object]
$remotePlans = @()
$cleanupResults = New-Object System.Collections.Generic.List[object]
$binaryVersions = New-Object System.Collections.Generic.List[object]
$remotePlatforms = New-Object System.Collections.Generic.List[object]
$script:LocalPlatform = Get-LocalPlatform
$script:OpenSshCapability = Get-OpenSshCapability
$script:CurrentRemoteOs = $null
$script:CurrentRemoteArch = $null
$localBinaryVersion = & $localBin --version 2>&1
$localBinaryVersion = ($localBinaryVersion | Out-String).Trim()
$binaryVersions.Add([pscustomobject]@{
    target = "local"
    local = $localBinaryVersion
    remote = $null
})
$invocationCommand = Get-BenchmarkInvocation `
    -ScenarioName $scenarioName `
    -ScenarioRunLevel $runLevelName `
    -ScenarioTargets $Targets `
    -ScenarioUseRemotePayload ([bool]$UseRemotePayload) `
    -ScenarioPayloadMiB $PayloadMiB `
    -ScenarioConcurrency $Concurrency `
    -ScenarioTransportPoolSizes $TransportPoolSizes `
    -ScenarioTransportPoolSize $TransportPoolSize `
    -ScenarioQuicMaxBidiStreams $QuicMaxBidiStreams `
    -ScenarioQuicStreamReceiveWindow $QuicStreamReceiveWindow `
    -ScenarioQuicReceiveWindow $QuicReceiveWindow `
    -ScenarioQuicKeepAliveIntervalSecs $QuicKeepAliveIntervalSecs `
    -ScenarioQuicIdleTimeoutSecs $QuicIdleTimeoutSecs `
    -ScenarioQuicDebugLog ([bool]$QuicDebugLog) `
    -ScenarioQuicProfile ([bool]$QuicProfile) `
    -ScenarioSpxProfile ([bool]$SpxProfile) `
    -ScenarioSshConnectTimeout $SshConnectTimeout `
    -ScenarioRemoteCommandTimeout $RemoteCommandTimeout `
    -ScenarioSkipDirect ([bool]$SkipDirect) `
    -ScenarioRespectPreflightSkip ([bool]$RespectPreflightSkip) `
    -ScenarioIncludeSshControlMaster ([bool]$IncludeSshControlMaster) `
    -ScenarioSshControlMasterBaseline $SshControlMasterBaseline `
    -ScenarioSshControlPersistSecs $SshControlPersistSecs `
    -ScenarioStabilityDurationSecs $StabilityDurationSecs `
    -ScenarioStabilitySmallIntervalSecs $StabilitySmallIntervalSecs `
    -ScenarioStabilityInjectRemoteDaemonRestart ([bool]$StabilityInjectRemoteDaemonRestart) `
    -ScenarioKeepRemote ([bool]$KeepRemote) `
    -ScenarioCleanupStamp $CleanupStamp `
    -ScenarioResumeFromResults $ResumeFromResults `
    -ScenarioUrl $Url
$script:CurrentRunLevel = $runLevelName
$script:CurrentTransportPoolSize = $TransportPoolSizes[0]
$script:PreflightNegativeCache = @{}
$script:ResumeRowsByKey = @{}
$script:ResumeEvidence = New-Object System.Collections.Generic.List[object]
$script:ResumeSource = $null
$script:ResumeEnabled = $false

if (-not [string]::IsNullOrWhiteSpace($ResumeFromResults)) {
    if (-not (Test-Path -LiteralPath $ResumeFromResults)) {
        throw "resume results file not found: $ResumeFromResults"
    }
    $resumeSummary = Get-Content -Raw -LiteralPath $ResumeFromResults | ConvertFrom-Json
    $resumeMismatch = New-Object System.Collections.Generic.List[string]
    if ([string]$resumeSummary.run_level -ne $runLevelName) {
        $resumeMismatch.Add("run_level=$($resumeSummary.run_level) != $runLevelName")
    }
    if ([int]$resumeSummary.payload_mib -ne $PayloadMiB) {
        $resumeMismatch.Add("payload_mib=$($resumeSummary.payload_mib) != $PayloadMiB")
    }
    if ([int]$resumeSummary.concurrency -ne $Concurrency) {
        $resumeMismatch.Add("concurrency=$($resumeSummary.concurrency) != $Concurrency")
    }
    $resumePoolSizes = @($resumeSummary.transport_pool_sizes | ForEach-Object { [string]$_ }) -join ","
    $currentPoolSizes = @($TransportPoolSizes | ForEach-Object { [string]$_ }) -join ","
    if ($resumePoolSizes -ne $currentPoolSizes) {
        $resumeMismatch.Add("transport_pool_sizes=$resumePoolSizes != $currentPoolSizes")
    }
    if ($resumeMismatch.Count -gt 0) {
        Write-Warning "resume disabled because result metadata differs: $($resumeMismatch -join '; ')"
    } else {
        foreach ($row in @($resumeSummary.results)) {
            if (-not (Test-ResumableResultRow -Row $row)) {
                continue
            }
            $pool = Convert-BenchInt $row.transport_pool_size 0
            $key = New-BenchmarkCaseKey -RunLevelValue $runLevelName -Target ([string]$row.target) -Case ([string]$row.case) -PoolSize $pool
            if (-not $script:ResumeRowsByKey.ContainsKey($key)) {
                $script:ResumeRowsByKey[$key] = $row
            }
        }
        $script:ResumeSource = (Resolve-Path -LiteralPath $ResumeFromResults).Path
        $script:ResumeEnabled = $true
        Write-Host "resume_enabled=true"
        Write-Host "resume_reusable_rows=$($script:ResumeRowsByKey.Count)"
    }
}

function Add-BenchmarkResult {
    param(
        [string]$Target,
        [string]$Case,
        [int]$PoolSize,
        [scriptblock]$Measure
    )
    $key = New-BenchmarkCaseKey -RunLevelValue $script:CurrentRunLevel -Target $Target -Case $Case -PoolSize $PoolSize
    if ($script:ResumeEnabled -and $script:ResumeRowsByKey.ContainsKey($key)) {
        $row = Copy-ResultRowForResume -Row $script:ResumeRowsByKey[$key] -Source $script:ResumeSource -Key $key
        $results.Add($row)
        $script:ResumeEvidence.Add([pscustomobject]@{
            target = $Target
            case = $Case
            transport_pool_size = $PoolSize
            key = $key
            source = $script:ResumeSource
            skipped_runtime = $true
        })
        return
    }
    $results.Add((& $Measure))
}

function Get-ExpectedBenchmarkCaseSpecs {
    $specs = New-Object System.Collections.Generic.List[object]
    if ($script:CurrentRunLevel -eq "stability") {
        $pool = [int]$TransportPoolSizes[0]
        $specs.Add([pscustomobject]@{ case = "spx-ssh-direct"; pool = $pool })
        if (-not $SkipDirect) {
            $specs.Add([pscustomobject]@{ case = "spx-tls-direct"; pool = $pool })
            $specs.Add([pscustomobject]@{ case = "spx-quic-direct"; pool = $pool })
            $specs.Add([pscustomobject]@{ case = "quic-native-direct"; pool = $pool })
        }
        return $specs
    }

    $specs.Add([pscustomobject]@{ case = "sshd-D"; pool = 0 })
    if ($IncludeSshControlMaster) {
        if ($SshControlMasterBaseline -eq "fresh" -or $SshControlMasterBaseline -eq "both") {
            $specs.Add([pscustomobject]@{ case = "sshd-D-controlmaster-fresh"; pool = 0 })
        }
        if ($SshControlMasterBaseline -eq "reused" -or $SshControlMasterBaseline -eq "both") {
            $specs.Add([pscustomobject]@{ case = "sshd-D-controlmaster"; pool = 0 })
        }
    }
    foreach ($pool in $TransportPoolSizes) {
        $poolSize = [int]$pool
        $specs.Add([pscustomobject]@{ case = "spx-ssh-direct"; pool = $poolSize })
        $specs.Add([pscustomobject]@{ case = "ssh-native-direct"; pool = $poolSize })
        if (-not $SkipDirect) {
            $specs.Add([pscustomobject]@{ case = "spx-plain-direct"; pool = $poolSize })
            $specs.Add([pscustomobject]@{ case = "spx-tls-direct"; pool = $poolSize })
            $specs.Add([pscustomobject]@{ case = "spx-quic-direct"; pool = $poolSize })
            $specs.Add([pscustomobject]@{ case = "quic-native-direct"; pool = $poolSize })
        }
    }
    return $specs
}

function Test-ResumeHasTargetCases {
    param([string]$Target, [object[]]$Specs)
    if (-not $script:ResumeEnabled) {
        return $false
    }
    foreach ($spec in $Specs) {
        $key = New-BenchmarkCaseKey -RunLevelValue $script:CurrentRunLevel -Target $Target -Case ([string]$spec.case) -PoolSize ([int]$spec.pool)
        if (-not $script:ResumeRowsByKey.ContainsKey($key)) {
            return $false
        }
    }
    return $true
}

try {
    foreach ($target in $Targets) {
        $safeTarget = $target -replace '[^A-Za-z0-9_.-]', '_'
        $remoteDir = "/tmp/transport-bench-$stamp-$safeTarget"
        $base = 42000 + (Get-Random -Minimum 0 -Maximum 1000)
        $control = $base
        $plain = $base + 1
        $tls = $base + 2
        $quic = $base + 3
        $http = $base + 4
        $token = "bench-$stamp-$safeTarget"
        $directHost = Get-DirectHost $target
        $caseUrl = $Url
        $script:CurrentRemoteOs = $null
        $script:CurrentRemoteArch = $null
        $expectedSpecs = Get-ExpectedBenchmarkCaseSpecs
        if (Test-ResumeHasTargetCases -Target $target -Specs $expectedSpecs) {
            foreach ($spec in $expectedSpecs) {
                Add-BenchmarkResult -Target $target -Case ([string]$spec.case) -PoolSize ([int]$spec.pool) -Measure {
                    throw "unexpected runtime execution for fully resumed case"
                }
            }
            continue
        }

        try {
            $remotePlatform = Get-RemotePlatform -Target $target
            $script:CurrentRemoteOs = $remotePlatform.os
            $script:CurrentRemoteArch = $remotePlatform.arch
            $remotePlatforms.Add([pscustomobject]@{
                target = $target
                os = $remotePlatform.os
                arch = $remotePlatform.arch
            })
            Invoke-Remote $target "rm -rf '$remoteDir'; mkdir -p '$remoteDir/home'"
            Copy-Remote $target $remoteBinSource "$remoteDir/ssh_proxy"
            Copy-Remote $target $cert "$remoteDir/cert.pem"
            Copy-Remote $target $key "$remoteDir/key.pem"
            Copy-Remote $target $rangeServer "$remoteDir/range_server.py"
            if ($UseRemotePayload) {
                Invoke-Remote $target "dd if=/dev/zero of='$remoteDir/payload.bin' bs=1M count=$PayloadMiB status=none; cd '$remoteDir'; nohup python3 '$remoteDir/range_server.py' 127.0.0.1 $http > '$remoteDir/http.log' 2>&1 < /dev/null & echo `$! > '$remoteDir/http.pid'"
                $payloadReady = $false
                for ($i = 0; $i -lt 20; $i++) {
                    $sshArgs = (Get-SshOptions) + @($target, "curl -fsS --max-time 5 -r 0-0 -o /dev/null 'http://127.0.0.1:$http/payload.bin' >/dev/null 2>&1")
                    & ssh.exe @sshArgs
                    if ($LASTEXITCODE -eq 0) {
                        $payloadReady = $true
                        break
                    }
                    Start-Sleep -Milliseconds 500
                }
                if (-not $payloadReady) {
                    $sshArgs = (Get-SshOptions) + @($target, "cat '$remoteDir/http.log' 2>/dev/null || true")
                    $log = & ssh.exe @sshArgs
                    throw "remote payload server on $target port $http did not become ready: $log"
                }
                $caseUrl = "http://127.0.0.1:$http/payload.bin"
            }
            $quicLogFilter = Get-QuicLogFilter
            Invoke-Remote $target "chmod 700 '$remoteDir/ssh_proxy'; SSH_PROXY_HOME='$remoteDir/home' nohup '$remoteDir/ssh_proxy' --log '$quicLogFilter' node daemon --control tcp://127.0.0.1:$control --transport 0.0.0.0:$plain --tls-transport 0.0.0.0:$tls --quic-transport 0.0.0.0:$quic --quic-max-bidi-streams $QuicMaxBidiStreams --quic-stream-receive-window $QuicStreamReceiveWindow --quic-receive-window $QuicReceiveWindow --quic-keep-alive-interval-secs $QuicKeepAliveIntervalSecs --quic-idle-timeout-secs $QuicIdleTimeoutSecs --tls-cert '$remoteDir/cert.pem' --tls-key '$remoteDir/key.pem' --token '$token' --routes-path '$remoteDir/routes.json' --no-route-autostart > '$remoteDir/daemon.log' 2>&1 < /dev/null & echo `$! > '$remoteDir/daemon.pid'"

            $ready = $false
            for ($i = 0; $i -lt 40; $i++) {
                $sshArgs = (Get-SshOptions) + @($target, "'$remoteDir/ssh_proxy' --log warn node control --endpoint tcp://127.0.0.1:$control --token '$token' status >/dev/null 2>&1")
                & ssh.exe @sshArgs
                if ($LASTEXITCODE -eq 0) {
                    $ready = $true
                    break
                }
                Start-Sleep -Milliseconds 500
            }
            if (-not $ready) {
                throw "remote daemon on $target did not become ready"
            }
            $remoteBinaryVersion = Invoke-RemoteCapture $target "'$remoteDir/ssh_proxy' --version"
            $binaryVersions.Add([pscustomobject]@{
                target = $target
                local = $localBinaryVersion
                remote = $remoteBinaryVersion
            })
        }
        catch {
            $results.Add((New-ErrorResult -Target $target -Case "remote-daemon-setup" -Message $_.Exception.Message))
            continue
        }

        $remotePlan = New-RemoteBenchPlan -Target $target -Stamp $stamp -DirectHost $directHost -Control $control -Plain $plain -Tls $tls -Quic $quic -Http $http
        $remotePlans += $remotePlan

        if ($runLevelName -eq "stability") {
            $script:CurrentTransportPoolSize = $TransportPoolSizes[0]
            Add-BenchmarkResult -Target $target -Case "spx-ssh-direct" -PoolSize $script:CurrentTransportPoolSize -Measure {
                Measure-StabilityProxyCase -Target $target -Case "spx-ssh-direct" -CaseUrl $caseUrl -ProxyArgs @("--remote-transport", "tcp", "--remote-tcp", "127.0.0.1:$plain", "--remote-token", $token) -RemotePlan $remotePlan
            }
            if (-not $SkipDirect) {
                Add-BenchmarkResult -Target $target -Case "spx-tls-direct" -PoolSize $script:CurrentTransportPoolSize -Measure {
                    Measure-StabilityProxyCase -Target $target -Case "spx-tls-direct" -CaseUrl $caseUrl -ProxyArgs @("--remote-transport", "tls-tcp", "--remote-tls", "${directHost}:$tls", "--remote-ca", $cert, "--remote-name", "ssh-proxy-bench", "--remote-token", $token) -RemotePlan $remotePlan
                }
                Add-BenchmarkResult -Target $target -Case "spx-quic-direct" -PoolSize $script:CurrentTransportPoolSize -Measure {
                    Measure-StabilityProxyCase -Target $target -Case "spx-quic-direct" -CaseUrl $caseUrl -ProxyArgs @("--remote-transport", "quic", "--remote-quic", "${directHost}:$quic", "--remote-ca", $cert, "--remote-name", "ssh-proxy-bench", "--remote-token", $token) -RemotePlan $remotePlan
                }
                Add-BenchmarkResult -Target $target -Case "quic-native-direct" -PoolSize $script:CurrentTransportPoolSize -Measure {
                    Measure-StabilityProxyCase -Target $target -Case "quic-native-direct" -CaseUrl $caseUrl -ProxyArgs @("--remote-transport", "quic-native", "--remote-quic", "${directHost}:$quic", "--remote-ca", $cert, "--remote-name", "ssh-proxy-bench") -RemotePlan $remotePlan
                }
            }
        } else {
            $script:CurrentTransportPoolSize = 0
            Add-BenchmarkResult -Target $target -Case "sshd-D" -PoolSize 0 -Measure {
                Measure-SshdBaseline -Target $target -CaseUrl $caseUrl
            }
            if ($IncludeSshControlMaster) {
                if ($SshControlMasterBaseline -eq "fresh" -or $SshControlMasterBaseline -eq "both") {
                    Add-BenchmarkResult -Target $target -Case "sshd-D-controlmaster-fresh" -PoolSize 0 -Measure {
                        Measure-SshdControlMasterBaseline -Target $target -CaseUrl $caseUrl -Mode "fresh"
                    }
                }
                if ($SshControlMasterBaseline -eq "reused" -or $SshControlMasterBaseline -eq "both") {
                    Add-BenchmarkResult -Target $target -Case "sshd-D-controlmaster" -PoolSize 0 -Measure {
                        Measure-SshdControlMasterBaseline -Target $target -CaseUrl $caseUrl -Mode "reused"
                    }
                }
            }

            foreach ($poolSize in $TransportPoolSizes) {
                $script:CurrentTransportPoolSize = $poolSize
                Add-BenchmarkResult -Target $target -Case "spx-ssh-direct" -PoolSize $poolSize -Measure {
                    Measure-ProxyCase -Target $target -Case "spx-ssh-direct" -CaseUrl $caseUrl -ProxyArgs @("--remote-transport", "tcp", "--remote-tcp", "127.0.0.1:$plain", "--remote-token", $token)
                }
                Add-BenchmarkResult -Target $target -Case "ssh-native-direct" -PoolSize $poolSize -Measure {
                    Measure-ProxyCase -Target $target -Case "ssh-native-direct" -CaseUrl $caseUrl -ProxyArgs @("--remote-transport", "ssh-native", "--ssh-session-pool-size", $poolSize.ToString())
                }

                if (-not $SkipDirect) {
                    Add-BenchmarkResult -Target $target -Case "spx-plain-direct" -PoolSize $poolSize -Measure {
                        Measure-ProxyCase -Target $target -Case "spx-plain-direct" -CaseUrl $caseUrl -ProxyArgs @("--remote-transport", "plain-tcp", "--remote-tcp", "${directHost}:$plain", "--remote-token", $token)
                    }
                    Add-BenchmarkResult -Target $target -Case "spx-tls-direct" -PoolSize $poolSize -Measure {
                        Measure-ProxyCase -Target $target -Case "spx-tls-direct" -CaseUrl $caseUrl -ProxyArgs @("--remote-transport", "tls-tcp", "--remote-tls", "${directHost}:$tls", "--remote-ca", $cert, "--remote-name", "ssh-proxy-bench", "--remote-token", $token)
                    }
                    Add-BenchmarkResult -Target $target -Case "spx-quic-direct" -PoolSize $poolSize -Measure {
                        Measure-ProxyCase -Target $target -Case "spx-quic-direct" -CaseUrl $caseUrl -ProxyArgs @("--remote-transport", "quic", "--remote-quic", "${directHost}:$quic", "--remote-ca", $cert, "--remote-name", "ssh-proxy-bench", "--remote-token", $token)
                    }
                    Add-BenchmarkResult -Target $target -Case "quic-native-direct" -PoolSize $poolSize -Measure {
                        Measure-ProxyCase -Target $target -Case "quic-native-direct" -CaseUrl $caseUrl -ProxyArgs @("--remote-transport", "quic-native", "--remote-quic", "${directHost}:$quic", "--remote-ca", $cert, "--remote-name", "ssh-proxy-bench")
                    }
                }
            }
        }
    }
}
finally {
    foreach ($plan in $remotePlans) {
        if (-not $KeepRemote) {
            try {
                $cleanupResults.Add((Invoke-RemoteCleanup -Plan $plan))
            }
            catch {
                Write-Warning "cleanup failed for $($plan.target): $($_.Exception.Message)"
                $failure = Get-BenchmarkFailureClassification -Message $_.Exception.Message -ForcedKind "cleanup_failed" -ForcedStage "cleanup"
                $cleanupResults.Add([pscustomobject]@{
                    target = $plan.target
                    remote_dir = $plan.remote_dir
                    command = $null
                    exit_code = $null
                    removed = $false
                    kept = $false
                    paths = $plan.paths
                    pid_files = $plan.pid_files
                    output = $null
                    primary_failure_stage = $failure.primary_failure_stage
                    human_summary = $failure.human_summary
                    error_kind = $failure.error_kind
                    error = $_.Exception.Message
                })
            }
        } else {
            $cleanupResults.Add([pscustomobject]@{
                target = $plan.target
                remote_dir = $plan.remote_dir
                command = $null
                exit_code = 0
                removed = $false
                kept = $true
                paths = $plan.paths
                pid_files = $plan.pid_files
                output = $null
                primary_failure_stage = $null
                human_summary = $null
                error_kind = $null
                error = $null
            })
        }
    }
}

$csv = Join-Path $localWork "results.csv"
$json = Join-Path $localWork "results.json"
$resultRows = @($results.ToArray())
$stabilityCases = @($resultRows | Where-Object { $_.run_level -eq "stability" } | Select-Object target, case, selected_protocol, transport_pool_size, stability_duration_secs, stability_small_requests, stability_lost_requests, stability_max_latency_ms, stability_degraded_intervals, stability_reconnect_count, stability_large_failures, stability_remote_restart_injected, error_kind, human_summary)
$sshVsSpxOverhead = Set-SshVsSpxOverheadComparisons -Rows $resultRows
$opensshControlMasterComparisons = Get-OpenSshControlMasterComparisons -Rows $resultRows
$baselineQualitySummary = Get-BaselineQualitySummary -Rows $resultRows
$tlsOverPlainRatios = Get-TlsOverPlainRatios -Rows $resultRows
$resultRows | Export-Csv -NoTypeInformation -Path $csv
$poolRecommendations = Get-PoolRecommendations -Rows $resultRows -ExpectedConcurrency $Concurrency
$poolRecommendationsByWorkload = Get-PoolRecommendationsByWorkload -Rows $resultRows -ExpectedConcurrency $Concurrency
$sshNativePoolDiagnostics = Get-SshNativePoolDiagnostics -Rows $resultRows
$nativeQuicComparisons = Get-NativeQuicComparisons -Rows $resultRows
$quicProfileSummaries = Get-QuicProfileSummaries -Rows $resultRows
$quicProfileRecommendations = Get-QuicProfileRecommendations -Rows $resultRows
$targetTopologies = Get-TargetTopologies -Rows $resultRows
$preflightNegativeCacheEntries = @($script:PreflightNegativeCache.Keys | Sort-Object | ForEach-Object {
    $parts = [string]$_ -split '\|', 3
    [pscustomobject]@{
        target = if ($parts.Count -ge 1) { $parts[0] } else { $null }
        protocol = if ($parts.Count -ge 2) { $parts[1] } else { $null }
        endpoint = if ($parts.Count -ge 3) { $parts[2] } else { $null }
        reason = Get-PlanCaptureValue -PlanCapture $script:PreflightNegativeCache[$_] -Name "plan_topology_failure"
        candidate_failures = Get-PlanCaptureValue -PlanCapture $script:PreflightNegativeCache[$_] -Name "plan_candidate_failures"
    }
})
$gitCommit = try { (git -C $root rev-parse HEAD).Trim() } catch { $null }
$summary = [pscustomobject]@{
    stamp = $stamp
    targets = $Targets
    url = $Url
    scenario = if ([string]::IsNullOrWhiteSpace($scenarioName)) { $null } else { $scenarioName }
    run_level = $runLevelName
    run_level_preset = $runLevelPreset
    command = $invocationCommand
    git_commit = $gitCommit
    artifacts = [pscustomobject]@{
        local_work = $localWork
        results_csv = $csv
        results_json = $json
    }
    binary_versions = $binaryVersions.ToArray()
    local_platform = $script:LocalPlatform
    openssh_capability = $script:OpenSshCapability
    baseline_quality = $baselineQualitySummary
    remote_platforms = $remotePlatforms.ToArray()
    use_remote_payload = [bool]$UseRemotePayload
    payload_mib = $PayloadMiB
    concurrency = $Concurrency
    transport_pool_sizes = $TransportPoolSizes
    quic_max_bidi_streams = $QuicMaxBidiStreams
    quic_stream_receive_window = $QuicStreamReceiveWindow
    quic_receive_window = $QuicReceiveWindow
    quic_keep_alive_interval_secs = $QuicKeepAliveIntervalSecs
    quic_idle_timeout_secs = $QuicIdleTimeoutSecs
    quic_debug_log = [bool]$QuicDebugLog
    quic_profile = [bool]$QuicProfile
    quic_profile_sweep_plan = if ($QuicProfile) { Get-QuicProfileSweepPlan } else { $null }
    quic_udp_gso_source = "unknown: quinn 0.11 endpoint API does not expose effective UDP GSO capability"
    stability_duration_secs = $StabilityDurationSecs
    stability_small_interval_secs = $StabilitySmallIntervalSecs
    stability_inject_remote_daemon_restart = [bool]$StabilityInjectRemoteDaemonRestart
    spx_profile = [bool]$SpxProfile
    include_ssh_control_master = [bool]$IncludeSshControlMaster
    ssh_control_master_baseline = $SshControlMasterBaseline
    ssh_control_persist_secs = $SshControlPersistSecs
    skip_direct = [bool]$SkipDirect
    respect_preflight_skip = [bool]$RespectPreflightSkip
    preflight_negative_cache_entries = $preflightNegativeCacheEntries
    keep_remote = [bool]$KeepRemote
    cleanup_stamp = if ([string]::IsNullOrWhiteSpace($CleanupStamp)) { $null } else { $CleanupStamp }
    ssh_connect_timeout = $SshConnectTimeout
    remote_command_timeout = $RemoteCommandTimeout
    timeout_budgets = [pscustomobject]@{
        ssh_connect_secs = $SshConnectTimeout
        remote_command_secs = $RemoteCommandTimeout
        warmup_curl_secs = 60
        large_curl_secs = 300
        concurrent_curl_secs = 120
        remote_payload_probe_secs = 5
        proxy_startup_secs = 20
        daemon_startup_probe_attempts = 40
        daemon_startup_probe_interval_ms = 500
        quic_keep_alive_interval_secs = $QuicKeepAliveIntervalSecs
        quic_idle_timeout_secs = $QuicIdleTimeoutSecs
    }
    resume = [pscustomobject]@{
        enabled = [bool]$script:ResumeEnabled
        source = $script:ResumeSource
        reusable_rows = $script:ResumeRowsByKey.Count
        skipped_runtime_rows = $script:ResumeEvidence.Count
        evidence = $script:ResumeEvidence.ToArray()
    }
    local_work = $localWork
    remote_plans = $remotePlans
    cleanup = $cleanupResults
    stability_cases = $stabilityCases
    target_topologies = $targetTopologies
    pool_recommendations = $poolRecommendations
    pool_recommendations_by_workload = $poolRecommendationsByWorkload
    ssh_native_pool_diagnostics = $sshNativePoolDiagnostics
    ssh_vs_spx_overhead = $sshVsSpxOverhead
    openssh_controlmaster_comparisons = $opensshControlMasterComparisons
    tls_over_plain_ratio = $tlsOverPlainRatios
    native_quic_comparisons = $nativeQuicComparisons
    quic_profile_summaries = $quicProfileSummaries
    quic_profile_recommendations = $quicProfileRecommendations
    results = $resultRows
}
$summary | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -Path $json

$targetTopologies |
    Select-Object target, topology_class, recommended_default_transport, reachable_direct_protocols, failed_direct_protocols, ssh_jump_chain |
    Format-Table -AutoSize
$sshNativePoolDiagnostics |
    Select-Object target, implicit_default_max_pool, tested_pools, best_large_pool_size, best_concurrent_pool_size, high_pool_policy |
    Format-Table -AutoSize
$sshVsSpxOverhead |
    Select-Object target, pool, spx_large_mibps, native_large_mibps, spx_concurrent_mibps, native_concurrent_mibps, ssh_direct_channel_open_latency_ms, spx_tcp_relay_duration_ms |
    Format-Table -AutoSize
$opensshControlMasterComparisons |
    Select-Object target, case, openssh_control_master_mode, comparison_quality, baseline_quality, controlmaster_quality, baseline_large_mibps, controlmaster_large_mibps, baseline_concurrent_mibps, controlmaster_concurrent_mibps, error |
    Format-Table -AutoSize
$tlsOverPlainRatios |
    Select-Object target, pool, tls_over_plain_large_ratio, tls_over_plain_concurrent_ratio, plain_write_frames_per_batch, tls_write_frames_per_batch, plain_vectored_writes_per_frame, tls_vectored_writes_per_frame, plain_relay_remote_to_client_mibps, tls_relay_remote_to_client_mibps |
    Format-Table -AutoSize
$nativeQuicComparisons |
    Select-Object target, comparison_scope, pool, framed_large_mibps, native_large_mibps, large_speedup, framed_concurrent_mibps, native_concurrent_mibps, concurrent_speedup, bottleneck_suspects |
    Format-Table -AutoSize
$quicProfileSummaries |
    Select-Object target, case, pool, large_mibps, quic_receive_window, quic_stream_receive_window, process_cpu_delta_ms, bottleneck_suspects |
    Format-Table -AutoSize
$quicProfileRecommendations |
    Select-Object large_flow, high_concurrency, mixed |
    Format-List
$resultRows | Format-Table -AutoSize
Write-Host "results_csv=$csv"
Write-Host "results_json=$json"
