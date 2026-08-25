[CmdletBinding()]
param(
    [switch]$Check,
    [switch]$ResumeAfterL0
)

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

function Test-PostgresIdentifier([string]$value) {
    return $value -cmatch '^[a-z_][a-z0-9_]{0,62}$'
}

function Test-PostgresPassword([string]$value) {
    $lowered = $value.ToLowerInvariant()
    $distinct = @($value.ToCharArray() | Select-Object -Unique).Count
    return $value -match '^[A-Za-z0-9._~-]{16,}$' -and
        $distinct -ge 8 -and
        -not $lowered.Contains('placeholder') -and
        -not $lowered.Contains('change_me') -and
        $lowered -ne 'your_strong_password_here'
}

function Assert-L0Configuration([string]$path) {
    $user = (Get-EnvironmentEntry $path 'POSTGRES_USER').Value
    $database = (Get-EnvironmentEntry $path 'POSTGRES_DB').Value
    $password = (Get-EnvironmentEntry $path 'POSTGRES_PASSWORD').Value
    if (-not (Test-PostgresIdentifier $user)) {
        throw 'POSTGRES_USER must be a lowercase PostgreSQL identifier (1-63 characters).'
    }
    if (-not (Test-PostgresIdentifier $database)) {
        throw 'POSTGRES_DB must be a lowercase PostgreSQL identifier (1-63 characters).'
    }
    if (-not (Test-PostgresPassword $password)) {
        throw 'POSTGRES_PASSWORD must be URL-safe, non-placeholder, at least 16 characters, and contain at least 8 distinct characters.'
    }
}

function Initialize-L0Configuration([string]$path, [bool]$allowPrompt) {
    $marker = Get-EnvironmentEntry $path 'BOOTSTRAP_L0_COMPLETE'
    if ($marker.Exists -and $marker.Value -notin @('true', 'false')) {
        throw 'BOOTSTRAP_L0_COMPLETE must be exactly true or false.'
    }
    if ($marker.Value -eq 'true') {
        Assert-L0Configuration $path
        return $false
    }

    $user = (Get-EnvironmentEntry $path 'POSTGRES_USER').Value
    $database = (Get-EnvironmentEntry $path 'POSTGRES_DB').Value
    $password = (Get-EnvironmentEntry $path 'POSTGRES_PASSWORD').Value
    if ((Test-PostgresIdentifier $user) -and
        (Test-PostgresIdentifier $database) -and
        (Test-PostgresPassword $password)) {
        # Existing and automation-preseeded installations migrate without a prompt.
        Set-EnvironmentValue $path 'BOOTSTRAP_L0_COMPLETE' 'true'
        return $false
    }

    if (-not $allowPrompt -or [Console]::IsInputRedirected) {
        throw 'L0 database setup is incomplete. Run start.cmd interactively, or preseed POSTGRES_USER, POSTGRES_DB, and a strong POSTGRES_PASSWORD in .env.'
    }

    Write-Host ''
    Write-Host 'First launch: configure required local PostgreSQL (L0)' -ForegroundColor Cyan
    Write-Host 'The database password is generated automatically. Model, Redis, and object storage settings can wait.'
    $defaultUser = if (Test-PostgresIdentifier $user) { $user } else { 'novel' }
    $defaultDatabase = if (Test-PostgresIdentifier $database) { $database } else { 'novel_world' }
    $enteredUser = Read-Host "PostgreSQL user [$defaultUser]"
    $enteredDatabase = Read-Host "PostgreSQL database [$defaultDatabase]"
    if ([string]::IsNullOrWhiteSpace($enteredUser)) { $enteredUser = $defaultUser }
    if ([string]::IsNullOrWhiteSpace($enteredDatabase)) { $enteredDatabase = $defaultDatabase }
    if (-not (Test-PostgresIdentifier $enteredUser)) {
        throw 'PostgreSQL user must be 1-63 lowercase letters, digits, or underscores and cannot start with a digit.'
    }
    if (-not (Test-PostgresIdentifier $enteredDatabase)) {
        throw 'PostgreSQL database must be 1-63 lowercase letters, digits, or underscores and cannot start with a digit.'
    }

    Set-EnvironmentValue $path 'POSTGRES_USER' $enteredUser
    Set-EnvironmentValue $path 'POSTGRES_DB' $enteredDatabase
    Set-EnvironmentValue $path 'POSTGRES_PASSWORD' (New-RandomHex 16)
    # Commit point: never mark L0 complete until every hard database value is persisted.
    Set-EnvironmentValue $path 'BOOTSTRAP_L0_COMPLETE' 'true'
    Assert-L0Configuration $path
    return $true
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
        $rejected = $false
        try { $null = Initialize-L0Configuration $freshEnv $false } catch { $rejected = $true }
        if (-not $rejected) { throw 'An unconfigured non-interactive L0 launch was accepted.' }

        $seededPassword = '0123456789abcdef0123456789abcdef'
        Set-EnvironmentValue $freshEnv 'POSTGRES_USER' 'novel'
        Set-EnvironmentValue $freshEnv 'POSTGRES_DB' 'novel_world'
        Set-EnvironmentValue $freshEnv 'POSTGRES_PASSWORD' $seededPassword
        $configuredInteractively = Initialize-L0Configuration $freshEnv $false
        if ($configuredInteractively) { throw 'A valid preseed unexpectedly requested interactive setup.' }
        Initialize-Environment $freshEnv
        $content = [System.IO.File]::ReadAllText($freshEnv)
        foreach ($expected in @(
            @{ Key = 'JWT_SECRET'; Length = 64 },
            @{ Key = 'RUNTIME_CONFIG_KEY'; Length = 64 },
            @{ Key = 'INTERNAL_SERVICE_TOKEN'; Length = 64 }
        )) {
            if ($content -notmatch "(?m)^$($expected.Key)=[0-9a-f]{$($expected.Length)}\r?$") {
                throw "Failed to generate $($expected.Key)"
            }
        }
        if ($content -notmatch '(?m)^BOOTSTRAP_L0_COMPLETE=true\r?$' -or
            (Get-EnvironmentEntry $freshEnv 'POSTGRES_PASSWORD').Value -ne $seededPassword) {
            throw 'A valid preseed must be committed without replacing its database password.'
        }
        if ($content -notmatch '(?m)^CACHE_MODE=postgres\r?$' -or
            $content -notmatch '(?m)^REDIS_PASSWORD=\r?$') {
            throw 'Fresh installs must persist postgres mode without generating Redis credentials.'
        }
        if ($content -notmatch '(?m)^LLM_API_KEY=\r?$') {
            throw 'The default LLM key must remain empty for browser setup.'
        }

        $invalidCommittedEnv = Join-Path $temporaryDirectory 'invalid-committed.env'
        Copy-Item (Join-Path $scriptRoot '.env.example') $invalidCommittedEnv
        Set-EnvironmentValue $invalidCommittedEnv 'BOOTSTRAP_L0_COMPLETE' 'true'
        $rejected = $false
        try { $null = Initialize-L0Configuration $invalidCommittedEnv $false } catch { $rejected = $true }
        if (-not $rejected) { throw 'A committed L0 marker bypassed database validation.' }

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
        if ($windowsLauncher -notmatch '(?s)BOOTSTRAP_L0_COMPLETE.*ResumeAfterL0.*powershell\.exe.*-ResumeAfterL0') {
            throw 'Windows launcher must commit L0 and automatically restart itself.'
        }
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
$envPath = Join-Path $scriptRoot '.env'
if (-not (Test-Path $envPath)) { Copy-Item (Join-Path $scriptRoot '.env.example') $envPath }
$l0Configured = Initialize-L0Configuration $envPath $true
if ($l0Configured) {
    if ($ResumeAfterL0) { throw 'L0 setup restart loop detected.' }
    Write-Host 'L0 database settings saved. Restarting the launcher automatically...' -ForegroundColor Green
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath -ResumeAfterL0
    exit $LASTEXITCODE
}

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    throw 'Docker Desktop is not installed. Install it from https://docs.docker.com/desktop/setup/install/windows-install/'
}
& docker compose version *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'Docker Compose v2 is not available. Start or update Docker Desktop.'
}

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
