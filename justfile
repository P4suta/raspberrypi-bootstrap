set shell := ["bash", "-euc"]

mise_schema := "https://raw.githubusercontent.com/jdx/mise/aa26090fc10184bf61997e5784f72c365c8c8ece/schema/mise.json"
cloud_init_schema := "https://raw.githubusercontent.com/canonical/cloud-init/e682bef5be7f18d6c803fbc3ad35e277fdf03e39/cloudinit/config/schemas/schema-cloud-config-v1.json"

default:
    @just --list

check:
    MISE_ENV=checks mise exec -- just check-source

[private]
check-source:
    mise exec taplo@0.10.0 -- taplo fmt --check mise.toml mise.armhf.toml mise.arm64.toml mise.apps.toml mise.sites.toml mise.monitoring.toml mise.checks.toml verification/preflight/mise.toml repository.toml Cargo.toml xtask/Cargo.toml deny.toml .gitleaks.toml
    mise exec taplo@0.10.0 -- taplo lint --schema '{{ mise_schema }}' mise.toml mise.armhf.toml mise.arm64.toml mise.apps.toml mise.sites.toml mise.monitoring.toml mise.checks.toml verification/preflight/mise.toml
    mise exec taplo@0.10.0 -- taplo lint --no-schema repository.toml deny.toml .gitleaks.toml
    mise exec uv@0.13.6 -- uvx --from check-jsonschema==0.39.0 check-jsonschema --schemafile 'https://coderabbit.ai/integrations/schema.v2.json' .coderabbit.yaml
    just check-cloud-init boot/tailscale-trixie-arm64.yaml
    just check-image-source
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    cargo run --locked -- check
    actionlint
    cargo run --locked -- check-publication
    cargo run --locked -- check-dependencies

check-cloud-init config:
    mise exec uv@0.13.6 -- uvx --from check-jsonschema==0.39.0 check-jsonschema --schemafile '{{ cloud_init_schema }}' --default-filetype yaml '{{ config }}'

check-image-source:
    mise exec yq@4.54.3 -- yq -r '.write_files[0].content | trim' boot/tailscale-trixie-arm64.yaml | cmp image/mise-tailscale.sources -

build-image generator config:
    install -d {{ quote(justfile_directory() / ".local/image-work") }}
    {{ quote(generator) }} build -S {{ quote(justfile_directory() / "image") }} -B {{ quote(justfile_directory() / ".local/image-work") }} -c {{ quote(config) }}

fmt:
    mise exec taplo@0.10.0 -- taplo fmt mise.toml mise.armhf.toml mise.arm64.toml mise.apps.toml mise.sites.toml mise.monitoring.toml mise.checks.toml verification/preflight/mise.toml repository.toml Cargo.toml xtask/Cargo.toml deny.toml .gitleaks.toml
    cargo fmt --all

plan:
    mise bootstrap --dry-run --only packages,files,services,tools

apply:
    mise bootstrap --yes --update --only packages,files,services,tools

verify-host:
    mise bootstrap status --missing
    systemctl is-system-running
    systemctl is-active "user@$(id -u).service"
    systemctl is-active systemd-growfs-root.service

apps-plan:
    MISE_ENV=arm64,apps mise bootstrap --dry-run --only accounts,packages,files,services

apps-apply:
    MISE_ENV=arm64,apps mise bootstrap --yes --only accounts,packages,files,services
    sudo loginctl enable-linger apps
    sudo systemctl start "user@$(id -u apps).service"
    sudo --user apps env XDG_RUNTIME_DIR="/run/user/$(id -u apps)" systemctl --user daemon-reload

apps-check:
    sudo systemctl is-active "user@$(id -u apps).service"
    sudo --user apps env --chdir=/var/lib/apps XDG_RUNTIME_DIR="/run/user/$(id -u apps)" podman unshare /usr/bin/true

sites-plan:
    MISE_ENV=arm64,sites mise bootstrap --dry-run --only files

sites-apply:
    MISE_ENV=arm64,sites mise bootstrap --yes --only files
    sudo --user apps env XDG_RUNTIME_DIR="/run/user/$(id -u apps)" systemctl --user daemon-reload
    sudo --user apps env XDG_RUNTIME_DIR="/run/user/$(id -u apps)" systemctl --user restart site-study-player.service

sites-check:
    sudo --user apps env XDG_RUNTIME_DIR="/run/user/$(id -u apps)" systemd-analyze --user verify --man=no --generators=true site-study-player.service
    sudo --user apps env XDG_RUNTIME_DIR="/run/user/$(id -u apps)" systemctl --user is-active site-study-player.service
    sudo --user apps env --chdir=/var/lib/apps XDG_RUNTIME_DIR="/run/user/$(id -u apps)" podman exec site-study-player caddy validate --config /etc/caddy/Caddyfile

sites-serve:
    sudo tailscale serve --bg --https=443 http://127.0.0.1:8080

sites-http url:
    mise exec hurl@8.0.1 -- hurl --test --no-output --connect-timeout 5 --max-time 30 --variable site_url={{ quote(url) }} verification/sites-health.hurl

sites-entrypoints origin:
    mise exec hurl@8.0.1 -- hurl --test --no-output --connect-timeout 5 --max-time 30 --variable site_origin={{ quote(origin) }} verification/sites-entrypoints.hurl

monitoring-plan:
    MISE_ENV=arm64,apps,monitoring mise bootstrap --dry-run --only accounts,packages,files,services

monitoring-preflight:
    cargo run --locked -- preflight

monitoring-preflight-init mise agent:
    cargo run --locked -- preflight-init {{ quote(mise) }} {{ quote(agent) }}

monitoring-apply:
    MISE_ENV=arm64,apps,monitoring mise bootstrap --yes --only accounts,packages,files,services

monitoring-check:
    sudo --user apps env --chdir=/var/lib/apps XDG_RUNTIME_DIR="/run/user/$(id -u apps)" systemd-analyze --user verify --man=no --generators=true monitoring-beszel.service monitoring-ntfy.service
    sudo --user apps env --chdir=/var/lib/apps XDG_RUNTIME_DIR="/run/user/$(id -u apps)" systemctl --user is-active monitoring-beszel.service monitoring-ntfy.service
    systemctl is-active monitoring-agent.service
