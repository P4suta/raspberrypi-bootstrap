---
date: 2026-11-07
---

# Use a 64-bit operating system

## Status

Proposed.

## Context

[Raspberry Pi 5 has a 64-bit processor](https://www.raspberrypi.com/products/raspberry-pi-5/).
A general-purpose installation has no identified 32-bit dependency.
The package architecture determines native tools, even when the kernel supports 64-bit execution.

## Decision

Use 64-bit Raspberry Pi OS Lite and the `arm64` bootstrap profile for a fresh Raspberry Pi 5 installation.
Prepare and verify installation media and configuration on the Mac.
Use the Pi for installation, app execution, and checks that need the actual device.
Keep `armhf` only for a specific 32-bit dependency.
Use the [official image installation procedure](https://www.raspberrypi.com/documentation/computers/os.html#os-installation) for an architecture change.

## Alternatives

Keeping the supplied 32-bit OS preserves its current configuration but constrains native package selection.
An in-place architecture conversion would need its own recovery procedure.

## Consequences

Changing the active card needs physical access and preservation of wanted data.
Image verification on the Mac can't establish that the Pi boots or rejoins the tailnet.
Keep device identities and authentication outside the repository.

## Confirmation

Run `just check` and verify the official image checksum and signature before writing media.
Observe `dpkg --print-architecture` returning `arm64`, `getconf LONG_BIT` returning `64`, and authenticated SSH through Tailscale after a normal reboot.
