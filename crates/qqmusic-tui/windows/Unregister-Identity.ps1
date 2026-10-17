[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
if (Get-Process -Name qqm -ErrorAction SilentlyContinue) {
    throw 'Close the TUI and run qqm ctl quit before removing the identity.'
}
Get-AppxPackage -Name 'Jinzhongjia.QQMusicTui' | Remove-AppxPackage
Write-Output 'Identity removed. Configuration and login credentials were preserved.'
