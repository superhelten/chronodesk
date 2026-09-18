# Verifies the two lifecycle guarantees end to end:
#
#  - one overlay per config file: a second launch against the same app.ron
#    steps aside, while one against another file runs next to it (which is what
#    lets these scripts run while your own overlay is up);
#  - "Start with Windows" writes, recognises and removes a Run entry, honours a
#    Task Manager "disabled" flag, and repoints an entry left by a moved exe.
#
# The registry is read here with PowerShell rather than through the app, so the
# assertions check what Windows will see at login. CHRONODESK_RUN_KEY points the
# run at a scratch key and CHRONODESK_CONFIG at a scratch file: neither your
# real Run key nor your config is touched. Unlike the other scripts this one
# leaves a running overlay alone — coexisting with it is part of the test.
#
# Run after: cargo build --release --features instrument
$ErrorActionPreference = 'Stop'
$s = Split-Path $MyInvocation.MyCommand.Path
$exe = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$portFile = "$env:TEMP\chronodesk-instrument.port"
$scratch = Join-Path $env:TEMP 'chronodesk-lifecycle-test'
$ronA = Join-Path $scratch 'a\app.ron'
$ronB = Join-Path $scratch 'b\app.ron'
$root = "Software\ChronoDesk-lifecycle-test-$PID"
$runKey = "HKCU:\$root\Run"
$approvedKey = "HKCU:\$root\StartupApproved\Run"

$client = $null; $reader = $null; $writer = $null
function Connect([int]$port) {
  $script:client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', $port)
  $stream = $client.GetStream()
  $script:reader = New-Object System.IO.StreamReader($stream)
  $script:writer = New-Object System.IO.StreamWriter($stream)
  $script:writer.AutoFlush = $true
}
function Send([string]$line) { $script:writer.WriteLine($line); $script:reader.ReadLine() }
function Check($name, $ok, $detail) {
  "{0} {1}{2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $(if ($detail) { " — $detail" } else { '' })
}
function Launch($ron, $arguments) {
  $env:CHRONODESK_CONFIG = $ron
  $env:CHRONODESK_RUN_KEY = $root
  try {
    if ($arguments) { Start-Process $exe -ArgumentList $arguments -PassThru } else { Start-Process $exe -PassThru }
  } finally { $env:CHRONODESK_CONFIG = $null; $env:CHRONODESK_RUN_KEY = $null }
}
function RunValue { (Get-ItemProperty $runKey -ErrorAction SilentlyContinue).ChronoDesk }
# The app applies a command on its next frame; give it one.
function Toggle { Send "cmd autostart" | Out-Null; Start-Sleep -Milliseconds 700 }

$results = @(); $started = @()
$realBefore = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run').ChronoDesk
try {
  Remove-Item $portFile -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force (Split-Path $ronA), (Split-Path $ronB) | Out-Null

  $first = Launch $ronA '--instrument'; $started += $first
  foreach ($i in 1..40) { Start-Sleep -Milliseconds 250; if (Test-Path $portFile) { break } }
  if (-not (Test-Path $portFile)) { throw "no instrument port published" }
  Connect ([int](Get-Content $portFile))
  Start-Sleep 1

  # --- A: one overlay per config file ---------------------------------------
  # Spelled differently on purpose: same file, other case and separators.
  $second = Launch ($ronA.ToUpper().Replace('\', '/')) $null; $started += $second
  $gone = $second.WaitForExit(5000)
  $results += Check "instance: a second launch on the same config steps aside" $gone ""
  if ($gone) { $results += Check "instance: it leaves quietly" ($second.ExitCode -eq 0) "exit code $($second.ExitCode)" }
  $results += Check "instance: the first one is unaffected" (-not $first.HasExited -and (Send "stats") -match 'frames=') ""

  $other = Launch $ronB $null; $started += $other
  Start-Sleep 3
  $results += Check "instance: another config file runs alongside" (-not $other.HasExited) ""
  if (-not $other.HasExited) { $other.Kill(); $other.WaitForExit(3000) | Out-Null }

  # --- B: Start with Windows -------------------------------------------------
  $results += Check "autostart: nothing is registered to begin with" ($null -eq (RunValue)) ""
  Toggle
  $results += Check "autostart: switching on writes this exe, quoted" ((RunValue) -eq "`"$exe`"") (RunValue)

  # What Task Manager writes when the entry is switched off there.
  New-Item $approvedKey -Force | Out-Null
  Set-ItemProperty $approvedKey ChronoDesk ([byte[]](3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)) -Type Binary
  # A click flips the check mark the user sees, so let the app notice the veto
  # first (it re-reads the registry at most every 10 s, on the clock's own ticks).
  Start-Sleep 12
  Toggle
  $flag = (Get-ItemProperty $approvedKey -ErrorAction SilentlyContinue).ChronoDesk
  $results += Check "autostart: disabled in Task Manager counts as off, and one click re-enables it" `
    ($null -eq $flag -and (RunValue) -eq "`"$exe`"") "flag '$flag', value $(RunValue)"

  Toggle
  $results += Check "autostart: switching off removes the entry" ($null -eq (RunValue)) (RunValue)

  # An entry left behind by a copy that has since moved.
  New-Item $runKey -Force | Out-Null
  Set-ItemProperty $runKey ChronoDesk '"C:\old place\chronodesk.exe"'
  Start-Sleep 12
  Toggle
  $results += Check "autostart: an entry for a moved exe counts as off, and one click repoints it" `
    ((RunValue) -eq "`"$exe`"") (RunValue)

  $real = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run').ChronoDesk
  $results += Check "autostart: the real Run key was never touched" ($real -eq $realBefore) $real
}
finally {
  if ($writer) { try { Send "quit" | Out-Null } catch {} }
  if ($client) { $client.Close() }
  Start-Sleep 1
  foreach ($p in $started) { if ($p -and -not $p.HasExited) { $p.Kill() } }
  Remove-Item "HKCU:\$root" -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item $portFile -ErrorAction SilentlyContinue
}
$results
"", ("{0} passed, {1} failed" -f ($results | ? { $_ -like 'PASS*' }).Count, ($results | ? { $_ -like 'FAIL*' }).Count)
