# Builds what gets handed out: dist\ChronoDesk-Setup.exe and its SHA-256.
#
# The setup is the app itself under another name (see src/install.rs), so
# "packaging" is a release build and a copy. It is built into its own target
# directory: a release build, without the `instrument` feature the test
# scripts need in target\release, and not blocked by an overlay that happens
# to be running from there.
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $MyInvocation.MyCommand.Path)
$dist = Join-Path $repo 'dist'
$setup = Join-Path $dist 'ChronoDesk-Setup.exe'

cargo build --release --manifest-path (Join-Path $repo 'Cargo.toml') --target-dir (Join-Path $repo 'target\dist')
if ($LASTEXITCODE -ne 0) { throw "the release build failed" }

New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item (Join-Path $repo 'target\dist\release\chronodesk.exe') $setup -Force
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
Set-Content (Join-Path $dist 'ChronoDesk-Setup.exe.sha256') "$hash  ChronoDesk-Setup.exe" -NoNewline

"{0}  ({1:n2} MB)" -f $setup, ((Get-Item $setup).Length / 1MB)
"sha256  $hash"
