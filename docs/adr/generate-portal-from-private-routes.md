---
date: 2026-11-22
---

# Generate the personal portal from private routes

## Status

Proposed.

## Context

A person needs one bookmark for services with different private HTTPS ports.
The host configuration already identifies each destination, and the [family site](serve-family-sites-through-tailnet.md) has its own audience.

## Decision

Generate a static HTML file from the routes that have display names during the existing host update.
Use the tailnet hostname and the HTTPS port for each link.
Use [Tailscale Serve file hosting](https://tailscale.com/docs/reference/tailscale-cli/serve#use-https-and-http-servers) on a dedicated port after granting the intended account access.

## Alternatives

A bookmark file needs a second edit whenever a service moves.
A dashboard service could show live status, but opening a service by name needs no background process or credentials.

## Consequences

The host backup covers the generated file.
The tailnet policy controls access to the portal and each destination independently.
Revisit the static page if selecting a service requires live data.

## Confirmation

Check link selection, changes, removal, and restoration with synthetic routes.
Exercise named [HTML links](https://www.w3.org/WAI/WCAG22/Understanding/link-purpose-in-context.html) with a keyboard and at a [narrow viewport](https://www.w3.org/WAI/WCAG22/Understanding/reflow.html).
At deployment, check the dedicated port from permitted and excluded tailnet accounts.
