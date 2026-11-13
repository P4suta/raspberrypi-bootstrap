---
date: 2026-11-13
---

# Collect metrics and deliver notifications on the Pi

## Status

Accepted.

## Context

Mac sleep interrupts the existing Beszel hub.
Graphs cover the Mac, Linux server, Windows laptop, and Pi with selected iPhone notifications.

## Decision

Run Beszel and ntfy under the existing unprivileged account with digest-pinned Podman Quadlets.
Preserve the hub database and identity before redirecting agents.
Expose authenticated management endpoints through Tailscale Serve on separate ports.
Use the configured Pi login.
Exclude the NAS.
Keep notification content at home and use the [documented iOS relay](https://docs.ntfy.sh/config/#ios-instant-notifications) for wake-up messages.

## Alternatives

Keeping the hub on the Mac preserves its dependency on laptop uptime.
An external hub changes where metrics live.
An external heartbeat detects whole-home outages but exceeds the agreed scope.

## Consequences

The Pi becomes a shared failure boundary for collection and notifications.
The iPhone needs Tailscale access to retrieve notification content.
Reconsider the shared failure boundary if whole-home outage detection becomes required.

## Confirmation

The isolated ARM64 rehearsal verified authentication, notification recovery, reboot persistence, and backup restoration before deployment.
Native checks verified restored history, four connected agents, service startup after reboot, and delivery from the hub to ntfy.
The phone subscription receives notifications.
Separate locked-screen testing is outside the requested scope.
