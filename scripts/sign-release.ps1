# Signs a published release, so the app can install it by itself.
#
# The app installs an update only when SHA256SUMS.txt comes with a signature
# made by the key whose public half is built into it (src/update.rs). The
# private key never leaves this machine: it is not in the repository and not
# in CI, so a compromised GitHub account can publish an exe but not a
# signature the app accepts. Without a valid signature the app falls back to
# opening the release page, so a release that was not signed is still safe.
#
#   pwsh -File scripts/sign-release.ps1 -Init          # create the key once; prints the public key
#   pwsh -File scripts/sign-release.ps1 -Tag v0.3.0    # after the release workflow has published
#   pwsh -File scripts/sign-release.ps1 -Message text  # signature of a string, for the test vector
#
# The key is kept DPAPI-protected for the current Windows user. Losing it only
# means the next build carries a new public key; releases signed with the old
# one are then offered as a download instead.
param(
  [switch]$Init,
  [string]$Tag,
  [string]$Message
)
$ErrorActionPreference = 'Stop'
$keyFile = Join-Path $env:USERPROFILE '.chronodesk\release-signing-key.dpapi'

function Hex([byte[]]$bytes) { -join ($bytes | ForEach-Object { $_.ToString('x2') }) }

function Load-Key {
  if (-not (Test-Path $keyFile)) { throw "no signing key at $keyFile; run with -Init first" }
  $pkcs8 = [Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($keyFile), $null, 'CurrentUser')
  $key = [Security.Cryptography.ECDsa]::Create()
  $read = 0
  $key.ImportPkcs8PrivateKey($pkcs8, [ref]$read)
  $key
}

function Public-Hex($key) {
  $q = $key.ExportParameters($false).Q
  Hex ($q.X + $q.Y)
}

# IEEE P1363 (r || s, 64 bytes) over SHA-256: what BCryptVerifySignature takes.
function Sign([byte[]]$data) {
  $key = Load-Key
  Hex $key.SignData($data, [Security.Cryptography.HashAlgorithmName]::SHA256)
}

if ($Init) {
  if (Test-Path $keyFile) {
    "a key exists already: $keyFile"
  } else {
    New-Item -ItemType Directory -Force (Split-Path $keyFile) | Out-Null
    $key = [Security.Cryptography.ECDsa]::Create([Security.Cryptography.ECCurve+NamedCurves]::nistP256)
    $sealed = [Security.Cryptography.ProtectedData]::Protect($key.ExportPkcs8PrivateKey(), $null, 'CurrentUser')
    [IO.File]::WriteAllBytes($keyFile, $sealed)
    "created $keyFile"
  }
  "public key (X || Y) for src/update.rs:"
  Public-Hex (Load-Key)
  return
}

if ($Message) {
  Sign ([Text.Encoding]::UTF8.GetBytes($Message))
  return
}

if (-not $Tag) { throw "give -Init, -Tag vX.Y.Z or -Message text" }

$work = Join-Path $env:TEMP "chronodesk-sign-$Tag"
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory $work | Out-Null
try {
  gh release download $Tag -R superhelten/chronodesk -D $work -p 'SHA256SUMS.txt' -p 'ChronoDesk-Setup.exe' -p 'ChronoDesk.exe'
  if ($LASTEXITCODE -ne 0) { throw "could not download the assets of $Tag" }
  $sums = Join-Path $work 'SHA256SUMS.txt'
  # Only sign what was actually published: every listed file must match.
  foreach ($line in Get-Content $sums) {
    $hash, $name = $line -split '\s+', 2
    $actual = (Get-FileHash (Join-Path $work $name) -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $hash) { throw "$name does not match SHA256SUMS.txt" }
  }
  $signature = Join-Path $work 'SHA256SUMS.txt.sig'
  [IO.File]::WriteAllText($signature, (Sign ([IO.File]::ReadAllBytes($sums))) + "`n")
  gh release upload $Tag $signature -R superhelten/chronodesk --clobber
  if ($LASTEXITCODE -ne 0) { throw "could not upload the signature" }
  "signed $Tag"
}
finally {
  Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
