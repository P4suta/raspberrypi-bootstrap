# Private subnet routing

The optional routing profile makes private IPv4 networks reachable through a Tailscale gateway.
Configuration comes from explicit network prefixes and a site ID.
The generator rejects public networks, default routes, host bits, duplicate prefixes, and empty input.
It creates native IPv4 advertisements and their [4via6 equivalents](https://tailscale.com/docs/features/subnet-routers/4via6-subnets).
The gateway requires Tailscale 1.102.3 or later because earlier releases have a published 4via6 security defect.

Generate a machine-local environment file with an unused output path.

```sh
mise x -- just routing-config 7 10.1.1.0/24 /private/path/subnet-router.env
```

Keep a native `tailscale debug prefs` export, a `tailscale serve status --json` export, both forwarding values, and both firewall forwarding policies before applying.
Preserve any existing files at the three installation destinations.
The routing unit manages the complete advertisement list.
Use a gateway dedicated to the selected prefixes and keep unrelated subnet or exit-node advertisements on their existing gateway.
The apply command installs only the routing environment, forwarding configuration, and systemd unit.
It preserves the existing login and web service configuration.

```sh
mise x -- just check
mise x -- just routing-apply /private/path/subnet-router.env
systemctl status tailscale-subnet-router.service --no-pager
tailscale debug prefs
```

The unit sets default-denied forwarding before enabling packet forwarding.
Its forwarding file stays outside `sysctl.d` so early boot can't enable forwarding before the firewall policy.
Tailscale's existing forwarding chains admit checked tailnet traffic.
The gateway uses source address translation and stateful filtering.
The unit enables the same configuration after boot.
Applying a changed configuration restarts the routing unit so the new prefixes take effect immediately.
The [official subnet-router guide](https://tailscale.com/docs/features/subnet-routers) separates route advertisement, route approval, and access grants.
Approve only the generated routes in the Tailscale administration console and verify the intended source identity's access.

With MagicDNS enabled, the example device `10.1.1.16` has the site-specific name `10-1-1-16-via-7`.
The name selects the gateway even when the client uses an overlapping local network.
Protocols that verify a certificate hostname still need the device's authenticated hostname and an explicit connection address.
Subnet routing transports unicast traffic.
It doesn't relay multicast discovery, so run discovery on the device network or use a known address.

## Recovery

Withdraw the managed advertisements before disabling the unit.
Restore the recorded advertisement list instead of discarding unrelated routes.
Restore the recorded forwarding values, firewall policies, and files.
Remove an added sysctl file only if the file still contains the installed bytes.
Reload systemd after restoring the unit files.
Verify administrator access and the existing web services after recovery.
