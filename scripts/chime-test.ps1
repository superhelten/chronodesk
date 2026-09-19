# Verifies the timer chime end to end, and what it costs in wake-ups:
#
#  - a countdown finishing *behind another mode that is asleep* (a paused
#    stopwatch draws nothing) still chimes: three times, ten seconds apart,
#    then never again — and the app draws only the handful of frames that takes;
#  - with "Sound when finished" off, a finish is silent.
#
# `chimes=` in `stats` counts calls to the system sound, so this passes on a
# muted machine too. It does play for real: expect three notification sounds.
# The shortest timer is a minute, so the script takes about three.
#
# The window is made click-through for the run (`passthrough on`): a fresh
# config opens the overlay at (80, 80), and a real pointer resting or passing
# there would otherwise show up as frames and fail the frame counts.
#
# Like lifecycle-test.ps1 it runs under its own CHRONODESK_CONFIG and leaves a
# running overlay alone.
#
# Run after: cargo build --release --features instrument
$ErrorActionPreference = 'Stop'
$s = Split-Path $MyInvocation.MyCommand.Path
$exe = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$portFile = "$env:TEMP\chronodesk-instrument.port"
$scratch = Join-Path $env:TEMP 'chronodesk-chime-test'
$ron = Join-Path $scratch 'app.ron'

$client = $null; $reader = $null; $writer = $null
function Connect([int]$port) {
  $script:client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', $port)
  $stream = $client.GetStream()
  $script:reader = New-Object System.IO.StreamReader($stream)
  $script:writer = New-Object System.IO.StreamWriter($stream)
  $script:writer.AutoFlush = $true
}
function Send([string]$line) { $script:writer.WriteLine($line); $script:reader.ReadLine() }
function Field($report, $name) { if ($report -match "$name=([-0-9.]+)") { [double]$matches[1] } else { $null } }
function Check($name, $ok, $detail) {
  "{0} {1}{2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $(if ($detail) { " — $detail" } else { '' })
}

$results = @(); $proc = $null
try {
  Remove-Item $portFile -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force $scratch | Out-Null
  $env:CHRONODESK_CONFIG = $ron
  $proc = Start-Process $exe -ArgumentList '--instrument' -PassThru
  $env:CHRONODESK_CONFIG = $null
  foreach ($i in 1..40) { Start-Sleep -Milliseconds 250; if (Test-Path $portFile) { break } }
  if (-not (Test-Path $portFile)) { throw "no instrument port published" }
  Connect ([int](Get-Content $portFile))
  Send "passthrough on" | Out-Null
  Start-Sleep 1

  # --- A: a one-minute countdown, left running behind a paused stopwatch ------
  Send "cmd timer:1" | Out-Null
  Send "cmd startpause" | Out-Null
  $started = Get-Date
  Send "cmd mode:stopwatch" | Out-Null
  Start-Sleep 2
  Send "stats reset" | Out-Null

  Start-Sleep 50
  $before = Send "stats"
  $results += Check "running behind a paused stopwatch: nothing is drawn while it counts" `
    ((Field $before 'tick') -eq 0 -and (Field $before 'frames') -le 3) $before
  $results += Check "running: no chime before zero" ((Field $before 'chimes') -eq 0) ""

  # Past the finish (60 s) and the two repeats (70 s, 80 s).
  $wait = 84 - ((Get-Date) - $started).TotalSeconds
  if ($wait -gt 0) { Start-Sleep -Milliseconds ([int]($wait * 1000)) }
  $after = Send "stats"
  $results += Check "finished: three chimes, ten seconds apart" ((Field $after 'chimes') -eq 3) $after
  # Three alarm frames, the stats replies' own repaints, and slack for a double.
  $results += Check "finished: the alarm cost a handful of frames, not a cadence" ((Field $after 'frames') -le 10) `
    "frames=$(Field $after 'frames') over $(Field $after 'window_s') s"

  Start-Sleep 1
  Send "stats reset" | Out-Null
  Start-Sleep 14
  $quiet = Send "stats"
  $results += Check "afterwards: silent and asleep again" `
    ((Field $quiet 'chimes') -eq 3 -and (Field $quiet 'tick') -eq 0 -and (Field $quiet 'frames') -le 3) $quiet

  # --- B: sound off ----------------------------------------------------------
  Send "cmd timersound" | Out-Null
  Send "cmd mode:timer" | Out-Null
  Send "cmd reset" | Out-Null
  Send "cmd startpause" | Out-Null
  Send "cmd mode:stopwatch" | Out-Null
  Start-Sleep 2
  Send "stats reset" | Out-Null
  Start-Sleep 70
  $silent = Send "stats"
  $results += Check "sound off: a finish is silent" ((Field $silent 'chimes') -eq 3) $silent
  $results += Check "sound off: and schedules no wake of its own" ((Field $silent 'frames') -le 3) $silent
  Send "cmd mode:timer" | Out-Null
  Start-Sleep 1
  Send "quit" | Out-Null
  $writer = $null
  Start-Sleep 2
  $text = Get-Content $ron -Raw
  $results += Check "config: the choice is saved as a flat field" ($text -match 'timer_sound:\s*false') ""
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
