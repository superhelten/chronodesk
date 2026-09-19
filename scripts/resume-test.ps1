# Verifies that a stopwatch or countdown under way survives the process:
#
#  - a running stopwatch is killed without notice (what a power cut or a forced
#    logout looks like) and comes back running, with the time it was away on it;
#  - a paused one comes back paused, to the millisecond;
#  - a running countdown comes back with the time away taken off;
#  - one that ran out while nothing was running comes back finished and silent;
#  - a reset leaves nothing behind to restore.
#
# The counters are read back through the instrument channel (`state`), and the
# file is checked too, so a pass means both "it was written" and "it was used".
#
# Like chime-test.ps1 it runs under its own CHRONODESK_CONFIG and leaves a
# running overlay alone. Takes about a minute and makes no sound.
#
# Run after: cargo build --release --features instrument
$ErrorActionPreference = 'Stop'
$s = Split-Path $MyInvocation.MyCommand.Path
$exe = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$portFile = "$env:TEMP\chronodesk-instrument.port"
$scratch = Join-Path $env:TEMP 'chronodesk-resume-test'
$ron = Join-Path $scratch 'app.ron'

$client = $null; $reader = $null; $writer = $null; $proc = $null
function Send([string]$line) { $script:writer.WriteLine($line); $script:reader.ReadLine() }
function Field($report, $name) { if ($report -match "$name=([-0-9.]+)") { [double]$matches[1] } else { $null } }
function Check($name, $ok, $detail) {
  "{0} {1}{2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $(if ($detail) { " — $detail" } else { '' })
}
function Launch {
  Remove-Item $portFile -ErrorAction SilentlyContinue
  $env:CHRONODESK_CONFIG = $ron
  $script:proc = Start-Process $exe -ArgumentList '--instrument' -PassThru
  $env:CHRONODESK_CONFIG = $null
  foreach ($i in 1..40) { Start-Sleep -Milliseconds 250; if (Test-Path $portFile) { break } }
  if (-not (Test-Path $portFile)) { throw "no instrument port published" }
  $script:client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', [int](Get-Content $portFile))
  $stream = $client.GetStream()
  $script:reader = New-Object System.IO.StreamReader($stream)
  $script:writer = New-Object System.IO.StreamWriter($stream)
  $script:writer.AutoFlush = $true
  Send "passthrough on" | Out-Null
  # The first frame has to have been drawn before there is a state to read.
  foreach ($i in 1..50) { $state = Send "state"; if ($state -notlike 'err*') { break }; Start-Sleep -Milliseconds 100 }
}
# $hard: no notice at all. Otherwise the window is asked to close.
function Stop([switch]$hard) {
  if ($hard) { $proc.Kill() } else { Send "quit" | Out-Null }
  if (-not $proc.WaitForExit(5000)) { $proc.Kill(); $proc.WaitForExit() }
  $client.Close(); $script:client = $null; $script:writer = $null
}
# A reading of the app's counters, with the moment it was taken.
function Read-State { $state = Send "state"; [pscustomobject]@{ Text = $state; At = Get-Date } }
function Away($before, $after) { ($after.At - $before.At).TotalMilliseconds }

$results = @()
try {
  New-Item -ItemType Directory -Force $scratch | Out-Null
  Set-Content $ron '(schema_version:1,mode:"stopwatch",window:Some((x:40.0,y:40.0)))' -NoNewline

  # --- A: a running stopwatch, killed without notice ---------------------------
  Launch
  Send "cmd startpause" | Out-Null
  Start-Sleep 3
  $before = Read-State
  $text = Get-Content $ron -Raw
  $results += Check "stopwatch: written the moment it starts, as a wall-clock anchor" ($text -match 'stopwatch:\s*Some\(\s*\(' -and $text -match 'started_at_ms:\s*Some\(\d+\)') ""
  Stop -hard
  Start-Sleep 5
  Launch
  $after = Read-State
  $expected = (Field $before.Text 'sw_ms') + (Away $before $after)
  $drift = [math]::Abs((Field $after.Text 'sw_ms') - $expected)
  $results += Check "stopwatch: comes back running" ($after.Text -match 'sw_running=1') $after.Text
  $results += Check "stopwatch: the time away is on it" ($drift -le 1500) ("expected ~{0:n0} ms, got {1:n0} ms" -f $expected, (Field $after.Text 'sw_ms'))

  # --- B: paused -----------------------------------------------------------------
  Send "cmd startpause" | Out-Null
  Start-Sleep 1
  $paused = Read-State
  Stop
  Start-Sleep 3
  Launch
  $after = Read-State
  $results += Check "paused stopwatch: comes back paused, to the millisecond" `
    ($after.Text -match 'sw_running=0' -and (Field $after.Text 'sw_ms') -eq (Field $paused.Text 'sw_ms') -and (Field $paused.Text 'sw_ms') -gt 0) `
    ("{0} -> {1}" -f (Field $paused.Text 'sw_ms'), (Field $after.Text 'sw_ms'))

  # --- C: a running countdown ------------------------------------------------------
  Send "cmd timer:1" | Out-Null
  Send "cmd startpause" | Out-Null
  Start-Sleep 2
  $before = Read-State
  Stop -hard
  Start-Sleep 5
  Launch
  $after = Read-State
  $expected = (Field $before.Text 'cd_remaining_ms') - (Away $before $after)
  $drift = [math]::Abs((Field $after.Text 'cd_remaining_ms') - $expected)
  $results += Check "countdown: comes back running, in timer mode" ($after.Text -match 'cd_running=1' -and $after.Text -match 'mode=timer') $after.Text
  $results += Check "countdown: the time away is taken off" ($drift -le 1500) ("expected ~{0:n0} ms left, got {1:n0} ms" -f $expected, (Field $after.Text 'cd_remaining_ms'))
  $results += Check "countdown: and the paused stopwatch beside it is untouched" ((Field $after.Text 'sw_ms') -eq (Field $paused.Text 'sw_ms')) ""
  Stop

  # --- D: it ran out while nothing was running -------------------------------------
  # Started ten minutes ago, for one minute: long past the half minute in which
  # a finish is announced, so it must come back finished and stay quiet.
  $anchor = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - 600000
  Set-Content $ron "(schema_version:1,mode:`"timer`",timer_minutes:1,countdown:Some((accumulated_ms:0,started_at_ms:Some($anchor))),window:Some((x:40.0,y:40.0)))" -NoNewline
  Launch
  Start-Sleep 2
  $after = Read-State
  $stats = Send "stats"
  $results += Check "ran out while away: comes back finished" ($after.Text -match 'cd_finished=1' -and (Field $after.Text 'cd_remaining_ms') -eq 0) $after.Text
  $results += Check "ran out while away: no chime for a finish long past" ((Field $stats 'chimes') -eq 0) ""

  # --- E: reset leaves nothing to restore ---------------------------------------------
  Send "cmd reset" | Out-Null
  Start-Sleep 1
  $text = Get-Content $ron -Raw
  $results += Check "reset: nothing is left in the file" ($text -match 'countdown:\s*None' -and $text -match 'stopwatch:\s*None') ""
  Stop
  Launch
  $after = Read-State
  $results += Check "reset: and the next launch starts idle" ($after.Text -match 'cd_running=0' -and $after.Text -match 'cd_finished=0' -and (Field $after.Text 'cd_remaining_ms') -eq 60000) $after.Text
  Stop
}
finally {
  if ($writer) { try { Send "quit" | Out-Null } catch {} }
  if ($client) { $client.Close() }
  Start-Sleep 1
  if ($proc -and -not $proc.HasExited) { $proc.Kill() }
  Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item $portFile -ErrorAction SilentlyContinue
}
$results
"", ("{0} passed, {1} failed" -f ($results | ? { $_ -like 'PASS*' }).Count, ($results | ? { $_ -like 'FAIL*' }).Count)
