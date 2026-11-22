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

## Personal service portal

Add a `label` to each route you want to open from one bookmark in your private host configuration.
The portal lists named routes in configuration order and builds each link from `tailnet_name` and the HTTPS `port`.
Keep proxy targets, credentials, and the generated page in private storage.
The [example configuration](../verification/examples/host.json) includes three named services and an unlisted route.

Preview the example on the administration machine or pass the path to your private configuration.

```sh
mise x -- just portal-preview
mise x -- just portal-preview /private/path/host.json
```

Open `.local/service-portal.html` in a browser.
The example uses synthetic addresses.
After installing the updated maintenance helper, use the normal `host-plan`, `host-apply`, and `host-check` commands.
The first named route enables the generated file at `/var/lib/raspberrypi-maintenance/service-portal.html`.
Host updates replace it atomically and include it in the host backup.
Removing the last label clears the links from an existing portal.

Choose an unused HTTPS port outside the family site and service ports.
Check `tailscale serve status` and the [effective access rules](https://tailscale.com/docs/features/access-control/grants) before enabling the portal.
Grant your account access to the chosen port and verify that the policy denies access to other accounts.
Each destination still requires its own access.
For example, after reserving port `15443`, share the single HTML file with [Tailscale Serve](https://tailscale.com/docs/reference/tailscale-cli/serve#use-https-and-http-servers).

```sh
sudo tailscale serve --bg --https=15443 /var/lib/raspberrypi-maintenance/service-portal.html
```

Bookmark the HTTPS address printed by Serve and open a named service from an authorized device.
To retire the portal, run `sudo tailscale serve --https=15443 off` with the port you selected.

## Restore a host update

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
