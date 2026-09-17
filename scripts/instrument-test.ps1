# Drives ChronoDesk through its own instrument channel: no physical mouse, and
# all timings read from inside the process.
$ErrorActionPreference = 'Stop'
# Run after: cargo build --release --features instrument
$s = Split-Path $MyInvocation.MyCommand.Path
$exe = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$ron = "$env:APPDATA\chronodesk\data\app.ron"
$portFile = "$env:TEMP\chronodesk-instrument.port"
$userBackup = "$s\user_app_instr.ron"

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class I {
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
[I]::SetProcessDPIAware() | Out-Null

$client = $null; $stream = $null; $reader = $null; $writer = $null
function Connect([int]$port) {
  $script:client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', $port)
  $script:stream = $client.GetStream()
  $script:reader = New-Object System.IO.StreamReader($stream)
  $script:writer = New-Object System.IO.StreamWriter($stream)
  $script:writer.AutoFlush = $true
}
function Send([string]$line) { $script:writer.WriteLine($line); $script:reader.ReadLine() }
function Rect($proc) {
  $script:ov = [IntPtr]::Zero
  [I]::EnumWindows({ param($h, $l) $pid2 = 0; [I]::GetWindowThreadProcessId($h, [ref]$pid2) | Out-Null
    if ($pid2 -eq $proc.Id) { $sb = New-Object System.Text.StringBuilder 64; [I]::GetClassName($h, $sb, 64) | Out-Null
      if ($sb.ToString() -eq "Window Class" -and [I]::IsWindowVisible($h)) { $script:ov = $h } }
    return $true }, [IntPtr]::Zero) | Out-Null
  $r = New-Object I+RECT; [I]::GetWindowRect($script:ov, [ref]$r) | Out-Null; $r
}
function Shot($name) {
  $r = Rect $proc
  $b = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($r.L, $r.T, 0, 0, $b.Size); $g.Dispose()
  $z = New-Object System.Drawing.Bitmap ($b.Width * 2), ($b.Height * 2)
  $gz = [System.Drawing.Graphics]::FromImage($z); $gz.InterpolationMode = 'NearestNeighbor'
  $gz.DrawImage($b, 0, 0, $z.Width, $z.Height); $gz.Dispose()
  $z.Save("$s\$name.png"); $b.Dispose(); $z.Dispose()
}
function Field($report, $name) {
  if ($report -match "$name=([0-9.]+)") { [double]$matches[1] } else { $null }
}
function Check($name, $ok, $detail) { "{0} {1}{2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $(if ($detail) { " — $detail" } else { '' }) }

Copy-Item $ron $userBackup -ErrorAction SilentlyContinue
$results = @()
try {
  Get-Process chronodesk -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep 1
  Remove-Item $portFile -ErrorAction SilentlyContinue
  # Stopwatch mode so the hover controls exist; placed in a free corner.
  Set-Content $ron '(schema_version:1,mode:"stopwatch",timer_minutes:25,size:"medium",backdrop:false,chroma:false,show_seconds:true,always_on_top:true,text_outline:true,window:Some((x:40.0,y:40.0)))' -NoNewline
  $proc = Start-Process $exe -ArgumentList '--instrument' -PassThru
  foreach ($i in 1..40) { Start-Sleep -Milliseconds 250; if (Test-Path $portFile) { break } }
  if (-not (Test-Path $portFile)) { throw "no instrument port published" }
  $port = [int](Get-Content $portFile)
  Connect $port
  $results += Check "channel: app publishes a port and answers" ((Send "stats") -match 'frames=') "port $port"

  $r = Rect $proc
  $w = $r.R - $r.L; $h = $r.B - $r.T

  # --- A: clock mode, nothing hovered: one frame per second -----------------
  Send "cmd mode:clock" | Out-Null
  Send "hover off" | Out-Null
  Start-Sleep 2
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $clock = Send "stats"
  $results += Check "clock idle: ~1 fps" ((Field $clock 'fps') -ge 0.95 -and (Field $clock 'fps') -le 1.2) $clock
  $results += Check "clock idle: at most one double repaint" ((Field $clock 'short_gaps') -le 2) ("short gaps: " + (Field $clock 'short_gaps'))
  $results += Check "clock idle: every frame is a scheduled tick" ((Field $clock 'tick') -eq (Field $clock 'frames')) ""
  $results += Check "clock idle: ui pass under 2 ms" ((Field $clock 'mean_ms') -lt 2.0) ("mean " + (Field $clock 'mean_ms') + " ms, max " + (Field $clock 'max_ms') + " ms")

  # --- B: stopwatch paused: nothing changes, so nothing is drawn ------------
  Send "cmd mode:stopwatch" | Out-Null
  Start-Sleep 2
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $paused = Send "stats"
  # Frames here can only come from the commands the test itself sends.
  $results += Check "stopwatch paused: no timed frames at all" ((Field $paused 'tick') -eq 0) $paused

  # --- C: synthetic hover shows the controls without spinning ---------------
  Send "hover $([int]($w / 2)) $([int]($h * 0.25))" | Out-Null
  Start-Sleep 1
  Shot "instr_hover"
  Send "stats reset" | Out-Null
  Start-Sleep 8
  $hover = Send "stats"
  $results += Check "hover held: no repaint spin" ((Field $hover 'fps') -le 1.0) $hover

  # --- D: synthetic click on the start button -------------------------------
  $bx = [int](($w / 2) - 14); $by = [int]($h - 16)
  Send "click $bx $by" | Out-Null
  Start-Sleep 2
  Shot "instr_running"
  Send "stats reset" | Out-Null
  Start-Sleep 10
  $running = Send "stats"
  $results += Check "click: stopwatch runs at ~10 fps" ((Field $running 'fps') -ge 9.5 -and (Field $running 'fps') -le 10.5) $running
  $results += Check "running: every frame is a scheduled tick" ((Field $running 'tick') -eq (Field $running 'frames')) ""
  $results += Check "running: ui pass under 2 ms" ((Field $running 'mean_ms') -lt 2.0) ("mean " + (Field $running 'mean_ms') + " ms, max " + (Field $running 'max_ms') + " ms")

  # --- E: pause and release the hover: back to sleeping ---------------------
  Send "cmd startpause" | Out-Null
  Send "hover off" | Out-Null
  Start-Sleep 2
  Shot "instr_released"
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $after = Send "stats"
  $results += Check "after release: asleep again, no timed frames" ((Field $after 'tick') -eq 0) $after

  # --- F: and the clock still ticks afterwards ------------------------------
  Send "cmd mode:clock" | Out-Null
  Start-Sleep 2
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $back = Send "stats"
  $results += Check "clock again: back to ~1 fps" ((Field $back 'fps') -ge 0.95 -and (Field $back 'fps') -le 1.2) $back
}
finally {
  if ($writer) { try { Send "quit" | Out-Null } catch {} }
  if ($client) { $client.Close() }
  Start-Sleep 1
  Get-Process chronodesk -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep 1
  if (Test-Path $userBackup) { Copy-Item $userBackup $ron }
  Start-Process $exe
}
$results
"", ("{0} passed, {1} failed" -f ($results | ? { $_ -like 'PASS*' }).Count, ($results | ? { $_ -like 'FAIL*' }).Count)
