# Excalidraw for private replies

The workstation renders diagrams with the [official Excalidraw CLI](https://github.com/excalidraw/cli).
The Pi serves the editor and the files for each reply.
The official Excalidraw image runs the `nginx` web server.
Prefer [Pingap](https://pingap.io/) when selecting a reverse proxy or static file server for a new self-hosted service.
Opening a saved link loads the original drawing.
Readers can save their browser edits as a separate file.

## Install the service

Use the unprivileged app account with user lingering enabled.
Copy `apps/quadlet/excalidraw.container` to `~/.config/containers/systemd/excalidraw.container` and `apps/excalidraw/nginx.conf` to `~/.config/excalidraw-replies/nginx.conf`.
Create the archive under the app account.

```sh
install -d -m 700 ~/.config/excalidraw-replies ~/.local/share/excalidraw-replies
install -d -m 700 ~/.local/share/excalidraw-replies/incoming
install -d -m 755 ~/.local/share/excalidraw-replies/diagrams
```

Set `~/.config/excalidraw-replies/reader.map` to one quoted Tailscale login followed by `1;`, such as `"reader@example.invalid" 1;`.
Use mode 0644 inside the private configuration directory so the container user can read the file.
Missing and unrecognized identities receive HTTP 403.
Tailscale Serve [replaces identity headers supplied by clients](https://tailscale.com/docs/features/tailscale-serve#identity-headers).

```sh
systemctl --user daemon-reload
systemctl --user start excalidraw.service
systemctl --user is-active excalidraw.service
```

Check the current Serve configuration and the tailnet grants before assigning a private HTTPS port.
Route the selected port to `http://127.0.0.1:8097` with `tailscale serve --bg --https=PORT http://127.0.0.1:8097` from an account allowed to manage Serve.
Keep the hostname, port selection, and login in machine-local configuration.

## Publish from the workstation

Install the workstation command and provide Node.js 22 or later, npm, a browser supported by the Excalidraw CLI, and OpenSSH.

```sh
mise x -- cargo install --locked --path xtask --bin publish-excalidraw
```

Create `~/.config/excalidraw-replies/config.json` with the app account's SSH host entry and the private HTTPS origin.
Point the SSH host entry at the Pi's tailnet name or address.
Use an empty path on the origin.

```json
{"ssh_host":"diagram-host","origin":"https://drawing.example.invalid:15443"}
```

Run `publish-excalidraw drawing.mmd` to convert Mermaid, or pass an existing `.excalidraw` file to preserve its elements and styling.
The command renders a preview image, uploads both files outside the served directory, and publishes them with one rename.
It reads both files back over HTTPS before printing their locations.
Attach the image at `preview_path` in a private reply, or give its local path in a text terminal.
Share `png_url` or `editor_url` privately when the reader asks for a link.
The editor and diagram links work from an authorized device even after the container restarts.

## Attach a figure to an issue or blog post

Export the scene on the workstation as a standalone `.png` file.

```sh
npx --yes @excalidraw/cli@1.1.2 render drawing.excalidraw --out figure.png
```

Check the labels, filename, alt text, and file metadata for private network details.
Attach the image to the GitHub issue or add it to the blog's media library or assets.
Use the destination's image reference so readers can see the figure without access to the Pi.

## Verify access

Run the HTTP fixture on a test listener with a synthetic reader.
The fixture checks permitted and denied requests, immutable filenames, and directory listing behavior.
On the Pi, open two editor links in a browser, restart only `excalidraw.service`, and reopen both links.
Keep the archive and its staging directory on the same filesystem.
