# Architecture

The codebase has three boundaries.

## Configuration

`config.rs` discovers Compose files, loads interpolation variables, expands
Compose parameter expressions, and merges multiple files. The rendered YAML is
validated by `compose_spec` before it is converted into the internal model.

`model.rs` normalizes short and long Compose forms into one representation for
ports, mounts, networks, environment variables, commands, and build settings.

## Planning

`plan.rs` selects services using profiles and explicit CLI arguments, includes
dependencies when required, detects cycles, and produces a stable topological
order. Shutdown commands reverse that order.

## Execution

`backend.rs` is the WSLC boundary. It uses `wslc-rs` for SDK/component probes
and invokes the official `wslc.exe` CLI for persistent control-plane actions.
All containers and resources receive standard `com.docker.compose.*` labels.

The hybrid design is necessary because WSLC SDK 2.9.3 exposes creation and
opaque handle operations but no API for reopening a session or container from a
new process. Keeping execution behind one module allows migration to direct SDK
calls when those APIs exist.

