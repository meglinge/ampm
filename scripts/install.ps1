# AMPM per-user installer. Published copies are pinned to their release tag.
# Run downloaded scripts only from a repository you trust.
param(
    [string]$Repository = '__GITHUB_REPOSITORY__',
    [string]$ReleaseTag = '__RELEASE_TAG__',
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\AMPM'),
    [switch]$NoStart,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

function Test-AMPMBinary([string]$Path, [string]$Checksum) {
    if ($Checksum.Trim() -notmatch '^([0-9a-fA-F]{64})\s+ampm-windows-x64\.exe$') { throw 'Invalid AMPM checksum file.' }
    $expected = $Matches[1]
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -gt 64MB -or $bytes.Length -lt 64) { throw 'Invalid AMPM executable size.' }
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $actual = [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-', '') }
    finally { $sha.Dispose() }
    if ($actual -ne $expected) { throw 'SHA-256 mismatch. Installation cancelled.' }
    $pe = [BitConverter]::ToUInt32($bytes, 60)
    if ($bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a -or $pe -gt $bytes.Length - 6 -or
        [BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550 -or [BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664) {
        throw 'The download is not a Windows x64 executable.'
    }
}

function Test-WebView2 {
    $client = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    foreach ($root in @('HKCU:\Software\Microsoft\EdgeUpdate\Clients', 'HKLM:\Software\WOW6432Node\Microsoft\EdgeUpdate\Clients', 'HKLM:\Software\Microsoft\EdgeUpdate\Clients')) {
        $version = (Get-ItemProperty -LiteralPath "$root\$client" -Name pv -ErrorAction SilentlyContinue).pv
        if ($version -and $version -ne '0.0.0.0') { return $true }
    }
    return $false
}

function Install-AMPM {
    if ($env:OS -ne 'Windows_NT' -or -not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -eq 'ARM64' -or $env:PROCESSOR_ARCHITEW6432 -eq 'ARM64') {
        throw 'This installer currently supports Windows x64 only.'
    }
    if (-not [IO.Path]::IsPathRooted($InstallDir)) { throw 'InstallDir must be an absolute path.' }
    $target = Join-Path $InstallDir 'ampm.exe'
    $running = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $target })
    if ($running.Count) { throw 'AMPM is running. Exit it from the tray first, or use Settings > Check for updates.' }
    $shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'AMPM.lnk'
    $registry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\AMPM'
    if ($Uninstall) {
        # Remove only owned files, never the user's config, Runner folders or other files.
        foreach ($file in @('ampm.exe', 'uninstall.ps1')) {
            $owned = Join-Path $InstallDir $file
            if (Test-Path -LiteralPath $owned) { Remove-Item -LiteralPath $owned -Force }
        }
        if (Test-Path -LiteralPath $shortcut) { Remove-Item -LiteralPath $shortcut -Force }
        if (Test-Path -LiteralPath $registry) { Remove-Item -LiteralPath $registry -Recurse }
        Write-Host 'AMPM removed. Runner configuration and project files were preserved.'
        return
    }
    if ($Repository -notmatch '^[A-Za-z0-9_-]+/[A-Za-z0-9_.-]+$' -or $ReleaseTag -notmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
        throw 'Use install.ps1 from a published GitHub Release, or specify Repository and ReleaseTag.'
    }
    $base = "https://github.com/$Repository/releases/download/$ReleaseTag"
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $stage = Join-Path $InstallDir ('.install-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        Write-Host "Downloading AMPM $ReleaseTag..."
        $binary = Join-Path $stage 'ampm.exe'
        Invoke-WebRequest "$base/ampm-windows-x64.exe" -UseBasicParsing -OutFile $binary -TimeoutSec 120
        $checksumFile = Join-Path $stage 'checksum.txt'
        Invoke-WebRequest "$base/ampm-windows-x64.exe.sha256" -UseBasicParsing -OutFile $checksumFile -TimeoutSec 30
        $checksum = [IO.File]::ReadAllText($checksumFile)
        Test-AMPMBinary $binary $checksum
        if (-not (Test-WebView2)) {
            Write-Host 'Installing Microsoft Edge WebView2 Runtime...'
            $runtime = Join-Path $stage 'MicrosoftEdgeWebview2Setup.exe'
            Invoke-WebRequest 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -UseBasicParsing -OutFile $runtime -TimeoutSec 120
            $signature = Get-AuthenticodeSignature -LiteralPath $runtime
            if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') { throw 'Invalid Microsoft WebView2 installer signature.' }
            $process = Start-Process -FilePath $runtime -ArgumentList '/silent', '/install' -Wait -PassThru
            if ($process.ExitCode -ne 0 -or -not (Test-WebView2)) { throw 'WebView2 installation failed. Install it from Microsoft, then retry.' }
        }
        # Download the version-pinned uninstaller from this same release, not a moving branch.
        $uninstaller = Join-Path $stage 'uninstall.ps1'
        Invoke-WebRequest "$base/install.ps1" -UseBasicParsing -OutFile $uninstaller -TimeoutSec 30
        # Same-volume atomic replacement; an in-use executable fails without killing it.
        if (Test-Path -LiteralPath $target) { [IO.File]::Replace($binary, $target, (Join-Path $stage 'previous.exe')) }
        else { [IO.File]::Move($binary, $target) }
        Move-Item -LiteralPath $uninstaller -Destination (Join-Path $InstallDir 'uninstall.ps1') -Force
        $shell = New-Object -ComObject WScript.Shell
        $link = $shell.CreateShortcut($shortcut)
        $link.TargetPath = $target
        $link.Arguments = '--open'
        $link.WorkingDirectory = $InstallDir
        $link.IconLocation = $target
        $link.Save()
        New-Item -Path $registry -Force | Out-Null
        $uninstallCommand = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "' + (Join-Path $InstallDir 'uninstall.ps1') + '" -Uninstall -InstallDir "' + $InstallDir + '"'
        foreach ($entry in @{DisplayName='AMPM'; Publisher=$Repository.Split('/')[0]; InstallLocation=$InstallDir; DisplayIcon=$target; UninstallString=$uninstallCommand}.GetEnumerator()) {
            New-ItemProperty -Path $registry -Name $entry.Key -Value $entry.Value -PropertyType String -Force | Out-Null
        }
        Write-Host "Installed: $target"
        Write-Host 'Automatic updates are enabled. AMPM updates only when its window is hidden and all Runners are stopped.'
        Write-Host 'Amp CLI is separate: install it from ampcode.com and run amp login if needed.'
        if (-not $NoStart) { Start-Process -FilePath $target -ArgumentList '--open' | Out-Null }
    } finally {
        Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Install-AMPM
