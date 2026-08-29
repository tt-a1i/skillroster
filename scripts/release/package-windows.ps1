param(
    [Parameter(Mandatory = $true)][string]$Target,
    [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = "Stop"
$RepoRoot = [System.IO.Path]::GetFullPath((Get-Location).Path)
$ArchiveReadmeRelative = "docs/release-archive/README.md"
$DocsRoot = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot "docs"))
$ExpectedArchiveParent = [System.IO.Path]::GetFullPath(
    (Join-Path $DocsRoot "release-archive")
)
$ArchiveReadme = [System.IO.Path]::GetFullPath(
    (Join-Path $RepoRoot $ArchiveReadmeRelative)
)
if ($Target -ne "x86_64-pc-windows-msvc") {
    throw "Unsupported release target: $Target"
}
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$' -or $Version.Contains("..")) {
    throw "Version must be SemVer without build metadata"
}
foreach ($DirectoryPath in @($RepoRoot, $DocsRoot, $ExpectedArchiveParent)) {
    if (-not (Test-Path -LiteralPath $DirectoryPath -PathType Container)) {
        throw "Release archive README directory is missing: $DirectoryPath"
    }
    $DirectoryItem = Get-Item -LiteralPath $DirectoryPath -Force
    if (($DirectoryItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Release archive README ancestors must not be reparse points: $DirectoryPath"
    }
}
if (-not (Test-Path -LiteralPath $ArchiveReadme -PathType Leaf)) {
    throw "Release archive README is missing: $ArchiveReadme"
}
$ArchiveReadmeItem = Get-Item -LiteralPath $ArchiveReadme -Force
if (($ArchiveReadmeItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Release archive README must not be a reparse point"
}
$ResolvedArchiveParent = [System.IO.Path]::GetFullPath(
    (Resolve-Path -LiteralPath (Split-Path -Parent $ArchiveReadme)).Path
)
if (-not $ResolvedArchiveParent.Equals($ExpectedArchiveParent, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Release archive README parent must not resolve outside its fixed repository path"
}
$ResolvedArchiveReadme = [System.IO.Path]::GetFullPath(
    (Resolve-Path -LiteralPath $ArchiveReadme).Path
)
if (-not $ResolvedArchiveReadme.Equals($ArchiveReadme, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Release archive README must not resolve outside its fixed repository path"
}
$ArchiveReadmeText = [System.IO.File]::ReadAllText($ArchiveReadme)
if ($ArchiveReadmeText -match '(^|[^0-9])[vV]?[0-9]+\.[0-9]+\.[0-9]+([^0-9]|$)') {
    throw "Release archive README must not hard-code a release version"
}

$Name = "skillroster-$Version-$Target"
$DistRoot = [System.IO.Path]::GetFullPath((Join-Path (Get-Location) "dist"))
$Stage = [System.IO.Path]::GetFullPath((Join-Path $DistRoot $Name))
$Archive = [System.IO.Path]::GetFullPath((Join-Path $DistRoot "$Name.zip"))
$Checksum = "$Archive.sha256"
$VerifyRoot = [System.IO.Path]::GetFullPath((Join-Path $DistRoot "$Name-license-check"))
$Prefix = $DistRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
if (-not $Stage.StartsWith($Prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Staging path escapes dist"
}

New-Item -ItemType Directory -Force -Path $DistRoot | Out-Null
$DistItem = Get-Item -LiteralPath $DistRoot -Force
if (($DistItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "dist must not be a reparse point"
}
if ((Test-Path -LiteralPath $Stage) -or (Test-Path -LiteralPath $Archive) -or (Test-Path -LiteralPath $Checksum) -or (Test-Path -LiteralPath $VerifyRoot)) {
    throw "Refusing to overwrite an existing staging path or artifact"
}
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item "target/$Target/release/skillroster.exe" (Join-Path $Stage "skillroster.exe")
Copy-Item $ArchiveReadme (Join-Path $Stage "README.md")
Copy-Item "LICENSE" (Join-Path $Stage "LICENSE")
Compress-Archive -Path $Stage -DestinationPath $Archive -Force
Remove-Item $Stage -Recurse -Force

try {
    Expand-Archive -LiteralPath $Archive -DestinationPath $VerifyRoot
    $BundledLicense = Join-Path (Join-Path $VerifyRoot $Name) "LICENSE"
    $BundledReadme = Join-Path (Join-Path $VerifyRoot $Name) "README.md"
    if (-not (Test-Path -LiteralPath $BundledLicense -PathType Leaf)) {
        throw "Release archive does not contain LICENSE"
    }
    if (-not (Test-Path -LiteralPath $BundledReadme -PathType Leaf)) {
        throw "Release archive does not contain README.md"
    }
    $ExpectedLicenseHash = (Get-FileHash -Algorithm SHA256 "LICENSE").Hash
    $BundledLicenseHash = (Get-FileHash -Algorithm SHA256 $BundledLicense).Hash
    if ($ExpectedLicenseHash -ne $BundledLicenseHash) {
        throw "Release archive LICENSE differs from the repository LICENSE"
    }
    $ExpectedReadmeHash = (Get-FileHash -Algorithm SHA256 $ArchiveReadme).Hash
    $BundledReadmeHash = (Get-FileHash -Algorithm SHA256 $BundledReadme).Hash
    if ($ExpectedReadmeHash -ne $BundledReadmeHash) {
        throw "Release archive README differs from the version-neutral source"
    }
}
finally {
    if (Test-Path -LiteralPath $VerifyRoot) {
        Remove-Item $VerifyRoot -Recurse -Force
    }
}

$Hash = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
$ChecksumLine = "$Hash  $Name.zip`n"
[System.IO.File]::WriteAllText(
    $Checksum,
    $ChecksumLine,
    [System.Text.UTF8Encoding]::new($false)
)
