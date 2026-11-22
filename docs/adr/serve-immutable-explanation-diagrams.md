---
date: 2026-11-20
---

# Serve immutable explanation diagrams

## Status

Accepted.

## Context

A reply needs a readable preview and a link that opens the same editable drawing later.
The workstation can [render Excalidraw files locally](https://github.com/excalidraw/cli).

## Decision

Render each reply on the workstation and upload its scene and preview together with OpenSSH's `sftp` command.
Give each pair a fresh directory and publish it with one rename.
Serve the upstream editor and the read-only archive from a rootless container.
Restrict the loopback service to the configured [Tailscale Serve identity](https://tailscale.com/docs/features/tailscale-serve#identity-headers).

## Alternatives

A shared live canvas lets clients edit one drawing together.
A later edit can change the meaning of an earlier reply.

## Consequences

The files survive replacement of the container.
Readers can save their own edits as files.
Reconsider the archive if readers need to edit the same stored drawing together.

## Confirmation

Open two distinct editor links, compare both previews with their originals, and reopen them after a container restart.
Verify that an interrupted transfer keeps the files private and that the service rejects a different identity.
