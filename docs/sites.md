# Private family sites

Build static apps on the Mac and serve their portable output from a rootless container on the Pi.
The optional site profile uses the app account created by the app profile.
The study player's source and pinned toolchain remain in its own checkout.

## Prepare the build

Run the study project's maintained checks and production build.

```sh
mise run check
```

The deployed root is `/var/lib/apps/sites/study-player`, owned by the app account.
The study project's manual deployment controller manages `releases`, `current`, and the activation journal below that root.
The container mounts `current` read-only.
The controller sets readable file modes within the mounted release while the parent app directories stay private.
For migration from an unmanaged folder, seed a bundle of its exact files before applying the managed mount.
The study project's `docs/deployment.md` describes seeding and deployment.
Keep the host settings in machine-local configuration.

## Apply the service

Copy the site profile, site configuration, Quadlet definition, and project commands to the Pi.
Trust the reviewed profile and apply it after deploying the files.

```sh
MISE_ENV=arm64,sites mise trust
MISE_ENV=arm64 mise exec just@1.59.0 -- just sites-plan
MISE_ENV=arm64 mise exec just@1.59.0 -- just sites-apply
MISE_ENV=arm64 mise exec just@1.59.0 -- just sites-check
```

The apply command restarts the site to resolve the selected release mount.
Caddy uses a pinned multi-architecture image and container user `10001`.
The root filesystem and site mount reject writes.
The container's data and config directories use disposable temporary mounts.
The native Podman runtime rejects the `uid` tmpfs option, so those mounts use mode `1777` within the container.
The app keeps viewing progress in browser storage.
The server has no progress database.

## Access and verify

The listener binds to loopback on port `8080`.
Check existing Tailscale Serve routes before assigning the root path on port `443`.
Run `just sites-serve` to publish the site through the shared HTTPS entry point.
Keep the chosen hostname in machine-local inventory.
Each viewing device needs an authorized Tailscale connection.
Direct access over the local network should fail.

The study player uses `/study/`.
The root URL and `/study` redirect to `/study/` with HTTP status 308.
Unrelated service paths return 404.
Use the shared port 443 frontend for the study URL.

Keep port `443` for family web services and route future apps by path.
For an app that supports a path prefix, Tailscale Serve can proxy `/app` to another loopback listener with `tailscale serve --bg --https=443 --set-path=/app http://127.0.0.1:8081`.
The family access rule then stays unchanged.
Keep management interfaces outside the shared frontend.
The [Tailscale container guide](https://tailscale.com/docs/solutions/host-private-server-docker) documents path routing and persistent background operation.

```sh
mise exec just@1.59.0 -- just sites-entrypoints HTTPS_ORIGIN
mise exec just@1.59.0 -- just sites-http HTTPS_ORIGIN/study
```

The HTTP check reads the page, JavaScript, style sheet, and icon, and rejects a request for an environment file.
Use a real browser to check rendering and the embedded video provider.
The source project's deterministic browser tests use a controlled provider and don't establish live YouTube playback.
Check native startup and HTTP responses after a normal Pi reboot.

## Update and recover

Run the study project's `just deploy HOST_CONFIG` from a clean committed checkout on the Mac.
The command checks and builds the app, transfers a bounded bundle, and activates it under the app account.
It compares served bytes with the build and restores the preceding release if startup or health verification fails.
Another explicit invocation recovers an interrupted activation before applying its candidate.
Successful updates remove their uploads and keep the active and preceding releases.
Keep the host configuration path in machine-local inventory.
Bootstrap owns the rootless runtime.
The study project owns app activation.
