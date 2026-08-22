param(
    [Parameter(Mandatory = $true)][string]$Binary
)

$ErrorActionPreference = "Stop"
$Binary = [System.IO.Path]::GetFullPath($Binary)
if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    throw "Release binary does not exist: $Binary"
}

$Fixture = Join-Path ([System.IO.Path]::GetTempPath()) "skillroster-release-smoke-$PID-$([Guid]::NewGuid().ToString('N'))"
$HomeRoot = Join-Path $Fixture "home"
$StateRoot = Join-Path $Fixture "state"
$SkillRoot = Join-Path $HomeRoot ".codex/skills"
$Common = @("--home", $HomeRoot, "--state-dir", $StateRoot, "--json")

function Invoke-SkillRosterJson {
    param([Parameter(Mandatory = $true)][string[]]$CommandArgs)

    $Output = & $Binary @CommandArgs
    if ($LASTEXITCODE -ne 0) {
        throw "SkillRoster exited $LASTEXITCODE`: $Output"
    }
    $Document = $Output | ConvertFrom-Json
    if (-not $Document.ok -or $Document.schema_version -ne 1) {
        throw "SkillRoster returned an invalid Agent envelope: $Output"
    }
    return $Document
}

try {
    New-Item -ItemType Directory -Force -Path $SkillRoot | Out-Null
    $Scan = Invoke-SkillRosterJson -CommandArgs ($Common + @("scan"))
    if ($Scan.result.skill_count -ne 0) {
        throw "Synthetic home must start with zero Skills"
    }

    $Setup = Invoke-SkillRosterJson -CommandArgs ($Common + @("setup"))
    if ($null -eq $Setup.result.plan_id -or $Setup.result.state -ne "preview_ready") {
        throw "Setup did not produce a preview Plan"
    }

    $Apply = Invoke-SkillRosterJson -CommandArgs ($Common + @("apply", $Setup.result.plan_id))
    $Bootstrap = Join-Path $SkillRoot "skillroster/SKILL.md"
    if ($Apply.result.verification -ne "passed" -or -not (Test-Path -LiteralPath $Bootstrap -PathType Leaf)) {
        throw "Release Apply did not verify the bootstrap Skill"
    }

    $Undo = Invoke-SkillRosterJson -CommandArgs ($Common + @("undo", $Apply.result.receipt_id))
    if ($Undo.result.verification -ne "passed" -or (Test-Path -LiteralPath $Bootstrap)) {
        throw "Release Undo did not restore the synthetic Agent root"
    }

    $Status = Invoke-SkillRosterJson -CommandArgs ($Common + @("status"))
    if ($Status.result.recovery_state -ne "clear") {
        throw "Release smoke left recovery required"
    }
    Write-Output "release governance smoke passed"
}
finally {
    if (Test-Path -LiteralPath $Fixture) {
        Remove-Item -LiteralPath $Fixture -Recurse -Force
    }
}
