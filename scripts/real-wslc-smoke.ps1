param(
    [string]$Executable
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$composeFile = Join-Path $root "examples/recreate/compose.yaml"
$changedFile = Join-Path $root "examples/recreate/compose.changed.yaml"

if (-not $Executable) {
    $Executable = Join-Path $root "target/release/wslc-compose.exe"
}
if (-not (Test-Path -LiteralPath $Executable)) {
    throw "wslc-compose executable not found: $Executable (run cargo build --release --locked)"
}

$images = wslc image list --format json | ConvertFrom-Json
$alpine = $images |
    Where-Object {
        $_.Repository -and
        $_.Repository.EndsWith("/library/alpine") -and
        $_.Tag -eq "latest"
    } |
    Select-Object -First 1
if (-not $alpine) {
    throw "No local */library/alpine:latest image is available for the smoke test"
}

$env:WSLC_REGISTRY_MIRROR = $alpine.Repository.Split("/", 2)[0]
$env:WSLC_COMPOSE_IMAGE = "docker.io/library/alpine:latest"

function Get-TestContainerId {
    $container = wslc inspect --type container recreate-demo-web-1 | ConvertFrom-Json
    if ($container -is [array]) {
        return $container[0].Id
    }
    return $container.Id
}

try {
    & $Executable -f $composeFile up -d --pull never
    $initialId = Get-TestContainerId

    $sameOutput = & $Executable -f $composeFile up -d --pull never | Out-String
    $sameId = Get-TestContainerId
    if ($initialId -ne $sameId -or $sameOutput -notmatch "is up to date") {
        throw "An unchanged service was unexpectedly recreated"
    }

    $changedOutput = & $Executable -f $composeFile -f $changedFile up -d --pull never | Out-String
    $changedId = Get-TestContainerId
    if ($sameId -eq $changedId -or $changedOutput -notmatch "configuration changed") {
        throw "A changed service was not recreated"
    }

    & $Executable -f $composeFile up -d --pull never --no-recreate | Out-Null
    if ($changedId -ne (Get-TestContainerId)) {
        throw "--no-recreate did not preserve the existing container"
    }

    $logs = & $Executable -f $composeFile logs --tail 5 web worker | Out-String
    if ($logs -notmatch "web\s+\| web-v1" -or $logs -notmatch "worker\s+\| worker-v1") {
        throw "Multiplexed logs did not contain both service prefixes"
    }

    & $Executable -f $composeFile stats --no-trunc web worker
    Write-Host "Real WSLC smoke test passed"
    Write-Host "unchanged: $initialId"
    Write-Host "recreated: $changedId"
} finally {
    & $Executable -f $composeFile down --remove-orphans --volumes
}
