# Security Policy

`ssh_proxy` manages SSH sessions, daemon control sockets, route tokens, and TLS
material. Please treat security issues as sensitive until they are fixed.

## Supported Versions

The project is pre-1.0. Security fixes are made on the default branch first.
Tagged releases should be upgraded to the latest available version after a fix
is published.

## Reporting a Vulnerability

Use GitHub private vulnerability reporting if it is enabled for the repository.
If it is not available, open a minimal public issue that says you have a private
security report and avoid posting exploit details, private keys, tokens, or
internal hostnames.

Useful reports include:

- affected command or daemon mode;
- local and remote operating systems;
- selected transport (`ssh-native`, `spx-ssh-direct`, `tls-tcp`, `quic`, or
  `quic-native`);
- whether the issue requires local user access, SSH access, or network access;
- redacted route/status output.

Do not include daemon tokens, peer tokens, certificate private keys, SSH private
keys, or full proxy credentials.
