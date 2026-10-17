[CmdletBinding()]
param(
    [string]$ExecutablePath,
    [string]$PackagePath
)

$ErrorActionPreference = 'Stop'
if (-not $ExecutablePath) {
    $ExecutablePath = Join-Path $PSScriptRoot '..\qqm.exe'
}
if ([Environment]::OSVersion.Version.Build -lt 19041) {
    throw 'Application identity requires Windows 10 version 2004 or later.'
}
$executable = (Resolve-Path -LiteralPath $ExecutablePath).ProviderPath
if ([IO.Path]::GetFileName($executable) -ne 'qqm.exe') {
    throw 'ExecutablePath must point to qqm.exe built with MSVC and the identity manifest.'
}
if (Get-Process -Name qqm -ErrorAction SilentlyContinue) {
    throw 'Close the TUI and run qqm ctl quit before registering the identity.'
}
$externalLocation = Split-Path -Parent $executable
$manifest = Join-Path $PSScriptRoot 'AppxManifest.xml'
if ($PackagePath) {
    $package = (Resolve-Path -LiteralPath $PackagePath).ProviderPath
}

# External identity assets are resolved relative to the executable directory.
$assetDirectory = Join-Path $externalLocation 'qqm-assets'
New-Item -ItemType Directory -Path $assetDirectory -Force | Out-Null
foreach ($name in @('Logo.png', 'SmallLogo.png')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "qqm-assets\$name") -Destination (Join-Path $assetDirectory $name) -Force
}

# Replace this user's old registration, whose external path may have changed.
$existing = Get-AppxPackage -Name 'Jinzhongjia.QQMusicTui'
if ($existing) {
    $existing | Remove-AppxPackage
}
try {
    if ($PackagePath) {
        Add-AppxPackage -Path $package -ExternalLocation $externalLocation
    } else {
        Add-AppxPackage -Register $manifest -ExternalLocation $externalLocation
    }
} catch {
    throw "Identity registration failed. Loose manifest registration requires permission for development deployment. For production, use -PackagePath with a trusted signed package. This script does not change Developer Mode or certificate trust. Original error: $_"
}
Get-AppxPackage -Name 'Jinzhongjia.QQMusicTui' |
    Select-Object Name, PackageFamilyName, Status
Write-Output 'Identity registered. Restart qqm to use it.'
