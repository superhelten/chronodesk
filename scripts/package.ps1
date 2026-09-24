# Builds what gets handed out: dist\ChronoDesk-Setup.exe, dist\ChronoDesk.exe
# and dist\SHA256SUMS.txt.
#
# The two exes are the same file. Under a name containing "setup" it installs
# itself; under any other name it runs in place, which is the portable download.
#
# So "packaging" is a release build and two copies (see src/install.rs). It is built into its own target
# directory: a release build, without the `instrument` feature the test
# scripts need in target\release, and not blocked by an overlay that happens
# to be running from there.
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $MyInvocation.MyCommand.Path)
$dist = Join-Path $repo 'dist'
$setup = Join-Path $dist 'ChronoDesk-Setup.exe'

cargo build --release --locked --manifest-path (Join-Path $repo 'Cargo.toml') --target-dir (Join-Path $repo 'target\dist')
if ($LASTEXITCODE -ne 0) { throw "the release build failed" }

New-Item -ItemType Directory -Force $dist | Out-Null
Remove-Item (Join-Path $dist '*.sha256') -ErrorAction SilentlyContinue
$sums = foreach ($name in 'ChronoDesk-Setup.exe', 'ChronoDesk.exe') {
  $file = Join-Path $dist $name
  Copy-Item (Join-Path $repo 'target\dist\release\chronodesk.exe') $file -Force
  "{0}  {1}" -f (Get-FileHash $file -Algorithm SHA256).Hash.ToLower(), $name
}
# Plain LF line endings, so `sha256sum -c` reads every line.
[IO.File]::WriteAllText((Join-Path $dist 'SHA256SUMS.txt'), ($sums -join "`n") + "`n")

$sums
"{0:n2} MB each" -f ((Get-Item $setup).Length / 1MB)
