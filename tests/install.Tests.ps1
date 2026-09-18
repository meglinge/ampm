# Offline tests: real file installation, mocked network/registry/shortcuts/processes.
$ErrorActionPreference = 'Stop'
$installer = Join-Path $PSScriptRoot '..\scripts\install.ps1'
$root = Join-Path ([IO.Path]::GetTempPath()) ('ampm-install-test-' + [guid]::NewGuid().ToString('N'))
Microsoft.PowerShell.Management\New-Item -ItemType Directory $root | Out-Null
$fixture = @{}
$fixture.installDir = Join-Path $root 'Program Files & space'
$fixture.corrupt = $false
$fixture.running = $false
$fixture.runtime = $true
$fixture.signed = $true
$fixture.urls = @()
$fixture.properties = @{}
$fixture.links = @()
$fixture.starts = 0
$fixture.bytes = [byte[]]::new(128)
$fixture.bytes[0] = 0x4d; $fixture.bytes[1] = 0x5a; $fixture.bytes[60] = 64
$fixture.bytes[64] = 0x50; $fixture.bytes[65] = 0x45; $fixture.bytes[68] = 0x64; $fixture.bytes[69] = 0x86
$sha = [Security.Cryptography.SHA256]::Create()
try { $fixture.checksum = [BitConverter]::ToString($sha.ComputeHash($fixture.bytes)).Replace('-', '') + '  ampm-windows-x64.exe' }
finally { $sha.Dispose() }

function Assert($Condition, [string]$Message) { if (-not $Condition) { throw "Assertion failed: $Message" } }
function Get-FixtureHash($LiteralPath) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { [pscustomobject]@{Hash=[BitConverter]::ToString($sha.ComputeHash([IO.File]::ReadAllBytes($LiteralPath))).Replace('-', '')} }
    finally { $sha.Dispose() }
}
function Expect-Failure([scriptblock]$Action, [string]$Pattern) {
    $errorMessage = ''
    try { & $Action } catch { $errorMessage = $_.Exception.Message }
    Assert ($errorMessage -match $Pattern) "Expected '$Pattern', got '$errorMessage'"
}
function Invoke-WebRequest($Uri, [switch]$UseBasicParsing, $OutFile, $TimeoutSec) {
    $fixture.urls += $Uri
    if ($Uri -like '*/ampm-windows-x64.exe.sha256') {
        [IO.File]::WriteAllText($OutFile, $fixture.checksum)
    } elseif ($Uri -like '*/ampm-windows-x64.exe') {
        $data = $fixture.bytes.Clone()
        if ($fixture.corrupt) { $data[100] = 1 }
        [IO.File]::WriteAllBytes($OutFile, $data)
    } elseif ($Uri -like '*/install.ps1') {
        [IO.File]::WriteAllText($OutFile, '# fixture uninstaller')
    } elseif ($Uri -eq 'https://go.microsoft.com/fwlink/p/?LinkId=2124703') {
        [IO.File]::WriteAllText($OutFile, 'fixture runtime')
    } else { throw "Unexpected network URL: $Uri" }
}
function Get-Process($ErrorAction) {
    if ($fixture.running) { [pscustomobject]@{ Path = (Join-Path $fixture.installDir 'ampm.exe') } }
}
function Get-ItemProperty($LiteralPath, $Name, $ErrorAction) {
    Assert ($LiteralPath -like '*EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}') 'WebView registry key'
    if ($fixture.runtime) { [pscustomobject]@{pv='123.0.0.1'} }
}
function Get-AuthenticodeSignature($LiteralPath) {
    [pscustomobject]@{Status=$(if ($fixture.signed) {'Valid'} else {'NotSigned'}); SignerCertificate=[pscustomobject]@{Subject='CN=Microsoft Corporation, O=Microsoft Corporation'}}
}
function Start-Process($FilePath, $ArgumentList, [switch]$Wait, [switch]$PassThru) {
    Assert ($FilePath -like '*MicrosoftEdgeWebview2Setup.exe') 'No app launch with NoStart'
    Assert (($ArgumentList -join ' ') -eq '/silent /install') 'Silent runtime install'
    $fixture.runtime = $true
    $fixture.starts++
    [pscustomobject]@{ExitCode=0}
}
function New-Item($Path, $ItemType, [switch]$Force) {
    if ($Path -like 'HKCU:*') { return }
    Microsoft.PowerShell.Management\New-Item -Path $Path -ItemType $ItemType -Force:$Force
}
function New-ItemProperty($Path, $Name, $Value, $PropertyType, [switch]$Force) {
    Assert ($Path -eq 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\AMPM') 'Only per-user uninstall registration'
    $fixture.properties[$Name] = $Value
}
function New-Object($ComObject) {
    Assert ($ComObject -eq 'WScript.Shell') 'Shortcut COM type'
    $shell = [pscustomobject]@{}
    $shell | Add-Member ScriptMethod CreateShortcut {
        param($Path)
        $link = [pscustomobject]@{TargetPath=''; Arguments=''; WorkingDirectory=''; IconLocation=''}
        $link | Add-Member ScriptMethod Save { $fixture.links += $this }
        return $link
    }
    return $shell
}
function Test-Path($LiteralPath, $Path) {
    $p = if ($LiteralPath) { $LiteralPath } else { $Path }
    if ($p -like 'HKCU:*' -or $p -eq (Join-Path ([Environment]::GetFolderPath('Programs')) 'AMPM.lnk')) { return $false }
    Microsoft.PowerShell.Management\Test-Path -LiteralPath $p
}
function Remove-Item($LiteralPath, [switch]$Force, [switch]$Recurse, $ErrorAction) {
    Assert ($LiteralPath.StartsWith($root)) "Never remove outside test directory: $LiteralPath"
    Microsoft.PowerShell.Management\Remove-Item -LiteralPath $LiteralPath -Force:$Force -Recurse:$Recurse
}
function Run-Install { & $installer -Repository 'fixture/ampm' -ReleaseTag 'v0.1.0' -InstallDir $fixture.installDir -NoStart }

try {
    Run-Install
    $target = Join-Path $fixture.installDir 'ampm.exe'
    $original = (Get-FixtureHash -LiteralPath $target).Hash
    Assert ($original -eq $fixture.checksum.Split(' ')[0]) "First install binary: actual=$original expected=$($fixture.checksum)"
    Assert ($fixture.properties.InstallLocation -eq $fixture.installDir) 'Uninstall registration'
    Assert ($fixture.properties.UninstallString.Contains('-InstallDir "' + $fixture.installDir + '"')) 'Quoted uninstall path'
    Assert ($fixture.links.Count -eq 1 -and $fixture.links[0].TargetPath -eq $target -and $fixture.links[0].Arguments -eq '--open') 'Start Menu shortcut'
    [IO.File]::WriteAllText((Join-Path $fixture.installDir 'user-data.txt'), 'keep me')
    Run-Install
    Assert ((Get-FixtureHash -LiteralPath $target).Hash -eq $original) 'Reinstallation'

    $fixture.corrupt = $true
    Expect-Failure { Run-Install } 'SHA-256 mismatch'
    Assert ((Get-FixtureHash -LiteralPath $target).Hash -eq $original) 'Corrupt download preserves installed file'
    $fixture.corrupt = $false
    $fixture.running = $true
    $count = $fixture.urls.Count
    Expect-Failure { Run-Install } 'AMPM is running'
    Assert ($count -eq $fixture.urls.Count) 'Running app rejected before download'
    $fixture.running = $false
    $lock = [IO.File]::Open($target, 'Open', 'Read', 'None')
    try { Expect-Failure { Run-Install } 'used by another process|being used|另一个进程|正由' }
    finally { $lock.Dispose() }
    Assert ((Get-FixtureHash -LiteralPath $target).Hash -eq $original) 'File lock preserves installed file'

    $fixture.runtime = $false; $fixture.signed = $false
    Expect-Failure { Run-Install } 'Invalid Microsoft'
    Assert ($fixture.starts -eq 0) 'Untrusted runtime never executed'
    $fixture.signed = $true
    Run-Install
    Assert ($fixture.starts -eq 1 -and $fixture.runtime) 'Missing runtime installed'
    Assert (@(Get-ChildItem -LiteralPath $fixture.installDir -Filter '.install-*').Count -eq 0) 'All staging directories cleaned'
    Assert (@($fixture.urls | Where-Object { $_ -notlike 'https://github.com/fixture/ampm/releases/download/v0.1.0/*' -and $_ -notlike 'https://go.microsoft.com/*' }).Count -eq 0) 'Version-pinned assets only'

    & $installer -InstallDir $fixture.installDir -Uninstall
    Assert (-not (Test-Path -LiteralPath $target)) 'Executable removed'
    Assert ((Get-Content -LiteralPath (Join-Path $fixture.installDir 'user-data.txt') -Raw) -eq 'keep me') 'Uninstall preserves other files'
    Write-Host 'PASS: install, reinstall, checksum rejection, running/locked executable, runtime signature, uninstall.'
} finally {
    Microsoft.PowerShell.Management\Remove-Item -LiteralPath $root -Recurse -Force
}
