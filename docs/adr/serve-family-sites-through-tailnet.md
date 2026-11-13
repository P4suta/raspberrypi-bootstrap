---
date: 2026-11-07
---

# Serve family sites through the tailnet

## Status

Proposed.

## Context

Family devices need a private study player.
The study project requires authenticated hosting.

## Decision

Build the portable site on the Mac with the app's pinned tools.
Serve the resulting files through a digest-pinned Caddy container under the existing app account.
Mount the site files read-only and publish the listener on loopback.
Expose the shared HTTPS frontend on port 443 through Tailscale Serve.
Route future family web services by path and keep management interfaces separate.
Manage family access through the tailnet policy repository.
Keep addresses, private builds, and deployment receipts outside source control.

## Alternatives

Direct publication on the local network needs its own authentication decision.
A development server adds build tooling to the Pi's runtime.
The static app needs no deployment control plane.

## Consequences

Each viewing device needs an authorized Tailscale connection.
The server holds no viewing history.
The unchanged app keeps progress in each browser.
The optional site profile requires the app account and a deployed build before startup.

## Confirmation

Run the app's checks, native service verification, HTTP checks, and browser checks.
Observe startup after a normal reboot.
Native service and network behavior require observed integration checks.
