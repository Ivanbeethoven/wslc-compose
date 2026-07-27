# Windows WSLC Test Guide

This guide validates `wslc-compose` against a locally built WSL Containers
runtime on Windows. It uses the WSL branch carrying privileged-container and
FUSE support, then runs normal Compose, privileged FUSE, and BrewFS fio tests.

Run the commands in PowerShell. Administrator rights are required only while
deploying WSL service binaries. Keep all SDK state on a data drive: storage
tests create dynamic VHD files that can grow by several GiB.

## WSL runtime branch

Use [`Ivanbeethoven/WSL:codex/privileged-devices`](https://github.com/Ivanbeethoven/WSL/tree/codex/privileged-devices).
It includes the privileged-container changes used by the SDK backend.

The branch requires a full Windows build: CMake 3.25 or later, Visual Studio
2022 MSVC v143, Windows SDK 26100, MSBuild, ATL, UWP v143 tools, and C++ Clang.
Enable Developer Mode or use an elevated shell so CMake can create symbolic
links.

```powershell
$WslSource = "D:\src\WSL"
git clone https://github.com/Ivanbeethoven/WSL.git $WslSource
Set-Location $WslSource
git switch --track origin/codex/privileged-devices
git pull --ff-only

cmake . -DCMAKE_BUILD_TYPE=Release
cmake --build . --config Release -- -m

$WslBuild = Join-Path $WslSource "build\bin\x64\Release"
Get-Item `
  (Join-Path $WslBuild "wslservice.exe"), `
  (Join-Path $WslBuild "wslcsession.exe"), `
  (Join-Path $WslBuild "wslc.exe"), `
  (Join-Path $WslBuild "wslcsdk.dll")
```

Do not combine binaries from different branch revisions. A partial component
build can leave the service, COM proxy, and SDK out of sync.

## Deploy the local WSL build

Stop the service, back up the files being replaced, copy branch artifacts, then
start the service. Do this from an elevated PowerShell window.

```powershell
$WslInstall = Join-Path $env:ProgramFiles "WSL"
$Backup = Join-Path $env:ProgramData ("wslc-backup-" + (Get-Date -Format "yyyyMMddHHmmss"))
New-Item -ItemType Directory -Path $Backup -Force | Out-Null

net stop wslservice
Copy-Item (Join-Path $WslInstall "wslservice.exe") $Backup -Force
Copy-Item (Join-Path $WslInstall "wslcsession.exe") $Backup -Force
Copy-Item (Join-Path $WslInstall "wslc.exe") $Backup -Force

Copy-Item (Join-Path $WslBuild "wslservice.exe") $WslInstall -Force
Copy-Item (Join-Path $WslBuild "wslcsession.exe") $WslInstall -Force
Copy-Item (Join-Path $WslBuild "wslc.exe") $WslInstall -Force

net start wslservice
Get-Service wslservice
wsl --version
wslc version
```

Do not overwrite the system SDK DLL. Put the matching branch SDK directory
first on `PATH` in each test shell instead:

```powershell
$env:WSLC_SDK_DIR = $WslBuild
$env:PATH = "$env:WSLC_SDK_DIR;$env:PATH"
```

To restore the packaged runtime, stop `wslservice`, copy the three backed-up
executables from `$Backup` to `$WslInstall`, then start the service.

## Build wslc-rs and wslc-compose

`wslc-compose` uses the published `wslc` crate. Clone `wslc-rs` as well only
when running its SDK integration tests or developing both projects together.

```powershell
$Workspace = "D:\src"
Set-Location $Workspace
git clone https://github.com/Ivanbeethoven/wslc-rs.git
git clone https://github.com/Ivanbeethoven/wslc-compose.git

Set-Location "$Workspace\wslc-rs"
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings

Set-Location "$Workspace\wslc-compose"
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release

$WslcCompose = Join-Path $PWD "target\release\wslc-compose.exe"
& $WslcCompose version
```

If a running SDK daemon locks the normal release executable, build into a
separate D-drive target rather than overwriting a running process:

```powershell
cargo build --release --target-dir D:\wslc-compose-build
$WslcCompose = "D:\wslc-compose-build\release\wslc-compose.exe"
```

## Configure D-drive session data

Use a fresh root for each workload. It contains the SDK daemon record, session
VHD, and SDK-managed named volumes. The capacity is a limit, not an eager 64
GiB allocation.

```powershell
$RunId = Get-Date -Format "yyyyMMddHHmmss"
$env:WSLC_COMPOSE_STATE_ROOT = "D:\wslc-compose-tests\$RunId"
$env:WSLC_COMPOSE_SESSION_VHD_SIZE_BYTES = "68719476736" # 64 GiB
$env:WSLC_COMPOSE_SDK_TIMEOUT_SECS = "3600"
```

Set a registry mirror only when needed. It must be a registry host without
`https://`; never add a private mirror to source or a Compose file.

```powershell
$env:WSLC_REGISTRY_MIRROR = "<registry-host>"
```

Validate the real SDK VHD path without downloading an image:

```powershell
Set-Location "$Workspace\wslc-rs"
$env:WSLC_TEST_STORAGE_ROOT = "D:\wslc-rs-tests\$RunId"
cargo test -p wslc --features integration --test integration_smoke `
  custom_session_vhd_on_configured_storage_root_can_list_images -- --nocapture
```

This verifies a real 64 GiB-capacity dynamic session VHD on `D:` and an SDK
`list_images` operation. The test terminates its own session.

## Normal Compose lifecycle

Use the bundled example first. It accepts `WSLC_COMPOSE_IMAGE`, so a registry
can be selected without changing source.

```powershell
Set-Location "$Workspace\wslc-compose"
$env:WSLC_COMPOSE_IMAGE = "docker.io/library/alpine:latest"

& $WslcCompose -f examples\compose.yaml config
& $WslcCompose -f examples\compose.yaml -p compose-smoke up -d
& $WslcCompose -f examples\compose.yaml -p compose-smoke ps
& $WslcCompose -f examples\compose.yaml -p compose-smoke exec app sh -lc "echo compose-ok"
& $WslcCompose -f examples\compose.yaml -p compose-smoke down --volumes
```

## Privileged FUSE probe

The deployed WSL branch is required for this test. The SDK backend currently
does not expose container logs, so create a temporary override that keeps the
container alive and run the checks through `exec`.

```powershell
Set-Location "$Workspace\wslc-compose"
$env:WSLC_COMPOSE_STATE_ROOT = "D:\wslc-compose-tests\fuse-$RunId"

@'
services:
  fuse-test:
    command: ["sh", "-c", "while true; do sleep 3600; done"]
'@ | Set-Content test_fuse\compose.live.yaml -NoNewline

& $WslcCompose -f test_fuse\compose.yaml -f test_fuse\compose.live.yaml -p fuse-smoke up -d
& $WslcCompose -f test_fuse\compose.yaml -f test_fuse\compose.live.yaml -p fuse-smoke ps
& $WslcCompose -f test_fuse\compose.yaml -f test_fuse\compose.live.yaml -p fuse-smoke exec fuse-test `
  sh -lc "apt-get update -qq && apt-get install -y -qq fuse3; ls -l /dev/fuse; grep fuse /proc/filesystems; dd if=/dev/fuse of=/dev/null bs=1 count=1"
& $WslcCompose -f test_fuse\compose.yaml -f test_fuse\compose.live.yaml -p fuse-smoke down --volumes
Remove-Item test_fuse\compose.live.yaml
```

The command must print a character device at `/dev/fuse` and a `fuse` entry
under `/proc/filesystems`. A missing device means the WSL service/session
deployment is not using the privileged branch.

## BrewFS fio through wslc-compose

Use the BrewFS performance branch:

```powershell
Set-Location $Workspace
git clone https://github.com/Ivanbeethoven/brewfs.git
Set-Location "$Workspace\brewfs"
git switch --track origin/codex/wslc-privileged-brewfs-perf
```

The runner requires a Linux BrewFS binary at `target\docker\brewfs`, or an
explicit `-BrewfsBinaryDir`. It creates Redis, RustFS, and the privileged FUSE
container independently for each profile.

```powershell
$env:WSLC_COMPOSE = $WslcCompose
$env:WSLC_COMPOSE_STATE_ROOT = "D:\wslc-compose-tests\brewfs-$RunId"
$Artifacts = "D:\brewfs-artifacts\$RunId"

.\docker\compose-xfstests\run_redis_perf_wslc.ps1 `
  -BrewfsBinaryDir .\target\docker `
  -ArtifactsDir $Artifacts `
  -Tools fio-bigwrite,fio-bigread
```

`fio-bigwrite` and `fio-bigread` each use eight 1 GiB jobs, or 8 GiB transferred
per profile. The artifact directory contains a fio JSON report, BrewFS log,
cache configuration, and a RustFS object report for each selected profile.

Validate fio results:

```powershell
Get-ChildItem $Artifacts -Filter "fio-*.json" | ForEach-Object {
  $report = Get-Content $_ -Raw | ConvertFrom-Json
  $bytes = [uint64](($report.jobs | ForEach-Object { $_.read.io_bytes + $_.write.io_bytes } |
    Measure-Object -Sum).Sum)
  [pscustomobject]@{
    Profile = $_.BaseName
    Errors = @($report.jobs | Where-Object { $_.error -ne 0 }).Count
    GiB = [math]::Round($bytes / 1GB, 2)
  }
} | Format-Table -AutoSize
```

Each selected big profile should report `Errors = 0` and `GiB = 8`. With the S3
backend, `fio-bigwrite-rustfs-objects.json` and
`fio-bigread-rustfs-objects.json` must contain objects.

## Cleanup and troubleshooting

`down --volumes` removes project resources, but the SDK daemon remains alive
for its state root. Do not delete a state root while its daemon or session is
active.

```powershell
$StateRoot = $env:WSLC_COMPOSE_STATE_ROOT
Get-CimInstance Win32_Process -Filter "Name = 'wslc-compose.exe'" |
  Where-Object { $_.CommandLine -match [regex]::Escape($StateRoot) } |
  Select-Object ProcessId, CommandLine
```

After that dedicated daemon has exited, remove only its exact root:

```powershell
Remove-Item -LiteralPath $StateRoot -Recurse -Force
```

If `storage.vhdx` remains locked, wait for the matching WSLC session to exit.
Do not stop the shared `wslservice` merely to remove a test VHD while unrelated
WSLC projects are active.

Common failures:

- `MissingComponents(ComponentFlags(WSL_PACKAGE))`: update/install WSL, then
  verify `Get-Service wslservice` reports `Running`.
- `SdkNotFound`: put the matching branch `wslcsdk.dll` directory first on the
  current shell `PATH`.
- `No such image` after a mirrored pull: use the current `wslc-compose` branch;
  it resolves the image once so pulling and creating use the same reference.
- Image pull with no progress: check `curl.exe -I https://<registry-host>/v2/`.
  The current WSLC SDK waits synchronously for dockerd pull responses and has no
  per-pull cancellation API; retain the D-drive state root for diagnosis.
