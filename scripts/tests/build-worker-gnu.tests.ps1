# Regression check for scripts/build-worker-gnu.ps1 (R1, task-1-review.md).
#
# Reproduces the exact failure mode the review flagged: a native `cargo` exit
# code of 101 (a real compile failure) with a stale `solver-worker.exe` left
# over from an earlier successful build already present at the GNU target
# path. Before the fix, $ErrorActionPreference = "Stop" did not turn that
# nonzero native exit into a terminating exception, so the script fell
# through to the artifact-existence check, found the stale file, copied it
# into target\release, and printed a success message.
#
# This test stubs `cargo` (exit 101) and `rustup` (exit 0) via a temp
# directory prepended to PATH, runs the real build-worker-gnu.ps1 against a
# temp working directory that already contains a stale GNU-release artifact,
# and asserts that the script throws and that target\release\solver-worker.exe
# is never created/copied.
#
# Run: powershell -NoProfile -File scripts/tests/build-worker-gnu.tests.ps1

$ErrorActionPreference = "Stop"

$scriptUnderTest = Resolve-Path (Join-Path $PSScriptRoot "..\build-worker-gnu.ps1")

$testId = [guid]::NewGuid().ToString("N")
$stubDir = Join-Path $env:TEMP "build-worker-gnu-stubs-$testId"
$workDir = Join-Path $env:TEMP "build-worker-gnu-work-$testId"

New-Item -ItemType Directory -Force -Path $stubDir | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $workDir "target\x86_64-pc-windows-gnu\release") | Out-Null

# Stale artifact from an earlier, successful build — must never be staged
# as the result of a build that just failed.
$staleArtifact = Join-Path $workDir "target\x86_64-pc-windows-gnu\release\solver-worker.exe"
Set-Content -LiteralPath $staleArtifact -Value "stale artifact from a prior successful build" -Encoding ASCII

# Stub cargo: simulates a real native build failure (exit 101).
@'
@echo off
echo cargo stub: simulated native build failure (exit 101)
exit /b 101
'@ | Set-Content -LiteralPath (Join-Path $stubDir "cargo.cmd") -Encoding ASCII

# Stub rustup: simulates a successful toolchain install (exit 0), so the
# failure under test comes from cargo alone.
@'
@echo off
echo rustup stub: simulated success (exit 0)
exit /b 0
'@ | Set-Content -LiteralPath (Join-Path $stubDir "rustup.cmd") -Encoding ASCII

$originalPath = $env:PATH
$originalLocation = Get-Location

$threw = $false
$exceptionMessage = $null

try {
    $env:PATH = "$stubDir;$originalPath"
    Set-Location -LiteralPath $workDir
    try {
        & $scriptUnderTest
    }
    catch {
        $threw = $true
        $exceptionMessage = $_.Exception.Message
    }
}
finally {
    Set-Location -LiteralPath $originalLocation
    $env:PATH = $originalPath
}

$runtimeCopy = Join-Path $workDir "target\release\solver-worker.exe"
$copyExists = Test-Path -LiteralPath $runtimeCopy

Write-Output "Exception thrown: $threw"
if ($threw) { Write-Output "Exception message: $exceptionMessage" }
Write-Output "Runtime copy exists: $copyExists"

Remove-Item -LiteralPath $stubDir -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $workDir -Recurse -Force -ErrorAction SilentlyContinue

if (-not $threw) {
    throw "REGRESSION: build-worker-gnu.ps1 did not throw after a simulated cargo build failure (exit 101) with a pre-existing stale artifact."
}
if ($copyExists) {
    throw "REGRESSION: build-worker-gnu.ps1 copied a stale artifact to target\release despite the simulated cargo build failure."
}

Write-Output "PASS: build-worker-gnu.ps1 throws on cargo failure and does not stage a stale artifact."
