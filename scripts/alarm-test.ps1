# Verifies the alarm end to end, and what it costs in wake-ups:
#
#  - an alarm left over from a session that was closed when it fell due is
#    dropped at startup, silently;
#  - armed behind a mode that is asleep (an idle stopwatch draws nothing), it
#    wakes the overlay on time, rings three times ten seconds apart, and then
#    disarms itself and is written down as off;
#  - a command while it rings silences it after the first chime;
#  - the menu switch arms it at the time last picked and disarms it again.
#
# `chimes=` in `stats` counts the chimes that fell due. A scripted instance
# counts them without playing them, so the run makes no sound and can go on
# while someone uses the machine. An alarm is set to the minute, so the
# script waits for two of them and takes about four minutes.
#
# The window is click-through for the run (`passthrough on`), so a real
# pointer passing over it cannot add frames or silence the alarm.
#
# Runs under its own CHRONODESK_CONFIG and leaves a running overlay alone.
#
# Run after: cargo build --release --features instrument
$ErrorActionPreference = 'Stop'
$s = Split-Path $MyInvocation.MyCommand.Path
$exe = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$portFile = "$env:TEMP\chronodesk-instrument.port"
$scratch = Join-Path $env:TEMP 'chronodesk-alarm-test'
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
function Epoch([datetime]$at) { [DateTimeOffset]::new($at).ToUnixTimeSeconds() }
# The next whole minute at least 40 s away, so there is time to look before it.
function NextMinute {
  $at = (Get-Date).AddSeconds(40)
  $at.Date.AddHours($at.Hour).AddMinutes($at.Minute + 1)
}
# `state` describes the frame drawn last, and a `cmd` takes effect on the next
# one; Settle waits for that frame before asking.
function Settle { Start-Sleep -Milliseconds 500 }
function SleepUntil([datetime]$at) {
  $ms = ($at - (Get-Date)).TotalMilliseconds
  if ($ms -gt 0) { Start-Sleep -Milliseconds ([int]$ms) }
}

$results = @(); $proc = $null
try {
  Remove-Item $portFile -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force $scratch | Out-Null
  # --- A: an alarm that fell due while the overlay was closed ---------------
  $missed = Epoch ((Get-Date).AddMinutes(-2))
  Set-Content -Path $ron -NoNewline -Value "(schema_version:1,first_run:false,mode:`"stopwatch`",alarm_at:`"06:15`",alarm_due:Some($missed))"
  $env:CHRONODESK_CONFIG = $ron
  $proc = Start-Process $exe -ArgumentList '--instrument' -PassThru
  $env:CHRONODESK_CONFIG = $null
  foreach ($i in 1..40) { Start-Sleep -Milliseconds 250; if (Test-Path $portFile) { break } }
  if (-not (Test-Path $portFile)) { throw "no instrument port published" }
  Connect ([int](Get-Content $portFile))
  Send "passthrough on" | Out-Null
  Start-Sleep 1
  $state = Send "state"
  $stats = Send "stats"
  $results += Check "missed while closed: dropped at startup, without a sound" `
    ((Field $state 'alarm_due') -eq 0 -and (Field $stats 'chimes') -eq 0) "$state | $stats"
  $results += Check "missed while closed: written down as off" ((Get-Content $ron -Raw) -match 'alarm_due:\s*None') ""

  # --- B: armed behind a stopwatch that is asleep ----------------------------
  $due = NextMinute
  Send ("cmd alarm:{0:HH:mm}" -f $due) | Out-Null
  Settle
  $state = Send "state"
  $results += Check "armed: for the next time the clock shows it" ((Field $state 'alarm_due') -eq (Epoch $due)) `
    ("{0} vs {1}" -f $state, (Epoch $due))
  $results += Check "armed: written down at once" ((Get-Content $ron -Raw) -match ("alarm_at:\s*`"{0:HH:mm}`"" -f $due)) ""
  Start-Sleep 1
  Send "stats reset" | Out-Null
  SleepUntil $due.AddSeconds(-5)
  $before = Send "stats"
  $results += Check "before it is due: asleep and silent" `
    ((Field $before 'frames') -le 3 -and (Field $before 'chimes') -eq 0) $before
  SleepUntil $due.AddSeconds(5)
  $state = Send "state"
  $results += Check "due: it rings" ((Field $state 'ringing') -eq 1) $state
  SleepUntil $due.AddSeconds(34)
  $after = Send "stats"
  $state = Send "state"
  $results += Check "rung: three chimes, ten seconds apart" ((Field $after 'chimes') -eq 3) $after
  $results += Check "rung: then off, and written down" `
    ((Field $state 'alarm_due') -eq 0 -and (Field $state 'ringing') -eq 0 -and (Get-Content $ron -Raw) -match 'alarm_due:\s*None') $state

  # --- C: silenced by a command ----------------------------------------------
  $due = NextMinute
  Send ("cmd alarm:{0:HH:mm}" -f $due) | Out-Null
  SleepUntil $due.AddSeconds(3)
  Send "cmd mode:stopwatch" | Out-Null
  Start-Sleep 1
  $state = Send "state"
  $results += Check "silenced: a command stops it" `
    ((Field $state 'ringing') -eq 0 -and (Field $state 'alarm_due') -eq 0) $state
  SleepUntil $due.AddSeconds(25)
  $stats = Send "stats"
  $results += Check "silenced: no chime after the first" ((Field $stats 'chimes') -eq 4) $stats

  # --- D: the menu switch ----------------------------------------------------
  Send "cmd alarm" | Out-Null
  Settle
  $on = Send "state"
  Send "cmd alarm" | Out-Null
  Settle
  $off = Send "state"
  $results += Check "switch: arms at the time last picked, and disarms" `
    ((Field $on 'alarm_due') -gt (Epoch (Get-Date)) -and (Field $off 'alarm_due') -eq 0) "$on | $off"
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
