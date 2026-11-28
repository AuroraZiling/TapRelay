param(
    [Parameter(Mandatory)]
    [ValidatePattern('^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$')]
    [string]$Version
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
Set-Location "$PSScriptRoot/.."
if (!$env:CERTUM_EMAIL -or !$env:CERTUM_OTP) { throw 'Missing Certum signing credentials.' }
$kits = "${env:ProgramFiles(x86)}/Windows Kits/10/bin"
$env:TAPRELAY_SIGNTOOL = (Get-ChildItem "$kits/*/x64/signtool.exe" |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending |
    Select-Object -First 1).FullName
if (!$env:TAPRELAY_SIGNTOOL) { throw 'Install the Windows SDK x64 SignTool.' }

$stagePath = 'release-package/windows'
if (Test-Path -LiteralPath $stagePath) {
    Remove-Item -LiteralPath $stagePath -Recurse -Force
}
$stage = New-Item -ItemType Directory -Path $stagePath -Force
$assets = New-Item -ItemType Directory -Path 'release-assets' -Force
$inputDir = New-Item -ItemType Directory -Path "$stage/input" -Force
Copy-Item -LiteralPath 'target/release/taprelay.exe' -Destination "$inputDir/TapRelay.exe"
Copy-Item -LiteralPath 'LICENSE', 'THIRD_PARTY_NOTICES.md' -Destination $inputDir
$sign = "$PSScriptRoot/sign-windows.ps1"
$template = 'pwsh -NoProfile -NonInteractive -File "' + $sign + '" {{file}}'
vpk pack --skip-updates --packId TapRelay --packVersion $Version --packAuthors 'Artemis Li' --packDir $inputDir.FullName `
    --mainExe TapRelay.exe --channel win --runtime win-x64 --delta None --noPortable `
    --icon 'crates/taprelay-app/resources/taprelay.ico' --outputDir "$stage/vpk" `
    --signParallel 1 --signTemplate $template

$feed = Get-Item -LiteralPath "$stage/vpk/releases.win.json"
$full = (Get-Content -LiteralPath $feed -Raw | ConvertFrom-Json).Assets
if ($full.Count -ne 1 -or $full[0].FileName -cne "TapRelay-$Version-full.nupkg" -or $full[0].Type -ne 'Full') {
    throw 'Expected one Windows full package for the release version.'
}
$package = Get-Item -LiteralPath "$stage/vpk/$($full[0].FileName)"
$setup = Get-Item -LiteralPath "$stage/vpk/TapRelay-win-Setup.exe"
& $sign -File $setup.FullName -VerifyOnly
[IO.Compression.ZipFile]::ExtractToDirectory($package.FullName, "$stage/extracted")
$payload = "$stage/extracted/lib/app"
$pes = @(Get-ChildItem -LiteralPath $payload -Recurse -File -Include '*.exe','*.dll')
if ($pes.Count -eq 0) { throw 'Expected at least one packaged Windows executable to verify.' }
foreach ($pe in $pes) {
    & $sign -File $pe.FullName -VerifyOnly
}

$prefix = "$assets/TapRelay_${Version}_Windows_x64"
Copy-Item -LiteralPath $setup -Destination "${prefix}_Setup.exe" -Force
Copy-Item -LiteralPath $feed, $package -Destination $assets -Force
Compress-Archive -LiteralPath "$payload/TapRelay.exe", "$payload/LICENSE", "$payload/THIRD_PARTY_NOTICES.md" `
    -DestinationPath "$prefix.zip" -Force
