# Headless development setup

Prepare software and validate configuration in an ARM64 Linux guest on the Mac, then run apps on the Raspberry Pi.
The repository keeps operating system provisioning separate from app development.
Prepare app builds on the Mac when the app provides an ARM64 build command.
Run apps and device-specific checks on the Pi.

## Prepare software before first boot

Use [rpi-image-gen v2.9.0](https://github.com/raspberrypi/rpi-image-gen/tree/v2.9.0), commit `d2d5705bcb37284c4b08c0ce43c039b1f1004e25`, with Debian 13 ARM64.
Follow the generator's [host requirements](https://github.com/raspberrypi/rpi-image-gen/tree/v2.9.0#dependencies) and install its dependencies.
Keep the generator and build checkout on the guest's native Linux filesystem.
Mac shared folders can hold source transfers and completed images.
Include `/usr/sbin` and `/sbin` in the builder's `PATH`.
For a rootless builder, configure [subordinate user and group ranges](https://docs.podman.io/en/latest/markdown/podman.1.html#rootless-mode) and verify both mappings with `podman unshare`.
Installing `uidmap` alone doesn't set up those ranges.

The [image configuration](../image/config/arm64-private-access.yaml) uses Debian Trixie and Raspberry Pi packages.
It installs Tailscale, native development packages, mise, Rust, and just before first boot.
The build checks the recovery public key, mise checksum, enrollment service definition, and user-session dependency.
The image includes native root-partition growth support and firmware settings that turn off onboard Wi-Fi and Bluetooth.
The administrator account uses passwordless sudo and SSH public key authentication.
Keep the username, hostname, and recovery public key path in a private configuration outside version control.
Replace `USER`, `HOST`, and the public key path in the example.

```yaml
include:
  file: arm64-private-access.yaml
device:
  user1: USER
  hostname: HOST
access:
  recovery_key: /path/to/recovery.pub
```

Run the project command from the build checkout in the Linux guest.
Pass the generator executable and private configuration as arguments.

```sh
MISE_ENV=arm64 mise exec just@1.59.0 -- just build-image /path/to/rpi-image-gen /path/to/private.yaml
```

The SD image appears at `.local/image-work/image-arm64-private-access/arm64-private-access.img`.
The generator also produces the package manifest and software bill of materials.
Write the generated image with Raspberry Pi Imager's custom-image option.
The private configuration already supplies the account and recovery key.
First boot generates the SSH host identity and starts Tailscale enrollment after network and time synchronization.
Complete the normal browser sign-in for the device.
An image build doesn't establish actual Raspberry Pi boot or peripheral behavior.

## Prepare a stock image

Use [Raspberry Pi Imager](https://www.raspberrypi.com/documentation/computers/getting-started.html#headless-remote-setup) to select the device, operating system, and SD card.
Choose 64-bit Raspberry Pi OS Lite for a general-purpose Raspberry Pi 5 installation.
Writing an image replaces the selected card's contents, so use an empty card or preserve its data first.
Keep `Exclude system drives` selected.
Configure the username and hostname, enable SSH, and choose public key authentication with the Mac's SSH public key.
The private key stays in the Mac's configured SSH agent.

For 64-bit Trixie, merge the [Tailscale boot seed](../boot/tailscale-trixie-arm64.yaml) into the private cloud-init user data before writing the medium.
Keep the account, hostname, and recovery public key in the private configuration.
The seed installs Tailscale and starts Tailscale SSH enrollment on first boot.
It enables the [systemd time synchronization barrier](https://www.freedesktop.org/software/systemd/man/257/systemd-time-wait-sync.service.html) before final-stage package installation.
The operator completes the normal browser sign-in.
Mark the wired interface as required in cloud-init network configuration because package installation needs the network.
Use the same boot configuration for SD cards or [USB mass storage](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#usb-mass-storage-boot).
USB boot also needs a compatible device, adequate power, and USB in the observed boot order.

Insert the prepared card into the Pi and connect Ethernet to the router before connecting power.
Connect the Mac to the same local network.
Replace `USER` and `HOST` with the account and hostname configured in Imager.

```sh
ssh USER@HOST.local true
```

If hostname discovery fails, find the Pi in the router's connected devices and use its IP address as the SSH destination.
The [official remote access guide](https://www.raspberrypi.com/documentation/computers/remote-access.html) describes SSH enablement and address discovery.

## Fetch the public configuration

Connect from the Mac with the configured login.

```sh
ssh USER@HOST.local
```

In the Pi's shell, clone the public repository before running the remaining setup commands.
The clone includes every deployment profile, verification command, and workflow without private runtime files.

```sh
sudo apt-get update
sudo apt-get install --yes ca-certificates git
mkdir -p projects
git clone https://github.com/P4suta/raspberrypi-bootstrap.git projects/raspberrypi-bootstrap
cd projects/raspberrypi-bootstrap
```

## Select the native platform

Run the package architecture query on the Pi.

```sh
dpkg --print-architecture
```

[dpkg reports the architecture of the packages it installs](https://manpages.debian.org/bookworm/dpkg/dpkg.1.en.html#QUERY_COMMANDS).
Use the `armhf` profile and the `armv7` mise executable when the result is `armhf`.
Use the `arm64` profile and the `arm64` mise executable when the result is `arm64`.
An `aarch64` kernel can run an `armhf` userland, so select the toolchain from the package architecture.
Use a fresh 64-bit OS image for Raspberry Pi 5 unless a specific dependency needs a 32-bit installation.
The remaining examples use `arm64`.

## Install the tool manager

Use an existing mise installation if it meets the version declared in `mise.toml`.
For a new installation, follow the [signed installer procedure](https://mise.jdx.dev/installing-mise.html#verify-the-install-script) with the pinned version and selected architecture.
Run the installer download commands from the repository root on the Pi.

```sh
sudo apt-get update
sudo apt-get install --yes ca-certificates curl gnupg
mkdir -p .local
gpg --keyserver hkps://keys.openpgp.org --recv-keys 24853EC9F655CE80B48E6C3A8B81C9D17413A06D
curl -fsSL -o .local/install.sh.sig https://mise.jdx.dev/install.sh.sig
gpg --output .local/install.sh --decrypt .local/install.sh.sig
```

Continue only after GPG reports a valid signature with fingerprint `24853EC9F655CE80B48E6C3A8B81C9D17413A06D`.
Set `MISE_INSTALL_ARCH=armv7` instead for an intentional `armhf` installation.

```sh
MISE_VERSION=v2026.10.24 MISE_INSTALL_ARCH=arm64 sh .local/install.sh
export PATH="$HOME/.local/bin:$PATH"
mise --version
```

## Preview and apply

Review `mise.toml`, the selected platform profile, and the justfile before trusting the configuration.
The platform profile declares Rust's native host.
The generated image already masks unused radio services.
For a stock image, create the native masks before previewing the shared service declarations.

```sh
sudo systemctl mask --now iwd.service bluetooth.service hciuart.service
```

```sh
MISE_ENV=arm64 mise trust
MISE_ENV=arm64 mise install just@1.59.0
MISE_ENV=arm64 mise exec just@1.59.0 -- just plan
MISE_ENV=arm64 mise exec just@1.59.0 -- just apply
```

The apply command manages native packages, system files, services, and pinned tools through [mise bootstrap](https://mise.jdx.dev/bootstrap.html).
It enables SSH and Tailscale and keeps unused radio services masked.
Native packages follow the device's operating system repositories, while Rust and just have exact version pins.
Each platform profile declares the [official Tailscale package repository](https://pkgs.tailscale.com/stable/) and its signing key fingerprint.
Run the preview again after applying to inspect the resulting state.

The generated image includes `radio-policy.txt` through the firmware configuration's native fragment mechanism.
A stock image also needs the firmware include after applying the configuration.
Images built before the radio policy need the same include.
Append the global section and include directive shown below.
Preserve the vendor's existing firmware configuration.

```ini
[all]
include radio-policy.txt
```

The file belongs at `/boot/firmware/config.txt`.
The managed radio policy uses mode `0755` to match the boot partition's mount permissions.
Reboot to apply the firmware settings.
The [native root-growth package](https://packages.debian.org/trixie/cloud-initramfs-growroot) expands the root partition during startup when free space follows it.
A managed dependency starts [systemd's native root filesystem growth service](https://manpages.debian.org/trixie/systemd/systemd-growfs-root.service.8.en.html) during startup.
Verify the partition layout and filesystem capacity on the actual device.

## Private remote access

The boot seed starts the [normal enrollment flow](https://tailscale.com/docs/install/linux) automatically.
Open the authentication URL on a trusted computer.

```sh
tailscale status
tailscale ip
```

Use [Tailscale SSH](https://tailscale.com/docs/features/tailscale-ssh) for normal private administration.
Keep local-network public key authentication for enrollment observation and recovery.
Keep destinations and host keys in machine-local configuration.
The bootstrap doesn't change tailnet policy, advertise routes, or publish services.

Verify `systemctl is-enabled ssh tailscaled` and `systemctl is-active ssh tailscaled` on the Pi.
For the stock-image cloud-init path, verify `systemctl is-enabled systemd-time-wait-sync.service` and inspect `systemd-analyze critical-chain cloud-final.service` for time synchronization before the final stage.
Open a new SSH connection through the tailnet and run `id -un`.
After a normal reboot, repeat the connection and service checks.
Run `just apply` again and confirm that `just plan` reports no changes.
Run the native host check from an authenticated login.

```sh
MISE_ENV=arm64 mise exec just@1.59.0 -- just verify-host
```

The check rejects configuration drift, failed system units, and an unavailable user service manager.
Observe wired connectivity and the absence of onboard radio interfaces after reboot.
Confirm that a local-network login without an authorized key fails.

## Validate configuration changes

Run the repository check on a development machine with mise installed.
The check command installs its pinned validation tools through the optional `checks` profile.

```sh
mise install
mise x -- just check
```

The check validates TOML formatting and the official mise schema pinned to the configured mise release's commit.
It also checks cloud-init input using the pinned schema and the shared Tailscale source used by both image paths.
The image generator rejects `verification/rejected-image-account.yaml` before filesystem construction because its sudo mode isn't valid.
Confirm boot, network access, and SSH authentication through a login and a command on the Pi.
Record the source revision or source hashes alongside the image checksum, package manifest, and observed boot result.
Configuration validation, image construction, real boot, and recovery establish different properties.
A readable backup doesn't establish that restoring and booting it works.
Keep restore and real power-loss exercises separate from normal reboot acceptance.

## Remote development

The optional [domyjob workflow](https://github.com/P4suta/domyjob/blob/main/skills/domyjob/SKILL.md) runs commands and sends local work to a configured SSH destination.
Keep SSH connection settings and device addresses in local configuration.
Run `domyjob doctor` for the selected destination before relying on remote jobs.
Pass the destination as a command argument instead of adding a private hostname to the repository.

The first domyjob connection builds its matching remote executable using the embedded source and tool configuration.
Install the embedded source's exact Rust version first and temporarily turn off automatic mise tool installation for that build to avoid installing unrelated development tools.
Restore the previous automatic installation policy after the build and verify a remote job before starting project work.
The Raspberry Pi bootstrap configuration itself declares only Rust and just as managed development tools.
