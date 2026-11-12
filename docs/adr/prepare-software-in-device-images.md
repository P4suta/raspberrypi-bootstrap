---
date: 2026-11-07
---

# Prepare software in device images

## Status

Proposed.

## Context

Software preparation should use the Mac's computing resources.
First-boot downloads depend on the device's network and clock.
[QEMU's Raspberry Pi models](https://www.qemu.org/docs/master/system/arm/raspi.html) don't include Raspberry Pi 5.

## Decision

Use [rpi-image-gen](https://github.com/raspberrypi/rpi-image-gen/tree/v2.9.0) on an ARM64 Debian build host to prepare packages and pinned tools.
Generate an SD disk image and software inventory.
Keep account settings and recovery public keys in private configuration.
Generate SSH host keys and Tailscale identity on the device.
Use first boot for network readiness and owner authentication.

## Alternatives

Installing every dependency on the Pi increases device setup work.
Board emulation can't establish compatibility with an unsupported device.

## Consequences

The build host needs Linux namespaces and complete subordinate user and group mappings.
Build filesystems need native Linux storage.
The administrator account uses passwordless sudo behind owner-controlled authentication.
An image build doesn't verify firmware, peripherals, or actual device boot.
Keep the stock-image boot seed available for recovery.

## Confirmation

Reject invalid configuration before filesystem creation.
Build an image and check package versions, native tool execution, service definitions, and absent enrolled identities.
Observe boot and private access on real hardware before adopting a generated image as the default.
