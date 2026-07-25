# wslc-compose

Docker Compose compatible orchestration for Microsoft WSL Containers on
Windows, built on [`wslc-rs`](https://github.com/Ivanbeethoven/wslc-rs) and the
official `wslc.exe` control plane.

```powershell
wslc-compose config
wslc-compose up -d
wslc-compose ps
wslc-compose logs web
wslc-compose exec web sh
wslc-compose down --volumes
```

`wslc-compose` accepts the familiar Compose file names and command layout, so a
typical project can use `wslc-compose` in place of `docker compose` without
running Docker Desktop.

## Status

This is an early `0.1.x` implementation targeting the preview WSLC runtime.
Compose files are validated with the Rust
[`compose_spec`](https://crates.io/crates/compose_spec) crate before any WSLC
resources are changed. Linux container paths are validated by this project's
normalizer because host-native path checks are incorrect for `/path` on Windows.

Implemented commands:

- `config`, `pull`, `build`, `create`, `up`, `down`
- `start`, `stop`, `restart`, `kill`, `rm`
- `ps`, `logs`, `exec`, `run`, `version`

Implemented Compose behavior includes configuration discovery, multiple `-f`
files, `.env`, parameter expansion, profiles, dependency ordering, images,
builds, commands, entrypoints, environment variables, ports, bind mounts, named
volumes, tmpfs, project networks, network aliases, labels, resource limits, and
stop settings.

See [compatibility.md](docs/compatibility.md) for the exact support matrix and
known WSLC gaps.

## Requirements

- Windows with the preview WSL Containers runtime installed
- `wslc.exe` on `PATH`
- `wslcsdk.dll` discoverable by `wslc-rs` for direct SDK validation (optional
  when the persistent `wslc.exe` backend is available)
- Rust 1.85 or newer when building from source

Check the host before using a project:

```powershell
wslc version
wslc-compose version
```

The WSLC SDK setup is documented in the
[`wslc-rs` installation guide](https://github.com/Ivanbeethoven/wslc-rs/blob/master/docs/sdk-installation.md).

## Install

```powershell
git clone https://github.com/Ivanbeethoven/wslc-compose.git
cd wslc-compose
cargo install --path .
```

## Compose Example

```yaml
name: demo

services:
  web:
    image: docker.io/library/alpine:latest
    command: ["sh", "-c", "while true; do date; sleep 5; done"]
    environment:
      APP_ENV: ${APP_ENV:-development}
    ports:
      - "8080:80"
    volumes:
      - app-data:/var/lib/app

  worker:
    image: docker.io/library/alpine:latest
    command: ["sh", "-c", "while true; do echo working; sleep 10; done"]
    depends_on:
      - web

volumes:
  app-data:
```

Run it with:

```powershell
wslc-compose up -d
wslc-compose ps
wslc-compose logs worker
wslc-compose down --volumes
```

If Docker Hub requires a mirror, configure a host only, without `https://`:

```powershell
$env:WSLC_REGISTRY_MIRROR = "<your-registry>"
```

No registry endpoint is hard-coded in this repository.

SDK-backed `exec` requests may run for a long time during builds or benchmarks.
The daemon response timeout defaults to one hour. Override it in seconds, or
set it to `0` to wait indefinitely:

```powershell
$env:WSLC_COMPOSE_SDK_TIMEOUT_SECS = "0"
```

The bundled example also accepts `WSLC_COMPOSE_IMAGE` when you want to validate
the lifecycle with an image from another public registry.

A build-only example is available under `examples/build`:

```powershell
wslc-compose -f examples/build/compose.yaml build
wslc-compose -f examples/build/compose.yaml up -d --no-build --pull never
wslc-compose -f examples/build/compose.yaml down
```

## Architecture

The WSLC SDK currently creates opaque session and container handles but does not
provide APIs to reopen them in a later CLI process. `wslc-compose` therefore
uses a hybrid backend:

- `wslc-rs` verifies SDK and host component availability.
- `wslc.exe` manages persistent images, networks, volumes, containers, logs,
  and exec sessions across separate command invocations.

The execution backend is isolated from configuration parsing so it can move to
direct SDK calls when reopen/list APIs become available. See
[architecture.md](docs/architecture.md).

## Development

```powershell
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

The test suite does not require a running WSLC service. Real container smoke
tests are performed separately on Windows hosts with WSLC installed.

## License

Licensed under either the Apache License, Version 2.0 or the MIT license.
