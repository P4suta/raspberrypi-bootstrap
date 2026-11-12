---
date: 2026-11-07
---

# Use Tailscale with existing OpenSSH

## Status

Superseded by [Tailscale SSH administration](use-tailscale-ssh-for-private-administration.md).

## Context

Private remote administration must survive reboot.
The operator's account uses SSH public key authentication.

## Decision

Use [cloud-init](https://tailscale.com/docs/install/with-cloud-init) to install Tailscale from its signed native package repository on first boot.
Keep the source signing key scoped to that repository.
Use [mise services](https://mise.jdx.dev/bootstrap/services.html#system-services) for later package and service convergence.
Use existing OpenSSH authentication over the tailnet.
Keep enrollment, device identity, and SSH destinations outside the repository.

## Alternatives

Tailscale SSH changes the existing authentication boundary.
A containerized network client adds a second service boundary to initial host administration.
Existing OpenSSH already provides authenticated access to the account.

## Consequences

Initial installation runs before a remote provisioning session.
Enrollment waits for operator authentication.
Raspberry Pi boot and network behavior require device observations rather than a configuration schema check.

## Confirmation

Run `just check` for platform configurations and the boot seed.
Verify an authenticated remote command through the tailnet, service persistence after reboot, and an unchanged second provisioning run.
