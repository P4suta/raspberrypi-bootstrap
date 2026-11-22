---
date: 2026-11-14
---

# Operate app services without sudo

## Status

Proposed.

## Context

An administrator login controls services that run without root.
Private host changes need reproducible configuration.

## Decision

Use the app account through [Tailscale SSH](https://tailscale.com/docs/features/tailscale-ssh), without sudo.
Move the metrics agent into its user service manager.
Use the telephone manager's guarded stop before boot.
Keep OS setup in the image and mise declarations.
Preserve unrelated HTTPS routes and identify deployed sources with checksums.
Keep administrator recovery until a physical restore succeeds.
Preserve the existing local network access policy.

## Alternatives

A sudo helper retains a privileged runtime boundary.
Ansible duplicates mise configuration.
Removing maintenance access requires verified physical recovery.

## Consequences

App operators can change app data.
OS updates still require maintenance access.
The kernel must enable memory accounting.

## Confirmation

Require tests and the ARM64 rehearsal.
Verify denied sudo, route preservation, rollback, and source drift.
Observe private access and memory limits after reboot.
Reconsider if app data needs stronger operator isolation.
