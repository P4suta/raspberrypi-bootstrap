---
date: 2026-11-15
status: proposed
---

# Route private subnets through Tailscale

## Context

Printers and IoT devices need access from tailnet clients without installing Tailscale on each device.
Client networks can overlap the destination network.

## Decision

Use an optional subnet-router profile with machine-local private prefixes.
Advertise native IPv4 routes and [4via6 routes](https://tailscale.com/docs/features/subnet-routers/4via6-subnets) with an explicit site identity.
Use native route approval and existing access grants.
Follow the [subnet-router guidance](https://tailscale.com/docs/features/subnet-routers) for persistent forwarding, source address translation, and default-denied forwarding outside Tailscale.
Keep login state and existing private web services.

## Alternatives

Per-service proxies require separate configuration for every device protocol.
Layer-two bridging would forward broadcasts but exceeds Tailscale's subnet-routing contract.

## Consequences

4via6 names select the intended site even on an overlapping local network.
Multicast discovery needs execution on the device network or an explicit device address.
Reconsider the gateway if the local network moves to separate security zones.

## Confirmation

Keep the record proposed until isolated forwarding checks, native route approval, and a routed device connection pass.
