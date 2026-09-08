[CmdletBinding()]
param(
    [string]$Version = $(if ($env:PSTACK_VERSION) { $env:PSTACK_VERSION } else { "latest" }),
    [string]$InstallDir = $(if ($env:PSTACK_INSTALL_DIR) { $env:PSTACK_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\pstack\bin" })
)

$ErrorActionPreference = "Stop"
$repository = "thalixinc/thalix-pstack"

$architecture = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
    "X64" { "x86_64" }
    "Arm64" { "aarch64" }
    default { throw "pstack: unsupported architecture: $($_)" }
}

$target = "$architecture-pc-windows-msvc"
$archive = "pstack-$target.zip"
if ($Version -eq "latest") {
    $releaseBase = "https://github.com/$repository/releases/latest/download"
} else {
    $tag = if ($Version.StartsWith("v")) { $Version } else { "v$Version" }
    $releaseBase = "https://github.com/$repository/releases/download/$tag"
}

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("pstack-install-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Path $tempDir | Out-Null

try {
    $archivePath = Join-Path $tempDir $archive
    $checksumsPath = Join-Path $tempDir "SHA256SUMS"
    Invoke-WebRequest -UseBasicParsing -Uri "$releaseBase/$archive" -OutFile $archivePath
    Invoke-WebRequest -UseBasicParsing -Uri "$releaseBase/SHA256SUMS" -OutFile $checksumsPath

    $checksumLine = Get-Content $checksumsPath | Where-Object { $_ -match "^[0-9a-fA-F]{64}\s+\*?$([regex]::Escape($archive))$" } | Select-Object -First 1
    if (-not $checksumLine) {
        throw "pstack: release checksum is missing for $archive"
    }

    $expected = ($checksumLine -split "\s+")[0].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 -Path $archivePath).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "pstack: checksum mismatch for $archive"
    }

    Expand-Archive -Path $archivePath -DestinationPath $tempDir -Force
    $binary = Join-Path $tempDir "pstack.exe"
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "pstack: release archive does not contain pstack.exe"
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $InstallDir "pstack.exe") -Force
    & (Join-Path $InstallDir "pstack.exe") version

    $pathEntries = $env:PATH -split ";"
    if ($pathEntries -notcontains $InstallDir) {
        Write-Host "Add $InstallDir to PATH to run pstack from any shell."
    }
} finally {
    Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}
