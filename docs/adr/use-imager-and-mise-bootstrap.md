---
date: 2026-11-07
---

# Use separate boot and package configuration

## Status

Proposed.

## Context

A headless Raspberry Pi needs an account, network access, and SSH before remote provisioning.
[Raspberry Pi Imager supports configuring those settings before first boot](https://www.raspberrypi.com/documentation/computers/getting-started.html#headless-remote-setup).
Developers need to inspect and reapply the native package and tool configuration.

## Decision

Use Raspberry Pi Imager for the initial account, hostname, and SSH public key.
Use the [declarative mise bootstrap package configuration](https://mise.jdx.dev/bootstrap/packages/) for native development packages and pinned language tools.
Keep device identities and authentication material in operator-managed configuration outside the repository.
Expose separate validation, preview, and apply commands in the [justfile](../../justfile).

## Alternatives

A custom image adds image maintenance and distribution.
A custom first-boot script duplicates Imager's account and SSH integration.
Keep stock first-boot helpers as a recovery path for prepared cards.

## Consequences

Provisioning starts after a working SSH login and a mise installation.
Pin Rust and just and let the operating system manage native packages.
The configuration provides convergent package and tool state across reruns.
Check boot, authentication, and remote command execution on the device.

## Confirmation

Validate the configuration with `just check`.
Apply it twice on a Raspberry Pi and confirm unchanged package and tool state.
