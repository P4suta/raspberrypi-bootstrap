---
date: 2026-11-07
---

# Run private apps with rootless Podman

## Status

Proposed.
[Retiring the unused note services](retire-unused-note-services.md) supersedes the note-service deployment.

## Context

A general-purpose Pi needs isolated apps, persistent notes, and private access.
Family sites need independent configuration and data.
NAS access remains outside the app runtime configuration.

## Decision

Use rootless [Podman Quadlet](https://docs.podman.io/en/v5.4.2/markdown/podman-systemd.unit.5.html) under an unprivileged service account.
Pin app images by digest and keep their state in named volumes.
Bind app ports to loopback and expose selected HTTPS endpoints through Tailscale Serve.
Keep credentials and destinations in private configuration.
Use local storage for Memos SQLite and SilverBullet data.
Keep NAS backup as separate pending work.

## Alternatives

Coolify adds a web interface and Docker-based deployment management.
Its adoption depends on the preferred site-management workflow.
Kubernetes adds a cluster control plane to a single-host deployment.
Native app installation couples app dependencies to the host.

## Consequences

The systemd user manager handles startup and restart.
Container replacement preserves data volumes.
Local persistence can't recover data after SD-card failure.
Account, storage, and networking behavior need native integration checks.

## Confirmation

Verify native image execution, private access, authentication, note writes, container replacement, and reboot persistence.
