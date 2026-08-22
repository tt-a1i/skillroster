param(
    [Parameter(Mandatory = $true)][string]$Target,
    [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = "Stop"
if ($Target -ne "x86_64-pc-windows-msvc") {
    throw "Unsupported release target: $Target"
}
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$' -or $Version.Contains("..")) {
    throw "Version must be SemVer without build metadata"
}

$Name = "skillroster-$Version-$Target"
$DistRoot = [System.IO.Path]::GetFullPath((Join-Path (Get-Location) "dist"))
$Stage = [System.IO.Path]::GetFullPath((Join-Path $DistRoot $Name))
$Archive = [System.IO.Path]::GetFullPath((Join-Path $DistRoot "$Name.zip"))
$Checksum = "$Archive.sha256"
$Prefix = $DistRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
if (-not $Stage.StartsWith($Prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Staging path escapes dist"
}

New-Item -ItemType Directory -Force -Path $DistRoot | Out-Null
$DistItem = Get-Item -LiteralPath $DistRoot -Force
if (($DistItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "dist must not be a reparse point"
}
if ((Test-Path -LiteralPath $Stage) -or (Test-Path -LiteralPath $Archive) -or (Test-Path -LiteralPath $Checksum)) {
    throw "Refusing to overwrite an existing staging path or artifact"
}
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item "target/$Target/release/skillroster.exe" (Join-Path $Stage "skillroster.exe")
Copy-Item "README.md" (Join-Path $Stage "README.md")
Compress-Archive -Path $Stage -DestinationPath $Archive -Force
Remove-Item $Stage -Recurse -Force

$Hash = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
"$Hash  $Name.zip" | Set-Content -Encoding ASCII "$Archive.sha256"
