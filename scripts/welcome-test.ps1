# Verifies the first-run welcome:
#
#  - a launch that finds no config file shows the card, and the card costs no
#    frames while it waits to be read;
#  - a click dismisses it, and that is written to app.ron at once;
#  - the next launch goes straight to the clock;
#  - "Quick tips" in the menu brings it back, and any menu choice both
#    dismisses it and does what it says;
#  - a config file from before the field existed belongs to someone who knows
#    the app: no card.
#
# Runs under its own CHRONODESK_CONFIG and leaves a running overlay alone; the
# window is click-through for the run, so a real pointer cannot dismiss the
# card by accident. Leaves scripts/instr_welcome.png behind for a look.
#
# Run after: cargo build --release --features instrument
$ErrorActionPreference = 'Stop'
$s = Split-Path $MyInvocation.MyCommand.Path
$exe = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$portFile = "$env:TEMP\chronodesk-instrument.port"
$scratch = Join-Path $env:TEMP 'chronodesk-welcome-test'
$ron = Join-Path $scratch 'app.ron'

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
"@ -ErrorAction SilentlyContinue
[W]::SetProcessDPIAware() | Out-Null

$client = $null; $reader = $null; $writer = $null; $proc = $null
function Send([string]$line) { $script:writer.WriteLine($line); $script:reader.ReadLine() }
function Field($report, $name) { if ($report -match "$name=([-0-9.]+)") { [double]$matches[1] } else { $null } }
function Check($name, $ok, $detail) {
  "{0} {1}{2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $(if ($detail) { " — $detail" } else { '' })
}
function WaitState([string]$pattern, [int]$timeoutMs = 5000) {
  $deadline = (Get-Date).AddMilliseconds($timeoutMs)
  do { $state = Send "state"; if ($state -match $pattern) { break }; Start-Sleep -Milliseconds 100 } while ((Get-Date) -lt $deadline)
  $state
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
  WaitState 'welcome=' | Out-Null
}
function Quit {
  Send "quit" | Out-Null
  if (-not $proc.WaitForExit(5000)) { $proc.Kill(); $proc.WaitForExit() }
  $client.Close(); $script:client = $null; $script:writer = $null
}
function Shot($name) {
  $script:ov = [IntPtr]::Zero
  [W]::EnumWindows({ param($h, $l) $pid2 = 0; [W]::GetWindowThreadProcessId($h, [ref]$pid2) | Out-Null
    if ($pid2 -eq $proc.Id) { $sb = New-Object System.Text.StringBuilder 64; [W]::GetClassName($h, $sb, 64) | Out-Null
      if ($sb.ToString() -eq "Window Class" -and [W]::IsWindowVisible($h)) { $script:ov = $h } }
    return $true }, [IntPtr]::Zero) | Out-Null
  $r = New-Object W+RECT; [W]::GetWindowRect($script:ov, [ref]$r) | Out-Null
  $b = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($r.L, $r.T, 0, 0, $b.Size); $g.Dispose()
  $b.Save("$s\$name.png"); $b.Dispose()
}

$results = @()
try {
  Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force $scratch | Out-Null

  # --- A: no config file at all ------------------------------------------------------
  Launch
  $state = Send "state"
  $results += Check "first launch: the welcome card is up" ($state -match 'welcome=1') $state
  $results += Check "first launch: it is a card, not a clock-sized window" ((Field $state 'win_w') -ge 300 -and (Field $state 'win_h') -ge 150) ("{0} x {1} pt" -f (Field $state 'win_w'), (Field $state 'win_h'))
  Start-Sleep 2
  Shot "instr_welcome"
  Send "stats reset" | Out-Null
  Start-Sleep 8
  $idle = Send "stats"
  $results += Check "first launch: the card costs no frames while it waits" ((Field $idle 'tick') -eq 0 -and (Field $idle 'frames') -le 3) $idle

  # --- B: a click anywhere on it ---------------------------------------------------------
  Send ("click {0} {1}" -f [int]((Field $state 'win_w') / 2), [int]((Field $state 'win_h') / 2)) | Out-Null
  $state = WaitState 'welcome=0'
  $results += Check "click: the card gives way to the clock" ($state -match 'welcome=0' -and (Field $state 'win_w') -lt 300) $state
  Send "hover off" | Out-Null
  $text = if (Test-Path $ron) { Get-Content $ron -Raw } else { '' }
  $results += Check "click: and that is on disk at once" ($text -match 'first_run:\s*false') ""
  Quit

  # --- C: the next launch ---------------------------------------------------------------------
  Launch
  $state = Send "state"
  $results += Check "second launch: straight to the clock" ($state -match 'welcome=0') $state

  # --- D: asked for from the menu ---------------------------------------------------------------
  Send "cmd welcome" | Out-Null
  $state = WaitState 'welcome=1'
  $results += Check "Quick tips: brings the card back" ($state -match 'welcome=1') $state
  Send "cmd mode:timer" | Out-Null
  $state = WaitState 'welcome=0'
  $results += Check "a menu choice: dismisses the card and does what it says" ($state -match 'welcome=0' -and $state -match 'mode=timer') $state
  Quit

  # --- E: a config file from before the field existed ----------------------------------------------
  Set-Content $ron '(schema_version:1,mode:"clock",window:Some((x:40.0,y:40.0)))' -NoNewline
  Launch
  $state = Send "state"
  $results += Check "an existing user: no card after an upgrade" ($state -match 'welcome=0') $state
  Quit
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
