param(
    [Parameter(Mandatory = $true)][string]$Repository,
    [Parameter(Mandatory = $true)][string]$Tag,
    [string]$Binary = 'target\release\ampm.exe',
    [string]$OutputDir = 'target\dist'
)
$ErrorActionPreference = 'Stop'
if ($Repository -notmatch '^[A-Za-z0-9_-]+/[A-Za-z0-9_.-]+$') { throw 'Invalid repository.' }
$metadata = (& cargo metadata --no-deps --format-version 1 | ConvertFrom-Json)
if ($LASTEXITCODE) { throw 'Cannot read package version.' }
$version = ($metadata.packages | Where-Object name -eq 'ampm').version
if ($Tag -cne "v$version" -or $Tag -notmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') { throw 'Tag must match the stable Cargo.toml version.' }
New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
$asset = Join-Path $OutputDir 'ampm-windows-x64.exe'
Copy-Item -LiteralPath $Binary -Destination $asset -Force
$sha = [Security.Cryptography.SHA256]::Create()
try { $hash = [BitConverter]::ToString($sha.ComputeHash([IO.File]::ReadAllBytes((Resolve-Path $asset)))).Replace('-', '').ToLowerInvariant() }
finally { $sha.Dispose() }
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText([IO.Path]::GetFullPath("$asset.sha256"), "$hash  ampm-windows-x64.exe`n", $utf8)
$template = Get-Content scripts\install.ps1 -Raw
$installer = $template.Replace('__GITHUB_REPOSITORY__', $Repository).Replace('__RELEASE_TAG__', $Tag)
[IO.File]::WriteAllText([IO.Path]::GetFullPath((Join-Path $OutputDir 'install.ps1')), $installer, $utf8)
Write-Output "Release files prepared in $OutputDir. Nothing was uploaded."
