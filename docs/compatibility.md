# Docker Compose Compatibility

`wslc-compose` targets the current Compose Specification rather than a single
historical `docker-compose.yml` version. Every merged and interpolated file is
deserialized through `compose_spec` before execution.

## CLI Commands

| Command | Status | Notes |
| --- | --- | --- |
| `config` | Supported | YAML/JSON output, services and profiles |
| `pull` | Supported | Honors WSLC registry mirror variables |
| `build` | Supported | Context, Dockerfile, args, target, labels |
| `create` | Supported | Dependency order and project resources |
| `up` | Supported | Create/start, pull policy, profiles, recreate |
| `down` | Supported | Containers, networks, optional named volumes |
| `start/stop/restart/kill/rm` | Supported | Deterministic Compose container names |
| `ps` | Supported | Uses Compose project labels; service selection is exact |
| `logs` | Partial | Follow mode currently supports one service |
| `exec` | Supported | Env, user, workdir, TTY, interactive, detach |
| `run` | Supported | One-off container, deps, env, ports, auto-remove |
| `events/stats/top/pause/unpause` | Not yet | Requires additional WSLC command modeling |
| `watch` | Not yet | Compose develop/watch is not implemented |

## Compose Fields

| Area | Status | Notes |
| --- | --- | --- |
| File discovery and multiple `-f` | Supported | Recursive map merge, sequence append |
| `.env` and `${...}` interpolation | Supported | Default, required, alternate and `$$` forms |
| `name`, `profiles`, `depends_on` | Partial | Dependency order plus started, healthy, and completed-successfully conditions; `required` and dependency `restart` remain pending |
| `image`, `pull_policy` | Supported | CLI pull policy wins; `missing` inspects the resolved local image first |
| `build` | Supported | WSLC Dockerfile builder backend |
| `command`, `entrypoint` | Supported | String and list forms |
| `environment`, `env_file` | Supported | Map/list and optional env files |
| `ports` | Supported | Short and long syntax; WSLC decides protocol support |
| `volumes` | Supported | Bind, named, anonymous, tmpfs; short/long syntax |
| `networks` | Supported | Project networks and aliases |
| `hostname`, `domainname`, `user`, `working_dir` | Supported | Passed to `wslc.exe` |
| `cpus`, `mem_limit`, `gpus` | Supported | Passed to WSLC container creation |
| `labels` | Supported | Compose identity labels are added automatically |
| `stop_signal`, `stop_grace_period` | Supported | Command timeout can override grace period |
| `restart` | Parsed only | WSLC restart policy is not enforced yet |
| `privileged` | Experimental | Uses a persistent local SDK daemon because SDK handles cannot be reopened by a later CLI process. On a WSL runtime with privileged FUSE support, the container receives `/dev/fuse`; explicit `devices`, capability additions, security options, and ulimits remain unsupported. |
| `healthcheck` | Partial | Commands, intervals, timeouts, start periods, retries, and disable are passed to `wslc.exe`; `start_interval` is not exposed by WSLC |
| `secrets`, `configs` | Parsed only | Not mounted by the current backend |
| `deploy` | Parsed only | Swarm/deployment semantics are out of scope |
| `extends`, `include` | Validation only | Full cross-file materialization is pending |

Fields that pass Compose validation but are not applied are reported as
warnings before a container is created. SDK projects that request unsupported
runtime isolation fields (`devices`, `cap_add`, `security_opt`, or `ulimits`)
fail before any containers are created rather than silently changing semantics.

## Known Differences

1. `up` runs containers in the WSLC global control plane. It does not create a
   Docker daemon or Docker API socket.
2. Multi-service attached log multiplexing is not implemented. Use `up -d`
   followed by `logs <service>`.
3. Default recreation currently uses deterministic existing containers. Use
   `--force-recreate` after changing container settings.
4. Compose merge supports recursive mappings and appended sequences. Advanced
   Compose tags such as `!reset` and unique-resource sequence replacement are
   planned.
