param([Parameter(Mandatory)][string]$SetupPath)

$ErrorActionPreference = 'Stop'
if (!$IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Run this install/uninstall test only on a disposable GitHub-hosted Windows runner.'
}

$setup = (Get-Item -LiteralPath $SetupPath).FullName
$root = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'TapRelay'
$data = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'TapRelay'
$arp = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\TapRelay'
$runKey = 'HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run'
$updaterTemp = Join-Path ([IO.Path]::GetTempPath()) 'velopack_TapRelay'
if ((Test-Path -LiteralPath $root) -or (Test-Path -LiteralPath $data) -or
    (Test-Path -LiteralPath $arp) -or (Test-Path -LiteralPath $updaterTemp) -or
    ($null -ne [Microsoft.Win32.Registry]::GetValue($runKey, 'TapRelay', $null)) -or
    (Get-Process -Name TapRelay -ErrorAction SilentlyContinue)) {
    throw 'Existing TapRelay installation, user data, autostart, or process; refusing to change it.'
}

function Invoke-SilentProcess([string]$File, [string[]]$Arguments) {
    $process = Start-Process -FilePath $File -ArgumentList $Arguments -WindowStyle Hidden -PassThru
    if (!$process.WaitForExit(90000)) { throw "$File did not exit in 90 seconds." }
    if ($process.ExitCode -ne 0) { throw "$File exited with code $($process.ExitCode)." }
}

Invoke-SilentProcess $setup @('--silent')
$app = Join-Path $root 'current\TapRelay.exe'
$updater = Join-Path $root 'Update.exe'
$manifest = Join-Path $root 'current\sq.version'
foreach ($file in @($app, $updater, $manifest)) {
    if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing installed file: $file" }
}
if (!$env:TAPRELAY_SIGNTOOL) {
    $kits = "${env:ProgramFiles(x86)}/Windows Kits/10/bin"
    $env:TAPRELAY_SIGNTOOL = (Get-ChildItem "$kits/*/x64/signtool.exe" |
        Sort-Object { [version]$_.Directory.Parent.Name } -Descending |
        Select-Object -First 1).FullName
}
if (!$env:TAPRELAY_SIGNTOOL) { throw 'Install the Windows SDK x64 SignTool.' }
$sign = "$PSScriptRoot/sign-windows.ps1"
& $sign -File $updater -VerifyOnly
& $sign -File $app -VerifyOnly
$registration = Get-ItemProperty -LiteralPath $arp
if ($registration.InstallLocation -ne $root -or
    $registration.QuietUninstallString -ne ('"{0}" --uninstall --silent' -f $updater)) {
    throw 'The per-user install location or registered silent uninstall command is wrong.'
}

# A sentinel tests retention without launching Bluetooth or creating a GUI session.
[void][IO.Directory]::CreateDirectory($data)
$config = Join-Path $data 'config.json'
$sentinel = '{"uninstallRetentionProbe":true}'
[IO.File]::WriteAllText($config, $sentinel)
$portableCommand = '"{0}"' -f (Join-Path $env:RUNNER_TEMP 'Portable TapRelay\TapRelay.exe')
[Microsoft.Win32.Registry]::SetValue($runKey, 'TapRelay', $portableCommand)
Invoke-SilentProcess $app @('--veloapp-uninstall', $registration.DisplayVersion)
if ([Microsoft.Win32.Registry]::GetValue($runKey, 'TapRelay', $null) -cne $portableCommand) {
    throw 'The uninstall hook removed or changed autostart for the portable copy.'
}

[Microsoft.Win32.Registry]::SetValue($runKey, 'TapRelay', ('"{0}"' -f $app))
Invoke-SilentProcess $updater @('--uninstall', '--silent')
if ((Test-Path -LiteralPath $arp) -or
    ($null -ne [Microsoft.Win32.Registry]::GetValue($runKey, 'TapRelay', $null))) {
    throw 'Silent uninstall left its registration or its autostart.'
}
# Velopack can delegate removal of locked files to a delayed cleanup process.
$cleanup = [Diagnostics.Stopwatch]::StartNew()
while (Test-Path -LiteralPath $app) {
    if ($cleanup.Elapsed.TotalSeconds -ge 15) { throw "Uninstall did not remove $app within 15 seconds." }
    Start-Sleep -Milliseconds 250
}
if ([IO.File]::ReadAllText($config) -cne $sentinel) { throw 'Uninstall changed the user configuration.' }
Write-Host 'Silent install/uninstall passed: layout, autostart ownership, and user configuration retention.'
