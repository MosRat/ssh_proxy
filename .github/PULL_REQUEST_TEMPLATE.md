# Pull Request Checklist

## Transport Changes

For changes that affect route selection, SSH fallback, SPX/TCP/TLS, QUIC, benchmark scripts, or
transport status output, include evidence for each item below.

- [ ] Route plan: attach or summarize `route --explain` output for the affected topology, including
  `selected_transport`, `transport_selection_reason`, preflight failures, fallback reason, and the
  expected next action.
- [ ] Route status: attach or summarize live `route status` / node daemon status fields that prove
  the selected protocol, active pool/session state, byte counters, open failures, and degraded or
  recovery state are observable.
- [ ] Benchmark row: include the relevant smoke or full benchmark result row with selected protocol,
  pool/session size, workload shape, throughput or latency, control health, and failure summary.
- [ ] Cleanup evidence: record what temporary listeners, remote daemons, payload servers, pid files,
  logs, and retained benchmark artifacts were removed or intentionally kept.
- [ ] Documentation: update `README.md`, `docs/operations.md`, `docs/architecture.md`, or public
  performance notes
  when the user-visible behavior, default, status field, or recommendation changes.

Attach only sanitized benchmark or diagnostic evidence. Do not include private host aliases, local
paths, credentials, internal notes, or raw local logs.
