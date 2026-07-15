[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Binary
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$binaryPath = [System.IO.Path]::GetFullPath($Binary)
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
    throw "Missing Windows executable: $binaryPath"
}

$bytes = [System.IO.File]::ReadAllBytes($binaryPath)
if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
    throw "Not a valid PE executable: $binaryPath"
}

$peOffset = [System.BitConverter]::ToInt32($bytes, 0x3C)
$optionalHeaderOffset = $peOffset + 24
$subsystemOffset = $optionalHeaderOffset + 68
if ($peOffset -lt 0 -or $subsystemOffset + 2 -gt $bytes.Length) {
    throw "Invalid PE header offsets: $binaryPath"
}

$peSignature = [System.Text.Encoding]::ASCII.GetString($bytes, $peOffset, 4)
if ($peSignature -ne "PE`0`0") {
    throw "Missing PE signature: $binaryPath"
}

$windowsGuiSubsystem = 2
$subsystem = [System.BitConverter]::ToUInt16($bytes, $subsystemOffset)
if ($subsystem -ne $windowsGuiSubsystem) {
    throw "Expected Windows GUI subsystem (2), found $subsystem in $binaryPath"
}

if (-not ("Yttt.Windows.NativeMethods" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

namespace Yttt.Windows {
    public static class NativeMethods {
        [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
        public static extern uint ExtractIconEx(
            string fileName,
            int iconIndex,
            IntPtr[] largeIcons,
            IntPtr[] smallIcons,
            uint iconCount);
    }
}
"@
}

$iconCount = [Yttt.Windows.NativeMethods]::ExtractIconEx(
    $binaryPath,
    -1,
    $null,
    $null,
    0)
if ($iconCount -lt 1) {
    throw "Executable has no embedded application icon: $binaryPath"
}

$versionInfo = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($binaryPath)
if ($versionInfo.ProductName -ne "yttt" -or $versionInfo.OriginalFilename -ne "yttt.exe") {
    throw "Executable version resources are incomplete: $binaryPath"
}

Write-Output "Verified Windows GUI executable with $iconCount embedded icon group(s): $binaryPath"
