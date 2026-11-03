#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$Version = "latest",
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "civitfit\bin")
)

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$Repo = "redsynapse/civitfit"
$Bin = "civitfit"

function Get-PlatformTarget {
    $arch = $null
    try {
        $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch {
        $arch = $env:PROCESSOR_ARCHITECTURE
    }

    switch ($arch) {
        "X64" { return "x86_64-pc-windows-msvc" }
        "AMD64" { return "x86_64-pc-windows-msvc" }
        "Arm64" { return "aarch64-pc-windows-msvc" }
        "ARM64" { return "aarch64-pc-windows-msvc" }
        default { throw "Unsupported architecture: $arch" }
    }
}

function Get-LatestTag {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
        -Headers @{ "User-Agent" = "civitfit-installer" }
    return $release.tag_name
}

$target = Get-PlatformTarget

if ($Version -eq "latest") {
    $Version = Get-LatestTag
}
if ($Version -notmatch '^v') {
    $Version = "v$Version"
}

$asset = "$Bin-$Version-$target.zip"
$url = "https://github.com/$Repo/releases/download/$Version/$asset"
$tmp = Join-Path $env:TEMP ("civitfit-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
    $zip = Join-Path $tmp $asset
    Write-Host "Downloading $asset"
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing

    $shaFile = Join-Path $tmp "$asset.sha256"
    $checksumAvailable = $true
    try {
        Invoke-WebRequest -Uri "$url.sha256" -OutFile $shaFile -UseBasicParsing
    } catch {
        $checksumAvailable = $false
        Write-Warning "Checksum not available; skipping verification"
    }

    if ($checksumAvailable) {
        $expected = (Get-Content $shaFile -Raw).Split()[0].ToLowerInvariant()
        $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($expected -ne $actual) {
            throw "Checksum mismatch for $asset"
        }
        Write-Host "Checksum OK"
    }

    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    $src = Join-Path $tmp "$Bin-$Version-$target\$Bin.exe"
    if (-not (Test-Path $src)) {
        throw "Binary not found in archive: $src"
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $dest = Join-Path $InstallDir "$Bin.exe"
    Copy-Item -Path $src -Destination $dest -Force

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not $userPath) { $userPath = "" }
    $parts = $userPath.Split(";", [System.StringSplitOptions]::RemoveEmptyEntries)
    if ($parts -notcontains $InstallDir) {
        $newPath = if ($userPath.TrimEnd(";")) {
            "$($userPath.TrimEnd(';'));$InstallDir"
        } else {
            $InstallDir
        }
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        Write-Host "Added $InstallDir to the user PATH. Open a new terminal to use civitfit."
    }

    Write-Host "Installed $Bin $Version to $dest"
    & $dest --version
} finally {
    Remove-Item -Path $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
