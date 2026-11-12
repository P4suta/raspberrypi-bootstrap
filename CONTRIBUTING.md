# Contributing

The configuration targets wired Raspberry Pi OS with systemd and Debian packages.
Development checks run on Linux and macOS with Git and [mise](https://mise.jdx.dev/getting-started.html).
The monitoring rehearsal uses an isolated ARM64 Debian 13 OrbStack machine on macOS.

## Check a checkout

Review the tool declarations before trusting the checkout.
Install the project tools and run the authoritative check.

```sh
mise trust
mise install
mise x -- just check
```

The check installs the pinned validation tools from `mise.checks.toml`.
It validates schemas, formatting, Rust types, Clippy, unit tests, GitHub Actions, locked dependencies, and publication candidates.
Publication scanning copies the Git candidate file set to a bounded temporary directory.
Private local directories and deployment artifacts must never enter the candidate set.
The secret scanner must reject a synthetic credential leak before the check succeeds.
Dependency checks fetch current RustSec data into a task-owned directory, print its revision, and check the immutable snapshot offline.

Configuration checks preserve the running device.
Use the [device setup guide](docs/setup.md), [app runtime guide](docs/apps.md), and [monitoring guide](docs/monitoring.md) for deployment and native verification.
Hardware boot, real network reachability, and recovery require their documented native checks.

## Propose a change

Describe the problem and intended behavior in an issue before implementation.
Keep pull requests focused and use an English Conventional Commit title.
Explain the problem, resulting behavior, and actual verification under `Why`, `Changes`, and `Validation` headings.
Preserve existing private state and validate deployment behavior locally before changing a running service.
Contributions use the repository's `MIT OR Apache-2.0` license.

## Report a vulnerability

Use the repository's private vulnerability reporting form for a report containing security-sensitive details.
Keep credentials, private device addresses, runtime databases, and personal data out of public issues and pull requests.
