[CmdletBinding()]
param(
    [switch]$NoBuild,
    [string]$Binary,
    [string]$Output = "target/windows/yttt-setup.exe",
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$Iscc
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $repoRoot "Cargo.toml"
$installerScript = Join-Path $repoRoot "packaging/windows/yttt.iss"
$setupIcon = Join-Path $repoRoot "assets/app-icon/windows/AppIcon.ico"

function Resolve-RepoPath([string]$Path) {
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Path))
}

if ($Binary -and -not $NoBuild) {
    throw "-Binary requires -NoBuild so the selected artifact cannot be overwritten."
}

if (-not $NoBuild) {
    & cargo build --release --locked --manifest-path $manifest --target $Target --bin yttt
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
}

if ($Binary) {
    $binaryPath = Resolve-RepoPath $Binary
} else {
    $binaryPath = Join-Path $repoRoot "target/$Target/release/yttt.exe"
}
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
    throw "Missing executable binary: $binaryPath"
}
if (-not (Test-Path -LiteralPath $setupIcon -PathType Leaf)) {
    throw "Missing installer icon: $setupIcon"
}

$metadataJson = & cargo metadata --format-version 1 --no-deps --locked --manifest-path $manifest
if ($LASTEXITCODE -ne 0) {
    throw "cargo metadata failed with exit code $LASTEXITCODE"
}
$metadata = $metadataJson | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "yttt" } | Select-Object -First 1
if (-not $package) {
    throw "Unable to find the yttt package version in cargo metadata."
}
$version = $package.version

$outputPath = Resolve-RepoPath $Output
$outputDirectory = Split-Path -Parent $outputPath
$outputBaseFilename = [System.IO.Path]::GetFileNameWithoutExtension($outputPath)
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null

$isccCandidates = @()
if ($Iscc) {
    $isccCandidates += Resolve-RepoPath $Iscc
}
$isccCommand = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
if ($isccCommand) {
    $isccCandidates += $isccCommand.Source
}
if (${env:ProgramFiles(x86)}) {
    $isccCandidates += Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6/ISCC.exe"
}
if ($env:ProgramFiles) {
    $isccCandidates += Join-Path $env:ProgramFiles "Inno Setup 6/ISCC.exe"
}
$isccPath = $isccCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if (-not $isccPath) {
    throw "ISCC.exe was not found. Install Inno Setup 6 or pass -Iscc PATH."
}

& $isccPath `
    "/DMyAppVersion=$version" `
    "/DSourceBinary=$binaryPath" `
    "/DSetupIcon=$setupIcon" `
    "/DOutputDirectory=$outputDirectory" `
    "/DOutputBaseFilename=$outputBaseFilename" `
    $installerScript
if ($LASTEXITCODE -ne 0) {
    throw "Inno Setup failed with exit code $LASTEXITCODE"
}
if (-not (Test-Path -LiteralPath $outputPath -PathType Leaf)) {
    throw "Inno Setup did not produce the expected installer: $outputPath"
}

Write-Output $outputPath
