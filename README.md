# Moonlight Web

Stream games from a [Sunshine](https://github.com/LizardByte/Sunshine) host in your browser.

Browsers can't speak the GameStream protocol (raw UDP, RTSP, ENet, client-cert HTTPS), so a
small Rust **bridge** acts as the Moonlight client (via `moonlight-common-c`) and re-serves the
stream to the browser over **WebRTC**. The **web UI** is Svelte + Vite.

```
Browser ──REST + WS signaling──▶ bridge ──GameStream──▶ Sunshine
   ◀────── WebRTC video/audio ──┘  ◀── input over DataChannels
```

## Quick start

```sh
./run.sh               # build UI + bridge (release) and serve on http://localhost:8080
```

`run.sh` enters the Nix dev shell by itself if the toolchain isn't on `PATH`, and fetches
the submodule and npm dependencies when they're missing.

| Command          | What it does                                               |
| ---------------- | ---------------------------------------------------------- |
| `./run.sh build` | Build web UI + bridge (release)                            |
| `./run.sh run`   | Build, then serve on `$MLW_BIND` (default)                 |
| `./run.sh dev`   | Debug bridge on :8080 + Vite hot-reload UI on :5173        |
| `./run.sh check` | `svelte-check`, `cargo test`, `cargo clippy -D warnings`   |

## Development environment

The toolchain (Rust, Node 22, cmake, OpenSSL, libclang) is defined once in `shell.nix`;
`flake.nix` reuses it. Any of these work:

```sh
nix develop            # flake (pinned nixpkgs via flake.lock)
nix-shell              # classic, uses your <nixpkgs> channel
direnv allow           # auto-enter via .envrc
```

### Configuration (env vars)

| Variable          | Default        | Purpose                                   |
| ----------------- | -------------- | ----------------------------------------- |
| `MLW_BIND`        | `0.0.0.0:8080` | HTTP listen address                       |
| `MLW_DATA_DIR`    | `data`         | Hosts, client certificate, pairing data   |
| `MLW_WEB_DIR`     | `web/dist`     | Built UI                                  |
| `MLW_ICE_SERVERS` | _(none)_       | Comma-separated STUN/TURN URLs (non-LAN)  |
| `MLW_WEBRTC_UDP_PORT` | `0` (random) | Fixed UDP port for WebRTC media (firewalls) |

## Self-hosting with Docker

Every push to `main` builds `ghcr.io/deathedit/moonlight-web` via GitHub Actions
(`.github/workflows/docker.yml`; tags `latest`, `sha-…`, and `X.Y.Z` for `vX.Y.Z` tags).

```sh
docker compose up -d        # uses docker-compose.yml
```

- **Host networking is required** (`network_mode: host`): WebRTC has to advertise the server's
  real LAN address, and mDNS discovery needs LAN multicast.
- **Firewall on the server:** allow TCP 8080 (web UI/API), UDP 8080 (WebRTC media, set by
  `MLW_WEBRTC_UDP_PORT`) and UDP 5353 (mDNS discovery).
- **Pairings live in the `/data` volume.** Keep it; losing it means pairing again.
- **No authentication.** Anyone who can reach the bridge can stream from and control your PC.
  Keep it on your LAN or behind a reverse proxy with authentication; don't expose it to the internet.
- **Use HTTPS when opening it from another machine.** Browsers only allow some features
  (gamepads, keyboard lock in fullscreen) in secure contexts — `https://` or `localhost`.
  Put it behind a reverse proxy with TLS (Caddy, Traefik, nginx); WebSocket support isn't needed.

Build the image yourself with `docker build -t moonlight-web .` from a `--recursive` clone.

## Host discovery

PCs running Sunshine are found automatically via mDNS (`_nvstream._tcp`). The machine running
the bridge must accept mDNS replies (UDP 5353). On NixOS, for example:

```nix
networking.firewall.allowedUDPPorts = [ 5353 ];
```

Hosts can always be added by IP address instead.

## Layout

- `bridge/` — axum server, GameStream HTTP API, pairing, WebRTC sessions, input
- `moonlight-common-sys/` — FFI bindings; builds the `moonlight-common-c` submodule with cmake
- `web/` — Svelte UI; design tokens in `web/src/lib/theme.css`
