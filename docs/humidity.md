# Camera storage humidity

The Pi listens for one SwitchBot W3400010 Indoor/Outdoor Meter over Bluetooth.
The rootless service records measurements under the `apps` account and shares the private ntfy instance with Beszel.
The exposure, display ranges, and notification policy live in this repository.

## Identify the meter

Apply the wired host and app profiles and finish the private ntfy setup.
Remove the old Bluetooth masks when upgrading an existing host.
Schedule a reboot to apply the boot firmware settings.
[Host setup](setup.md) · [Private monitoring](monitoring.md)

Preview and install the user unit and the BlueZ bus policy from the bootstrap checkout.

```sh
mise x -- just humidity-plan
mise x -- just humidity-apply
bluetoothctl show
```

Build the ARM64 receiver from its own checkout and load the image into the rootless `apps` image store.
The unit expects the local tag `localhost/humidity-monitor:installed`.
Pull the insulation tab and place the meter near the Pi.
Run discovery as `apps` and record the address printed alongside the expected measurements.

```sh
podman run --rm --userns=keep-id --user "$(id -u):$(id -g)" \
  --volume /run/dbus/system_bus_socket:/run/dbus/system_bus_socket:ro \
  --env DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket \
  localhost/humidity-monitor:installed discover hci0
```

## Start recording

On the administration machine, use the private directory that contains the monitoring credentials and `runtime/ntfy.env`.
Register the address reported by discovery.

```sh
mise x -- just humidity-prepare /private/path AA:BB:CC:DD:EE:FF
```

The command keeps existing readers and creates a publisher with write access to `home-status`.
On the Pi, install the generated `runtime/ntfy.env` in `~apps/.config/monitoring/ntfy.env` and `runtime/monitor.toml` in `~apps/.config/humidity-monitor/monitor.toml`.
Set both files to mode 0600 with user and group `apps`.
Pass the generated `runtime/humidity-ntfy-token` to the rootless secret store using a private input file.
Run the following commands as `apps` for the first installation.

```sh
podman secret create humidity-ntfy-token /private/path/runtime/humidity-ntfy-token
systemctl --user daemon-reload
systemctl --user restart monitoring-ntfy.service
systemctl --user start humidity-monitor.service
systemctl --user is-active humidity-monitor.service
```

The web service listens on the host at `127.0.0.1:8092`.
Add a route to the private host configuration with an unused HTTPS port and target `http://127.0.0.1:8092`.
Preview the route with `just host-plan`, apply it with `just host-apply`, and verify it with `just host-check`.
Open the private HTTPS address on a phone connected to the tailnet.

## Rehearse the deployment

Complete the ARM64 monitoring rehearsal to prepare the isolated guest and its test subscriber.
Export the receiver image tagged `localhost/humidity-monitor:installed` to a local archive.
Run the humidity rehearsal from the bootstrap checkout.

```sh
podman save --output /private/path/receiver.tar localhost/humidity-monitor:installed
mise x -- just humidity-preflight /private/path/receiver.tar
```

The Rust fixture emits simulated advertisements on the guest system bus.
It checks that the service runs as the app account and that the account owns the SQLite database.
It exercises high humidity and reception loss with shorter confirmation times and checks delivery through ntfy.
It restarts the guest and verifies a new reception alongside the original SQLite history.

## Check storage conditions

The chart marks 40–50% relative humidity as a storage reference and flags readings below 35% on screen.
[Tamron lens care guide](https://www.tamron.com/jp/consumer/sp/impression/detail/article-how-to-maintain-camera-lenses.html)

Humidity greater than 55% for 30 minutes triggers a notification.
A missing minute interrupts the confirmation period.
Five minutes at or below 55% trigger a recovery notification.
Ten minutes without a valid reading trigger a reception loss notification.
The first valid reading announces that reception has resumed.

Place the meter near the lens and away from the desiccant.
Close the drawer for an hour and check the history for gaps of ten minutes or more.
During commissioning, verify that recording resumes after a Pi reboot and that the phone receives notifications for both a loss and its recovery.
