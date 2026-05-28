# Service Provider Evaluation

The peer lifecycle uses an internal provider contract first. `service-manager`
remains a design reference, not a production dependency for v0.3.

## Provider Contract

Each provider must expose stable manager naming, install/start/stop/status
rendering, health classification, elevation requirements, and rollback hints.
Providers also feed `PeerLifecycleSpec` and `PeerLifecycleReport`, so local
daemon reports and remote peer reports use the same role, scope, provider, phase,
blocker, and recovery fields.
Remote install commands are executed through the lifecycle workflow rather than
direct SSH status calls, which keeps provider failures, lifecycle JSON, and
doctor/status redaction on one path.
The same contract covers:

- Windows system daemon through `windows-service` and the elevated install worker.
- Windows user remote peer through scheduled tasks.
- Linux local/remote services through systemd user/system units.
- macOS local/remote services through launchd LaunchAgents/LaunchDaemons.
- Linux remote fallback through the managed nohup supervisor.

## Why Not Switch Immediately

`service-manager` can simplify cross-platform service calls, but this repo
already has production-specific Windows behavior: hidden allowlisted worker,
structured JSONL logs, UAC cancellation mapping, versioned ProgramData binaries,
and rollback-safe service replacement. Those semantics must not be rewritten in
the same change as the lifecycle extraction.

## Adoption Gate

`service-manager` can become a production dependency only after provider
contract tests prove it preserves Windows SCM behavior, systemd/launchd command
shape, remote peer install reports, rollback/error mapping, and current fast
gate timing.
