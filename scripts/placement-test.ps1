# Verifies the startup placement check (E4): a saved position that belongs to a
# screen which is no longer attached must be recovered onto the primary monitor
# and written back to app.ron.
#
# The work area is read here straight from Windows (SPI_GETWORKAREA) rather than
# taken from the app, so the assertions check the overlay against the OS instead
# of against the app's own arithmetic. CHRONODESK_CONFIG points the run at a
# scratch file, so the config you actually use is never touched.
#
# Run after: cargo build --release --features instrument
$ErrorActionPreference = 'Stop'
$s = Split-Path $MyInvocation.MyCommand.Path
$exe = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$portFile = "$env:TEMP\chronodesk-instrument.port"
$scratch = Join-Path $env:TEMP 'chronodesk-placement-test'
$ron = Join-Path $scratch 'app.ron'

Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class P {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SystemParametersInfoW(uint a, uint b, ref RECT r, uint c);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
"@ -ErrorAction SilentlyContinue
# Without this the rectangles below would come back in virtualised coordinates.
[P]::SetProcessDPIAware() | Out-Null

$client = $null; $reader = $null; $writer = $null
function Connect([int]$port) {
  $script:client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', $port)
  $stream = $client.GetStream()
  $script:reader = New-Object System.IO.StreamReader($stream)
  $script:writer = New-Object System.IO.StreamWriter($stream)
  $script:writer.AutoFlush = $true
}
function Send([string]$line) { $script:writer.WriteLine($line); $script:reader.ReadLine() }
function Rect($proc) {
  $found = [IntPtr]::Zero
  [P]::EnumWindows({ param($h, $l) $other = 0; [P]::GetWindowThreadProcessId($h, [ref]$other) | Out-Null
    if ($other -eq $proc.Id) { $sb = New-Object System.Text.StringBuilder 64; [P]::GetClassName($h, $sb, 64) | Out-Null
      if ($sb.ToString() -eq "Window Class" -and [P]::IsWindowVisible($h)) { $script:found = $h } }
    return $true }, [IntPtr]::Zero) | Out-Null
  $r = New-Object P+RECT; [P]::GetWindowRect($script:found, [ref]$r) | Out-Null; $r
}
function Field($report, $name) {
  if ($report -match "$name=([-0-9.]+)") { [double]$matches[1] } else { $null }
}
function Check($name, $ok, $detail) {
  "{0} {1}{2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $(if ($detail) { " — $detail" } else { '' })
}
function Inside($r, $work) { $r.L -ge $work.L -and $r.T -ge $work.T -and $r.R -le $work.R -and $r.B -le $work.B }

$work = New-Object P+RECT
[P]::SystemParametersInfoW(0x0030, 0, [ref]$work, 0) | Out-Null

$results = @()
try {
  Get-Process chronodesk -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep 1
  Remove-Item $portFile -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force $scratch | Out-Null

  # A position far outside the only attached screen: the case the architecture
  # notes measured for F5, and what a disconnected second monitor leaves behind.
  Set-Content $ron '(schema_version:1,mode:"clock",timer_minutes:25,size:"medium",backdrop:false,chroma:false,show_seconds:true,always_on_top:true,text_outline:true,window:Some((x:7000.0,y:2400.0)))' -NoNewline

  $env:CHRONODESK_CONFIG = $ron
  $proc = Start-Process $exe -ArgumentList '--instrument' -PassThru
  $env:CHRONODESK_CONFIG = $null
  foreach ($i in 1..40) { Start-Sleep -Milliseconds 250; if (Test-Path $portFile) { break } }
  if (-not (Test-Path $portFile)) { throw "no instrument port published" }
  Connect ([int](Get-Content $portFile))
  Start-Sleep 1

  # --- A: the stranded position is recognised and recovered at startup -------
  $report = Send "placement"
  $ppp = Field $report 'ppp'
  $results += Check "startup: the stranded position is detected" ($report -match 'decision=off-screen') $report
  $results += Check "startup: the work area was read per monitor" ($report -match 'work=\(') ""

  $r = Rect $proc
  $w = $r.R - $r.L; $h = $r.B - $r.T
  $results += Check "startup: the overlay is inside the primary work area" (Inside $r $work) `
    ("window ($($r.L),$($r.T))-($($r.R),$($r.B)) vs work ($($work.L),$($work.T))-($($work.R),$($work.B))")
  # The work area already excludes the taskbar, so being inside it is the
  # DPI-independent form of "not overlapping the tray".
  $gapRight = $work.R - $r.R; $gapTop = $r.T - $work.T
  $results += Check "startup: it lands in the top-right corner with a margin" `
    ($gapRight -gt 0 -and $gapRight -le 70 -and $gapTop -gt 0 -and $gapTop -le 70) `
    "right gap $gapRight px, top gap $gapTop px (24 pt scaled by the monitor)"

  # --- B: a valid position is left alone ------------------------------------
  $before = Rect $proc
  Send ("place {0} {1}" -f [int](200 / $ppp), [int](200 / $ppp)) | Out-Null
  Start-Sleep 1
  $keep = Send "placement"
  $after = Rect $proc
  $results += Check "runtime: a position inside the work area is kept" ($keep -match 'decision=keep') $keep
  $results += Check "runtime: keeping does not move the window" `
    ($after.L -eq $before.L -and $after.T -eq $before.T) ""

  # --- C: a position overlapping the taskbar is nudged back in --------------
  $overlapY = $work.B - 30
  Send ("place {0} {1}" -f [int](500 / $ppp), [int]($overlapY / $ppp)) | Out-Null
  Start-Sleep 1
  $nudged = Send "placement"
  $r2 = Rect $proc
  $results += Check "runtime: a window over the taskbar is nudged up" ($nudged -match 'decision=clipped-work-area') $nudged
  $results += Check "runtime: after the nudge it clears the taskbar" (Inside $r2 $work) `
    ("window bottom $($r2.B) vs work bottom $($work.B)")

  # --- D: put it back off-screen, then confirm what reaches the file --------
  Send "place 7000 2400" | Out-Null
  Start-Sleep 1
  $final = Rect $proc
  Send "quit" | Out-Null
  $writer = $null
  Start-Sleep 2

  $text = Get-Content $ron -Raw
  # The file is pretty-printed across several lines, hence the loose whitespace.
  $saved = if ($text -match 'window:\s*Some\(\(\s*x:\s*([-0-9.]+),\s*y:\s*([-0-9.]+)') {
    @{ x = [double]$matches[1]; y = [double]$matches[2] }
  } else { $null }
  $results += Check "config: a position was written back to app.ron" ($null -ne $saved) $text
  if ($saved) {
    $results += Check "config: the stranded position is gone" `
      ($saved.x -ne 7000.0 -and $saved.y -ne 2400.0) "saved ($($saved.x),$($saved.y)) pt"
    $px = @{ x = [int]($saved.x * $ppp); y = [int]($saved.y * $ppp) }
    $results += Check "config: the written position is inside the work area" `
      ($px.x -ge $work.L -and $px.y -ge $work.T -and ($px.x + $w) -le $work.R -and ($px.y + $h) -le $work.B) `
      "saved ($($px.x),$($px.y)) px, overlay ${w}x${h}"
    $results += Check "config: it matches where the window actually ended up" `
      ([Math]::Abs($px.x - $final.L) -le 2 -and [Math]::Abs($px.y - $final.T) -le 2) `
      "file ($($px.x),$($px.y)) vs window ($($final.L),$($final.T))"
  }
}
finally {
  if ($writer) { try { Send "quit" | Out-Null } catch {} }
  if ($client) { $client.Close() }
  $env:CHRONODESK_CONFIG = $null
  Start-Sleep 1
  Get-Process chronodesk -ErrorAction SilentlyContinue | Stop-Process -Force
  Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
  Start-Sleep 1
  # Back to the user's own overlay and their own config file.
  Start-Process $exe
}
$results
"", ("{0} passed, {1} failed" -f ($results | ? { $_ -like 'PASS*' }).Count, ($results | ? { $_ -like 'FAIL*' }).Count)
