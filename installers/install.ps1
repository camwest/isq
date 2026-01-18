#Requires -Version 5.1
<#
.SYNOPSIS
    Installs isq CLI for Windows
.EXAMPLE
    irm https://cameronwestland.com/isq/install.ps1 | iex
#>

$ErrorActionPreference = 'Stop'
$Repo = "camwest/isq"
$Target = "x86_64-pc-windows-msvc"

function Write-Info([string]$Message) { Write-Host "=> $Message" -ForegroundColor Cyan }
function Write-Ok([string]$Message) { Write-Host "ok $Message" -ForegroundColor Green }

function Get-LatestVersion {
    $url = "https://api.github.com/repos/$Repo/releases/latest"
    try {
        return (Invoke-RestMethod -Uri $url -UseBasicParsing).tag_name
    }
    catch {
        $msg = if ($_.Exception.Message -match "rate limit") {
            "GitHub API rate limit exceeded. Try again later or download from: https://github.com/$Repo/releases/latest"
        } else { "Failed to fetch latest version: $_" }
        throw $msg
    }
}

function Add-ToUserPath([string]$Dir) {
    $currentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    if ($currentPath -split ';' -contains $Dir) {
        Write-Host "$Dir is already in PATH" -ForegroundColor Gray
        return
    }
    $newPath = if ($currentPath) { "$currentPath;$Dir" } else { $Dir }
    [Environment]::SetEnvironmentVariable("PATH", $newPath, "User")
    Write-Ok "Added $Dir to user PATH"
}

function Install-Isq {
    Write-Info "Installing isq..."

    $version = Get-LatestVersion
    Write-Info "Latest version: $version"

    $archive = "isq-$Target.zip"
    $baseUrl = "https://github.com/$Repo/releases/download/$version"
    $tmpDir = Join-Path $env:TEMP "isq-install-$(Get-Random)"
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

    try {
        $archivePath = Join-Path $tmpDir $archive
        $checksumPath = Join-Path $tmpDir "checksums.txt"

        Write-Info "Downloading $archive..."
        Invoke-WebRequest -Uri "$baseUrl/$archive" -OutFile $archivePath -UseBasicParsing

        Write-Info "Downloading checksums..."
        Invoke-WebRequest -Uri "$baseUrl/checksums.txt" -OutFile $checksumPath -UseBasicParsing

        Write-Info "Verifying checksum..."
        $expectedLine = Get-Content $checksumPath | Where-Object { $_ -match $archive }
        if (-not $expectedLine) { throw "Checksum for $archive not found" }
        $expectedHash = ($expectedLine -split '\s+')[0]
        $actualHash = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLower()

        if ($expectedHash -ne $actualHash) {
            throw "Checksum verification failed! Expected: $expectedHash, Got: $actualHash"
        }
        Write-Ok "Checksum verified"

        Write-Info "Extracting..."
        Expand-Archive -Path $archivePath -DestinationPath $tmpDir -Force

        $installDir = Join-Path $env:LOCALAPPDATA "Programs\isq"
        New-Item -ItemType Directory -Path $installDir -Force -ErrorAction SilentlyContinue | Out-Null

        Write-Info "Installing to $installDir..."
        $binaryPath = Join-Path $installDir "isq.exe"
        Copy-Item -Path (Join-Path $tmpDir "isq.exe") -Destination $binaryPath -Force

        Add-ToUserPath $installDir

        & $binaryPath install write-receipt --method standalone --binary-path $binaryPath --auto-update 2>$null

        Write-Host ""
        Write-Ok "isq $version installed to $binaryPath"
        Write-Host ""
        Write-Host "Restart your terminal to use isq, or run:" -ForegroundColor Yellow
        Write-Host "  `$env:PATH += `";$installDir`"" -ForegroundColor Yellow
        Write-Host ""
        Write-Host "Run 'isq --help' to get started."
    }
    finally {
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Main execution
try {
    Install-Isq
}
catch {
    Write-Host "error: $_" -ForegroundColor Red
    exit 1
}
