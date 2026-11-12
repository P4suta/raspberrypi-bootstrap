---
date: 2026-11-07
---

# Retire unused note services

## Status

Accepted.

## Context

SilverBullet and Memos are no longer part of the deployment.
The study player still uses the rootless container runtime.

## Decision

Remove the note services, containers, volumes, local export files, and web endpoints.
Keep the app account and container engine for the study player.
Keep existing off-device archives as recovery checkpoints.
Declare retired unit and export files absent so later bootstrap runs preserve retirement.

## Alternatives

Stopping the containers alone leaves their data and startup definitions in place.
Keeping the unwanted apps has no active consumer.

## Consequences

The Pi has no active note service or note database.
Restoration needs an archive and an explicit redeployment.
The study player and NAS credentials have separate lifecycles.

## Confirmation

Verify that retired services, volumes, files, and listeners are absent.
Verify that the study player starts and serves its production files.
