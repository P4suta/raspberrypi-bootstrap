# Pi maintenance

Daily app operations use the unprivileged `apps` account through Tailscale SSH.
The tailnet repository controls remote identities and destinations.
The bootstrap profile preserves existing local network access and administrator recovery.
OS maintenance still uses the administrator account.
SD image declarations prepare software and kernel policy.
Physical restore and power-loss recovery require separate hardware observations.

## Prepare reviewed sources

Run the source checks and isolated ARM64 rehearsal before deployment.

```sh
mise x -- just check
mise x -- just monitoring-preflight
mise x -- just source-stage /private/path/source-snapshot
```

The snapshot contains public source files, a standard `source.sha256` manifest, and its SHA-256 identity in `source.id`.
Staging rejects private paths and scans the copied bytes for credentials.
Verification rejects modified, missing, extra, and linked files.
Install the verified ARM64 helper as root-owned `/usr/local/bin/raspberrypi-maintenance` with mode 0755.
Keep the exact source snapshot root-owned and read-only under `/usr/local/share/raspberrypi-maintenance/source`.
Preserve the previous helper and snapshot before replacing them.
Verify the transferred helper checksum and source identity before execution.

```sh
raspberrypi-maintenance source-verify /usr/local/share/raspberrypi-maintenance/source
```

## Daily operations

Connect as `apps` through the machine-local Tailscale SSH host entry.
The account can inspect and restart its app services without sudo.

```sh
raspberrypi-maintenance apps-check
raspberrypi-maintenance apps-restart metrics
raspberrypi-maintenance apps-restart notifications
raspberrypi-maintenance apps-restart agent
raspberrypi-maintenance apps-restart telephone
```

Telephone restart calls the telephone manager's guarded stop before boot.
A rejected stop prevents boot and preserves an active call.
The helper accepts app names rather than arbitrary system units.
The app account can still change its own files and processes.
Use the administrator account for packages, kernel settings, host networking, and recovery.

## Host configuration

Create a private copy of the [synthetic host input](../verification/examples/host.json).
Record the target machine ID, canonical tailnet name, existing Ethernet management addresses, and owned HTTPS routes.
The managed network drop-in uses `eth0` and a private /30 link.
Regular local network addressing remains unchanged.
Each proxy target must be loopback or the fixed management peer.
An explicitly listed alternate permits a verified console handoff without overwriting its route.

```sh
sudo raspberrypi-maintenance host-plan /private/path/host.json
sudo raspberrypi-maintenance host-apply /private/path/host.json /private/path/fresh-host-backup
sudo systemctl reboot
sudo raspberrypi-maintenance host-check /etc/raspberrypi-maintenance/host.json
```

The host update backs up managed files and the native Serve configuration before changing them.
Serve writes use the native LocalAPI concurrency tag and preserve unrelated routes.
The boot argument `cgroup_enable=memory` enables the kernel memory controller while preserving other boot arguments.
The image includes the same argument through the generator's native `for/linux-base/cmdline.d` interface.
After reboot, verify finite limits in the app service cgroups and observe fresh metrics.
The host check verifies machine identity, declared file bytes, HTTPS routes, the memory controller, and the actual source address used to reach the management peer.

```sh
sudo raspberrypi-maintenance host-restore /private/path/host-backup
sudo raspberrypi-maintenance host-recover
```

Restoration changes only managed files and routes changed by the recorded update.
A later change to a managed file or route prevents restoration of the conflicting value.
An interrupted update blocks another update until recovery completes.
If Tailscale is unavailable, the command restores the files first.
When Tailscale returns, repeat `host-restore` with the same backup for an explicit restoration.
Run `host-recover` to finish an interrupted update with a pending backup.
Use the administrator account and local network path to recover access.

The native [Tailscale client](https://github.com/tailscale/tailscale/blob/v1.104.5/client/local/serve.go) defines Serve concurrency behavior.
The [Raspberry Pi kernel discussion](https://github.com/raspberrypi/linux/issues/6980) explains memory-controller enablement.
The [systemd network target guidance](https://systemd.io/NETWORK_ONLINE/) explains the rootless runtime's boot dependency.
