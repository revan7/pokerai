# Spec 3.6 fallback: build solver-worker alone with the GNU toolchain.
# Run ONLY when plan 2's V1 check rejects MSVC for the vendored solver
# (build failure, or FLOP-FAST more than 25% slower than the GNU figure in R8 section 5).
# An explicit `+toolchain` on the command line overrides rust-toolchain.toml, so the rest
# of the workspace keeps building with stable-x86_64-pc-windows-msvc.
$ErrorActionPreference = "Stop"

rustup toolchain install stable-x86_64-pc-windows-gnu --no-self-update
cargo +stable-x86_64-pc-windows-gnu build --release -p solver-worker --target x86_64-pc-windows-gnu

$built = "target\x86_64-pc-windows-gnu\release\solver-worker.exe"
if (-not (Test-Path $built)) { throw "GNU build produced no $built" }
New-Item -ItemType Directory -Force -Path "target\release" | Out-Null
Copy-Item $built "target\release\solver-worker.exe" -Force
Write-Host "solver-worker.exe built with stable-x86_64-pc-windows-gnu and staged in target\release"
