# Home monitoring

Beszel collects host metrics and ntfy delivers selected notifications from the Pi.
The containers use the existing rootless `apps` account and run with `User=10001:10001`.
The native Pi agent runs as `apps` with optional privileged collectors off.
The loopback listeners are ports 8090 and 8081, with authenticated HTTPS access through Tailscale Serve on ports 8443 and 10443.
Machine identities and credentials belong in machine-local configuration.

## Local deployment rehearsal

Finish local verification before changing a running host.
Download mise 2026.10.24 for Linux ARM64 from its [release](https://github.com/jdx/mise/releases/tag/v2026.10.24) and the Beszel agent 0.21.0 for Linux ARM64 from its [release](https://github.com/henrygd/beszel/releases/tag/v0.21.0).
Verify publisher checksums before extracting the agent.
The initializer also verifies that both executable hashes match the reviewed pins.

```sh
mise x -- just check
mise x -- just monitoring-preflight-init /path/to/mise-linux-arm64 /path/to/beszel-agent-linux-arm64
mise x -- just monitoring-preflight
```

The initializer creates or validates the task's isolated OrbStack machine with ARM64 Debian 13, two CPUs, 4 GiB of memory, and a 12 GiB disk limit.
The machine mounts the disposable workspace.
Home sharing and SSH agent forwarding stay off.
Network isolation blocks access to host and private-network addresses.
Public package and container registries remain available.
The rehearsal stages an explicit set of public sources and uses synthetic credentials, synthetic monitoring identities, and disposable volumes.
Production databases, credentials, and Tailscale state stay outside the rehearsal.

The rehearsal applies the actual account, package, Quadlet, and agent declarations.
It rejects missing nftables and aardvark-dns, then verifies that the declared package profile repairs the dependency failure.
It checks native collection, non-root process identities, internal hostname resolution, anonymous denial, separate publication and subscription permissions, and delivery from Beszel to ntfy.
Disposable disconnect and disk alerts use shortened thresholds to exercise both triggering and recovery.
The production policy uses a disk usage threshold of 90 percent for ten minutes on all four machines.
Only the continuously running Linux server has a five-minute disconnect threshold.
The rehearsal also verifies the production policy.
The hub client reads every page of record lists and rejects repeated identities.
A disposable registration test crosses the 100-record page boundary before cleanup.
NAS retirement must remove its registration and notification rules while preserving every other system, notification rule, and registration identity.
Stopped-volume export and import must preserve the account, hub key, system identity, and existing statistics.
Restarting the isolated machine must preserve automatic service startup, metrics, and the ntfy message cache.
Local notifications have no external upstream.

## Private runtime configuration

Create a mode-0600 JSON input outside tracked sources with `credentials_file`, `public_key_file`, `source_url`, `hub_url`, `ntfy_url`, `pi_host`, `required_systems`, and `offline_systems`.
Use the [synthetic input example](../verification/examples/monitoring.json) as the starting point for a private copy.
The credential file uses `Email:` and `Password:` fields from the existing installation.
The public key file contains the existing hub key.
Use four distinct system names for the Mac, Linux server, Windows laptop, and Pi.
Select only the Linux server for disconnect notifications.
The helper accepts HTTPS tailnet origins and the existing HTTP tailnet origin during migration.

```sh
mise x -- cargo run --locked -- prepare /private/path/client.json
```

Preparation registers the Pi on the source hub and writes mode-0600 runtime files next to the private input.
Keep `secrets.json` and the `runtime` directory private.
The runtime directory contains hub and ntfy environment files, Pi agent credentials, the public hub key, and iPhone connection details.
Transfer only each service's required runtime file to the destination account.

## Native deployment and migration

Use the configured Pi SSH account and persistent remote-command client.
Preview `monitoring-plan` before applying `monitoring-apply` on the target.
Keep the `apps` user manager running with lingering enabled and reload its units after applying the profile.
Credential existence conditions keep unprovisioned services stopped.

Stop the source hub and wait for its process to exit before taking a complete backup of `beszel_data` with its private key and both SQLite databases.
Verify the backup and preserve the source installation as the rollback checkpoint.
Restore the complete data directory into the stopped rootless `monitoring-beszel` volume and normalize ownership for user ID 10001.
Start the target hub and ntfy before changing agent endpoints.
Expose each loopback service through its selected Tailscale Serve HTTPS port, preserving existing routes.
Verify TLS from another tailnet device.
Redirect the existing agents without replacing their tokens or the hub key.
Then start the native Pi agent.

```sh
mise x -- cargo run --locked -- configure /private/path/client.json
mise x -- cargo run --locked -- status /private/path/client.json
mise x -- cargo run --locked -- notifications /private/path/client.json
mise x -- cargo run --locked -- verify /private/path/client.json
```

The configuration helper preserves unrelated user settings, directs alerts to the internal ntfy service, and installs the selected disk and disconnect rules.
The verification helper requires all four agents to connect.
It checks the five notification rules, authentication, fresh message identity, retrieval, and delivery from the hub.
Use `cached /private/path/client.json MESSAGE_ID` after restarting the target to verify that an earlier notification survives.
Perform target-specific network, hardware, and reboot checks only after the local rehearsal passes.
If migration fails, stop the target hub before restarting the preserved source hub and restore the prior agent endpoints.
Avoid simultaneous writers to either hub data directory.

## NAS retirement

The deployment excludes NAS monitoring.
The manual package uninstall is complete.
The monitoring work preserved NAS network, account, and storage settings.
Remove the NAS from the private target lists before retiring its hub registration.

```sh
mise x -- cargo run --locked -- retire-nas /private/path/client.json
```

Retirement deletes the NAS alerts, registration identity, saved agent token, and installation fields without contacting the NAS.
Preparation no longer registers a NAS agent.

## Phone setup

The phone subscription receives notifications.
Separate locked-screen delivery testing is outside the requested scope.
Add the private server, topic, and read-only credentials from `iphone.txt` to the native ntfy app, with Tailscale connected.
The production server uses the [documented iOS wake-up relay](https://docs.ntfy.sh/config/#ios-instant-notifications), while the app retrieves notification content from the private server.
Keep Tailscale connected while receiving notifications.
An offline Pi or home connection also interrupts the collector and notification service.
Whole-home outage detection needs a separate future decision.

## Reader password rotation

The password helper accepts a new 64-character printable ASCII password from standard input.
On macOS, copy the password and run the rotation command without exposing the value in shell arguments or terminal output.

```sh
pbpaste | mise x -- cargo run --locked -- reader-password /private/path/client.json
```

The helper updates the private credential file, `ntfy.env`, and `iphone.txt` while preserving the publisher identity and its configuration.
It keeps the previous files in a private `reader-password-backup` directory until verification succeeds.
Install the updated `ntfy.env` for the unprivileged service account and restart the ntfy service.
Run the verification command after the service becomes ready.

```sh
mise x -- cargo run --locked -- reader-password-check /private/path/client.json
```

Verification checks that the previous password fails and the replacement authenticates with read-only access.
Publisher and hub delivery checks must pass.
Successful verification removes the temporary previous credentials.
Update the iPhone app with the replacement password.
