param(
    [Parameter(Mandatory)][string]$File,
    [switch]$VerifyOnly
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
if (!$VerifyOnly) {
    & $env:TAPRELAY_SSIGN --timestamp-url 'http://time.certum.pl' $File
}
& $env:TAPRELAY_SIGNTOOL verify /pa /v /tw $File
