[CmdletBinding()]
param([switch]$Check)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function New-RandomHex([int]$byteCount) {
    $bytes = New-Object byte[] $byteCount
    $generator = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $generator.GetBytes($bytes)
    }
    finally {
        $generator.Dispose()
    }
    return -join ($bytes | ForEach-Object { $_.ToString('x2') })
}

function Initialize-Environment([string]$path) {
    $lines = @([System.IO.File]::ReadAllLines($path))
    $secrets = @(
        @{ Key = 'JWT_SECRET'; Placeholder = 'change_me_to_a_random_32_char_string'; Bytes = 32 },
        @{ Key = 'POSTGRES_PASSWORD'; Placeholder = 'your_strong_password_here'; Bytes = 16 },
        @{ Key = 'REDIS_PASSWORD'; Placeholder = 'your_redis_password_here'; Bytes = 16 },
        @{ Key = 'RUNTIME_CONFIG_KEY'; Placeholder = 'change_me_to_a_random_64_char_hex_string'; Bytes = 32 },
        @{ Key = 'INTERNAL_SERVICE_TOKEN'; Placeholder = 'change_me_to_a_random_internal_service_token'; Bytes = 32 }
    )

    foreach ($secret in $secrets) {
        $key = [string]$secret.Key
        $index = -1
        for ($position = 0; $position -lt $lines.Count; $position++) {
            if ($lines[$position].StartsWith("$key=")) {
                $index = $position
            }
        }
        $current = if ($index -ge 0) { $lines[$index].Substring($key.Length + 1) } else { '' }
        if ([string]::IsNullOrEmpty($current) -or $current -eq $secret.Placeholder) {
            $entry = "$key=$(New-RandomHex ([int]$secret.Bytes))"
            if ($index -ge 0) {
                $lines[$index] = $entry
            }
            else {
                $lines += $entry
            }
        }
    }

    for ($position = 0; $position -lt $lines.Count; $position++) {
        if ($lines[$position] -eq 'LLM_API_KEY=sk-your-api-key') {
            $lines[$position] = 'LLM_API_KEY='
        }
        elseif ($lines[$position] -eq 'IMAGE_GEN_API_KEY=sk-your-api-key') {
            $lines[$position] = 'IMAGE_GEN_API_KEY='
        }
    }
    [System.IO.File]::WriteAllLines($path, [string[]]$lines, $utf8NoBom)
}

function Test-EnvironmentInitialization {
    $temporaryEnv = Join-Path ([System.IO.Path]::GetTempPath()) "novelworld-$([guid]::NewGuid()).env"
    try {
        Copy-Item (Join-Path $scriptRoot '.env.example') $temporaryEnv
        Initialize-Environment $temporaryEnv
        $content = [System.IO.File]::ReadAllText($temporaryEnv)
        foreach ($expected in @(
            @{ Key = 'JWT_SECRET'; Length = 64 },
            @{ Key = 'POSTGRES_PASSWORD'; Length = 32 },
            @{ Key = 'REDIS_PASSWORD'; Length = 32 },
            @{ Key = 'RUNTIME_CONFIG_KEY'; Length = 64 },
            @{ Key = 'INTERNAL_SERVICE_TOKEN'; Length = 64 }
        )) {
            if ($content -notmatch "(?m)^$($expected.Key)=[0-9a-f]{$($expected.Length)}\r?$") {
                throw "Failed to generate $($expected.Key)"
            }
        }
        if ($content -notmatch '(?m)^LLM_API_KEY=\r?$') {
            throw 'The default LLM key must remain empty for browser setup'
        }
    }
    finally {
        Remove-Item $temporaryEnv -Force -ErrorAction SilentlyContinue
    }
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
if (-not (Test-Path $envPath)) {
    Copy-Item (Join-Path $scriptRoot '.env.example') $envPath
}
Initialize-Environment $envPath

Write-Host 'Starting NovelWorld...' -ForegroundColor Cyan
& docker compose up -d --build
if ($LASTEXITCODE -ne 0) {
    throw 'Docker Compose failed to start NovelWorld.'
}

Write-Host 'NovelWorld is running at http://localhost' -ForegroundColor Green
Write-Host 'Stop: docker compose down'
Write-Host 'Logs: docker compose logs -f'
try {
    Start-Process 'http://localhost'
}
catch {
    Write-Warning 'Open http://localhost in your browser.'
}
