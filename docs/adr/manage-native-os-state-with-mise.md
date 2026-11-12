---
date: 2026-11-07
---

# Manage native system state with mise

## Status

Proposed.

## Context

A general-purpose Pi needs native hardware updates and private administration.
Machine setup should declare desired state and prepare software on the build host.

## Decision

Keep Raspberry Pi OS ARM64 and its native kernel and firmware.
Use [mise bootstrap](https://mise.jdx.dev/bootstrap.html) for packages, system files, service state, and pinned tools.
Use rpi-image-gen for initial media.
Include the user-session runtime and native root-partition growth tools.
Use wired networking and turn off unused onboard radios.
Keep device identities in private configuration and same-device backups.

## Alternatives

[NixOS](https://github.com/NixOS/nixos-hardware/blob/master/raspberry-pi/README.md) offers system generations but changes the board's boot and update path.
Independent provisioning scripts duplicate package and service decisions.

## Consequences

Operating system updates use native packages.
Keep image hashes, package manifests, and a verified card backup.
Firmware changes require a reboot.
Image construction can't establish board behavior.

## Confirmation

Validate declarations and build inputs.
Reject missing startup dependencies.
Observe healthy services, private access, radio state, storage capacity, and convergence after a real boot.
