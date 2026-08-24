[CmdletBinding()]
param([switch]$Check)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function New-RandomHex([int]$byteCount) {
    $bytes = New-Object byte[] $byteCount
    $generator = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $generator.GetBytes($bytes) }
    finally { $generator.Dispose() }
    return -join ($bytes | ForEach-Object { $_.ToString('x2') })
}

function Get-EnvironmentEntry([string]$path, [string]$key) {
    $matches = @([System.IO.File]::ReadAllLines($path) | Where-Object { $_.StartsWith("$key=") })
    if ($matches.Count -gt 1) { throw "Duplicate $key entries in $path" }
    if ($matches.Count -eq 0) { return @{ Exists = $false; Value = '' } }
    return @{ Exists = $true; Value = $matches[0].Substring($key.Length + 1) }
}

function Set-EnvironmentValue([string]$path, [string]$key, [string]$value) {
    $lines = @([System.IO.File]::ReadAllLines($path))
    $index = -1
    for ($position = 0; $position -lt $lines.Count; $position++) {
        if ($lines[$position].StartsWith("$key=")) {
            if ($index -ge 0) { throw "Duplicate $key entries in $path" }
            $index = $position
        }
    }
    $entry = "$key=$value"
    if ($index -ge 0) { $lines[$index] = $entry } else { $lines += $entry }
    [System.IO.File]::WriteAllLines($path, [string[]]$lines, $utf8NoBom)
}

function Test-RedisPassword([string]$value) {
    $lowered = $value.ToLowerInvariant()
    $distinct = @($value.ToCharArray() | Select-Object -Unique).Count
    return $value -match '^[A-Za-z0-9._~-]{16,}$' -and
        $distinct -ge 8 -and
        -not $lowered.Contains('placeholder') -and
        -not $lowered.Contains('change_me') -and
        $lowered -ne 'your_redis_password_here' -and
        $lowered -ne 'runtime-redis-only'
}

function Resolve-CacheMode([string]$path) {
    $modeEntry = Get-EnvironmentEntry $path 'CACHE_MODE'
    $redisEntry = Get-EnvironmentEntry $path 'REDIS_PASSWORD'

    if (-not $modeEntry.Exists) {
        # Old launchers always generated a Redis password. New templates persist
        # postgres before this compatibility migration runs.
        $legacyRedis = -not [string]::IsNullOrEmpty($redisEntry.Value) -and
            $redisEntry.Value -ne 'your_redis_password_here'
        $mode = if ($legacyRedis) { 'redis' } else { 'postgres' }
        Set-EnvironmentValue $path 'CACHE_MODE' $mode
        if (-not $legacyRedis -and $redisEntry.Value -eq 'your_redis_password_here') {
            Set-EnvironmentValue $path 'REDIS_PASSWORD' ''
        }
    }
    else { $mode = $modeEntry.Value }

    if ($mode -notin @('postgres', 'redis')) {
        throw 'CACHE_MODE must be exactly postgres or redis.'
    }
    if ($mode -eq 'redis' -and -not (Test-RedisPassword $redisEntry.Value)) {
        throw 'CACHE_MODE=redis requires a URL-safe, non-placeholder REDIS_PASSWORD of at least 16 characters with 8 distinct characters.'
    }
    return $mode
}

function Initialize-Environment([string]$path) {
    $null = Resolve-CacheMode $path
    $secrets = @(
        @{ Key = 'JWT_SECRET'; Placeholder = 'change_me_to_a_random_32_char_string'; Bytes = 32 },
        @{ Key = 'POSTGRES_PASSWORD'; Placeholder = 'your_strong_password_here'; Bytes = 16 },
        @{ Key = 'RUNTIME_CONFIG_KEY'; Placeholder = 'change_me_to_a_random_64_char_hex_string'; Bytes = 32 },
        @{ Key = 'INTERNAL_SERVICE_TOKEN'; Placeholder = 'change_me_to_a_random_internal_service_token'; Bytes = 32 }
    )
    foreach ($secret in $secrets) {
        $entry = Get-EnvironmentEntry $path ([string]$secret.Key)
        if ([string]::IsNullOrEmpty($entry.Value) -or $entry.Value -eq $secret.Placeholder) {
            Set-EnvironmentValue $path ([string]$secret.Key) (New-RandomHex ([int]$secret.Bytes))
        }
    }
    foreach ($key in @('LLM_API_KEY', 'IMAGE_GEN_API_KEY')) {
        $entry = Get-EnvironmentEntry $path $key
        if ($entry.Value -eq 'sk-your-api-key') { Set-EnvironmentValue $path $key '' }
    }
}

function Test-EnvironmentInitialization {
    $temporaryDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "novelworld-$([guid]::NewGuid())"
    New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null
    try {
        $freshEnv = Join-Path $temporaryDirectory 'fresh.env'
        Copy-Item (Join-Path $scriptRoot '.env.example') $freshEnv
        Initialize-Environment $freshEnv
        $content = [System.IO.File]::ReadAllText($freshEnv)
        foreach ($expected in @(
            @{ Key = 'JWT_SECRET'; Length = 64 },
            @{ Key = 'POSTGRES_PASSWORD'; Length = 32 },
            @{ Key = 'RUNTIME_CONFIG_KEY'; Length = 64 },
            @{ Key = 'INTERNAL_SERVICE_TOKEN'; Length = 64 }
        )) {
            if ($content -notmatch "(?m)^$($expected.Key)=[0-9a-f]{$($expected.Length)}\r?$") {
                throw "Failed to generate $($expected.Key)"
            }
        }
        if ($content -notmatch '(?m)^CACHE_MODE=postgres\r?$' -or
            $content -notmatch '(?m)^REDIS_PASSWORD=\r?$') {
            throw 'Fresh installs must persist postgres mode without generating Redis credentials.'
        }
        if ($content -notmatch '(?m)^LLM_API_KEY=\r?$') {
            throw 'The default LLM key must remain empty for browser setup.'
        }

        $legacyEnv = Join-Path $temporaryDirectory 'legacy.env'
        $legacyLines = @([System.IO.File]::ReadAllLines((Join-Path $scriptRoot '.env.example')) |
            Where-Object { -not $_.StartsWith('CACHE_MODE=') })
        for ($position = 0; $position -lt $legacyLines.Count; $position++) {
            if ($legacyLines[$position].StartsWith('REDIS_PASSWORD=')) {
                $legacyLines[$position] = 'REDIS_PASSWORD=0123456789abcdef0123456789abcdef'
            }
        }
        [System.IO.File]::WriteAllLines($legacyEnv, [string[]]$legacyLines, $utf8NoBom)
        Initialize-Environment $legacyEnv
        if ((Get-EnvironmentEntry $legacyEnv 'CACHE_MODE').Value -ne 'redis') {
            throw 'A pre-decision environment with a Redis credential must migrate once to redis mode.'
        }

        $halfSelectedEnv = Join-Path $temporaryDirectory 'half-selected.env'
        Copy-Item (Join-Path $scriptRoot '.env.example') $halfSelectedEnv
        Set-EnvironmentValue $halfSelectedEnv 'CACHE_MODE' 'redis'
        $rejected = $false
        try { $null = Resolve-CacheMode $halfSelectedEnv } catch { $rejected = $true }
        if (-not $rejected) { throw 'A half-selected Redis mode was accepted.' }

        Set-EnvironmentValue $halfSelectedEnv 'REDIS_PASSWORD' 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
        $rejected = $false
        try { $null = Resolve-CacheMode $halfSelectedEnv } catch { $rejected = $true }
        if (-not $rejected) { throw 'A low-entropy Redis credential was accepted.' }

        $windowsLauncher = [System.IO.File]::ReadAllText((Join-Path $scriptRoot 'start.ps1'))
        if ($windowsLauncher -notmatch '(?s)compose.*--profile.*redis.*down.*composeArgs.*--wait') {
            throw 'Windows launcher must stop old writers and wait for selected-profile readiness.'
        }
        $unixLauncher = [System.IO.File]::ReadAllText((Join-Path $scriptRoot 'start.sh'))
        if ($unixLauncher -notmatch '(?s)docker compose --profile redis down.*--wait --wait-timeout 180') {
            throw 'Unix launcher must stop old writers and wait for selected-profile readiness.'
        }
        $compose = [System.IO.File]::ReadAllText((Join-Path $scriptRoot 'docker-compose.yml'))
        if ($compose -notmatch 'REDIS_PASSWORD must be URL-safe.*8 distinct characters') {
            throw 'The Redis profile must reject a direct half-selected or weak credential before Redis starts.'
        }
    }
    finally { Remove-Item $temporaryDirectory -Recurse -Force -ErrorAction SilentlyContinue }
    Write-Host 'Windows launcher self-check passed.' -ForegroundColor Green
}

if ($Check) {
    Test-EnvironmentInitialization
    exit 0
}

Set-Location $scriptRoot
if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    throw 'Docker Desktop is not installed. Install it from https://docs.docker.com/desktop/setup/install/windows-install/'
}
& docker compose version *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'Docker Compose v2 is not available. Start or update Docker Desktop.'
}

$envPath = Join-Path $scriptRoot '.env'
if (-not (Test-Path $envPath)) { Copy-Item (Join-Path $scriptRoot '.env.example') $envPath }
Initialize-Environment $envPath
$cacheMode = Resolve-CacheMode $envPath
$env:CACHE_MODE = $cacheMode
$env:COMPOSE_PROFILES = ''
$composeArgs = @('compose')
if ($cacheMode -eq 'redis') {
    $redisPassword = (Get-EnvironmentEntry $envPath 'REDIS_PASSWORD').Value
    $env:REDIS_PASSWORD = $redisPassword
    $env:REDIS_URL = "redis://:${redisPassword}@redis:6379"
    $composeArgs += @('--profile', 'redis')
}
else {
    $env:REDIS_PASSWORD = ''
    $env:REDIS_URL = 'memory://'
}

Write-Host 'Stopping any existing NovelWorld stack before migrations...' -ForegroundColor Cyan
& docker compose --profile redis down
if ($LASTEXITCODE -ne 0) { throw 'Docker Compose failed to stop the existing NovelWorld stack.' }

Write-Host "Starting NovelWorld (cache: $cacheMode)..." -ForegroundColor Cyan
$composeArgs += @('up', '-d', '--build', '--wait', '--wait-timeout', '180')
& docker @composeArgs
if ($LASTEXITCODE -ne 0) { throw 'Docker Compose did not make NovelWorld ready.' }

Write-Host 'NovelWorld is ready at http://localhost' -ForegroundColor Green
Write-Host 'Stop: docker compose --profile redis down'
Write-Host 'Logs: docker compose logs -f'
try { Start-Process 'http://localhost' }
catch { Write-Warning 'Open http://localhost in your browser.' }
