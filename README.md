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

## Layout

- `bridge/` — axum server, GameStream HTTP API, pairing, WebRTC sessions, input
- `moonlight-common-sys/` — FFI bindings; builds the `moonlight-common-c` submodule with cmake
- `web/` — Svelte UI; design tokens in `web/src/lib/theme.css`
