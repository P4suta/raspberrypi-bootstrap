# Rootless app runtime

The app profile installs Podman and prepares the unprivileged app account.
The account has a POSIX login shell and no administrator group membership or sudo rules.
User lingering keeps its systemd manager available after logout and at boot.
Use the administrator account for `apps-plan` and `apps-apply`.
Use the app account for daily operations after installing the [maintenance helper](operations.md).

```sh
MISE_ENV=arm64 mise exec just@1.59.0 -- just apps-plan
MISE_ENV=arm64 mise exec just@1.59.0 -- just apps-apply
```

```sh
raspberrypi-maintenance apps-check
raspberrypi-maintenance apps-restart agent
```

App binaries stay in pinned container images.
Give each persistent app its own data volume and backup contract.
Keep credentials and destination details outside source control.
Prepare static builds on the Mac and deploy them through the [site profile](sites.md).

The app profile no longer deploys SilverBullet or Memos.
The app profile declares their unit and local export files absent.
Their container and volume removal needs an explicit native operation after stopping the services.
Existing off-device archives remain recovery checkpoints.
The NAS access inputs stay in the purpose-specific Doppler configuration recorded in machine-local inventory.
Automatic NAS backup remains unconfigured.
