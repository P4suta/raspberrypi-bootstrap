---
date: 2026-11-07
---

# Use Tailscale SSH for private administration

## Status

Proposed.
Supersedes [existing OpenSSH administration](use-tailscale-with-existing-openssh.md).

## Context

Private administration should use the enrolled operator's identity.
Initial enrollment also needs a recovery path before private network access exists.

## Decision

Use cloud-init to install Tailscale and start enrollment with Tailscale SSH on first boot.
Enable systemd's time synchronization barrier before final-stage package installation.
Use the tailnet's existing network and SSH access rules without broadening permissions.
Keep local-network public key access for initial observation and recovery.
Prepare images and configuration on the Mac and observe hardware behavior on the Pi.

## Alternatives

OpenSSH over Tailscale requires client public key distribution.
Tailscale SSH alone leaves initial enrollment dependent on console access.
Neither should define the operator's normal setup procedure.

## Consequences

The operator completes normal Tailscale authentication.
[SSH check rules](https://tailscale.com/docs/features/tailscale-ssh#configure-tailscale-ssh-with-check-mode) can require later sign-in.
Keep credentials and machine identities outside source.

## Confirmation

Validate the boot seed using the pinned schema and a rejecting fixture.
Observe enrollment, an authorized account command, and access after reboot.
