---
date: 2026-11-13
---

# Serialize and resume reader credential changes

## Status

Proposed.

## Context

Preparation and password rotation write the same private runtime files.
An interrupted rotation can leave files from different credential generations.

## Decision

Hold one operating-system file lock across preparation, retirement, rotation, and rotation verification.
Stage complete previous and replacement credentials before publishing a pending rotation directory.
Replace individual files through synchronized sibling files and atomic rename.
Block preparation while rotation is pending and support idempotent resumption from the staged replacement.
Remove the previous generation after the service rejects its password and verifies the replacement permissions.
The [Rust file-lock contract](https://doc.rust-lang.org/1.99.1/std/fs/struct.File.html#method.try_lock) releases the lock when the process closes the file.

## Alternatives

Atomic file replacement alone preserves each file but permits mixed generations and overlapping preparation.
A database transaction would add a storage dependency while deployment still consumes separate environment files.

## Consequences

Pending rotations keep a private recovery directory and require resumption before preparation.
Reconsider the local lock if two machines must administer one credential store concurrently.

## Confirmation

Fault injection preserves saved files after interrupted writes.
Local tests verify exclusion, pending preparation refusal, and idempotent recovery.
The ARM64 rehearsal verified replacement authentication, previous-password denial, publisher permissions, and restart persistence.
