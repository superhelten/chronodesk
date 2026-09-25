# Verifies the timer's Pomodoro cycle in the running app:
#
#  - a focus period that runs out chimes and then waits, finished;
#  - Start moves on to the break, at the break's own length;
#  - the break survives the app being killed without notice, still running
#    and still a break;
#  - Reset starts the break over, and a second Reset the whole cycle;
#  - switching the cycle off leaves a plain timer.
#
# Focus periods are one minute long here, which makes the breaks one minute
# too. Chimes are counted, not played. Runs under its own CHRONODESK_CONFIG,
# leaves a running overlay alone, and takes a little over a minute.
#
# Run after: cargo build --release --features instrument
$ErrorActionPreference = 'Stop'
$s = Split-Path $MyInvocation.MyCommand.Path
$exe = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$portFile = "$env:TEMP\chronodesk-instrument.port"
$scratch = Join-Path $env:TEMP 'chronodesk-pomodoro-test'
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
  foreach ($i in 1..50) { $state = Send "state"; if ($state -notlike 'err*') { break }; Start-Sleep -Milliseconds 100 }
}
function Stop([switch]$hard) {
  if ($hard) { $proc.Kill() } else { Send "quit" | Out-Null }
  if (-not $proc.WaitForExit(5000)) { $proc.Kill(); $proc.WaitForExit() }
  $client.Close(); $script:client = $null; $script:writer = $null
}
# `state` reflects the last frame drawn, which can be one command behind.
function Settle { Start-Sleep -Milliseconds 500; Send "state" | Out-Null; Start-Sleep -Milliseconds 200; Send "state" }

$results = @()
try {
  New-Item -ItemType Directory -Force $scratch | Out-Null
  Set-Content $ron '(schema_version:1,first_run:false,mode:"timer",timer_minutes:1,pomodoro:true,window:Some((x:40.0,y:40.0)))' -NoNewline

  # --- A: a focus period runs out and waits ------------------------------------
  Launch
  $state = Settle
  $results += Check "start: first focus period, full length" `
    ($state -match 'pomodoro=1' -and $state -match 'phase=0' -and (Field $state 'cd_remaining_ms') -eq 60000) $state
  Send "cmd startpause" | Out-Null
  Start-Sleep 63
  $state = Settle
  $stats = Send "stats"
  $results += Check "focus: finished, and still the focus period" ($state -match 'cd_finished=1' -and $state -match 'phase=0') $state
  $results += Check "focus: chimed (counted, not played)" ((Field $stats 'chimes') -ge 1) "chimes=$(Field $stats 'chimes')"

  # --- B: Start moves on to the break ------------------------------------------
  Send "cmd startpause" | Out-Null
  $state = Settle
  $remaining = Field $state 'cd_remaining_ms'
  $results += Check "break: Start begins it, running" ($state -match 'phase=1' -and $state -match 'cd_running=1') $state
  $results += Check "break: at its own length" ($remaining -le 60000 -and $remaining -ge 55000) "remaining $remaining ms"
  Start-Sleep 1
  $results += Check "break: its place in the cycle is written" ((Get-Content $ron -Raw) -match 'pomodoro_phase:\s*1') ""

  # --- C: killed without notice, the break comes back ----------------------------
  Stop -hard
  Start-Sleep 3
  Launch
  $state = Settle
  $remaining = Field $state 'cd_remaining_ms'
  $results += Check "restart: still the break, still running" ($state -match 'phase=1' -and $state -match 'cd_running=1') $state
  $results += Check "restart: the time away is taken off" ($remaining -lt 57000 -and $remaining -gt 45000) "remaining $remaining ms"

  # --- D: Reset once for the period, twice for the cycle -------------------------
  Send "cmd reset" | Out-Null
  $state = Settle
  $results += Check "reset: the break starts over" `
    ($state -match 'phase=1' -and $state -match 'cd_running=0' -and (Field $state 'cd_remaining_ms') -eq 60000) $state
  Send "cmd reset" | Out-Null
  $state = Settle
  $results += Check "reset again: the cycle starts over" ($state -match 'phase=0' -and $state -match 'cd_running=0') $state

  # --- E: off again ---------------------------------------------------------------
  Send "cmd pomodoro" | Out-Null
  $state = Settle
  $results += Check "off: a plain timer" ($state -match 'pomodoro=0' -and (Field $state 'cd_remaining_ms') -eq 60000) $state
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
