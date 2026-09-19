# Drives ChronoDesk through its own instrument channel: no physical mouse, and
# all timings read from inside the process.
#
# The window is made click-through for the run (`passthrough on`), so a real
# pointer resting on or crossing it cannot disturb the synthetic one or add
# frames to a measurement. Where the script needs to know that something has
# happened it asks the app (`state`) instead of sleeping and hoping.
#
# Like the other scripts it runs under its own CHRONODESK_CONFIG and leaves a
# running overlay, and its config, alone.
$ErrorActionPreference = 'Stop'
# Run after: cargo build --release --features instrument
$s = Split-Path $MyInvocation.MyCommand.Path
$exe = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$scratch = Join-Path $env:TEMP 'chronodesk-instrument-test'
$ron = Join-Path $scratch 'app.ron'
$portFile = "$env:TEMP\chronodesk-instrument.port"

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
# The coordinates are passed on as the app printed them, never through a
# culture-dependent number format.
function Text($report, $name) { if ($report -match "$name=(\S+)") { $matches[1] } else { $null } }
# Asks until the app reports what is expected; the last answer either way.
function WaitState([string]$pattern, [int]$timeoutMs = 5000) {
  $deadline = (Get-Date).AddMilliseconds($timeoutMs)
  do { $state = Send "state"; if ($state -match $pattern) { break }; Start-Sleep -Milliseconds 100 } while ((Get-Date) -lt $deadline)
  $state
}
# Every reply queues a repaint, so two reads a moment apart must show the frame
# count moving. If it does not, the app has stopped drawing altogether, which
# is what a minimised window does (measured: no frames, no commands applied,
# until it is restored). A test instance has no minimise style bit, no tray
# icon and never takes the focus, so this should not happen any more; if it
# does, nothing measured afterwards means anything, and the run stops with one
# line instead of a page of failures.
function Assert-Drawing([string]$where) {
  $a = Field (Send "stats") 'frames'; Start-Sleep -Milliseconds 400; $b = Field (Send "stats") 'frames'
  if ($b -le $a) { throw "the app stopped drawing before '$where' (is its window minimised?) - not a test failure; rerun" }
}

$results = @(); $proc = $null
try {
  Remove-Item $portFile -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force $scratch | Out-Null
  # Stopwatch mode so the hover controls exist; placed in a free corner.
  Set-Content $ron '(schema_version:1,mode:"stopwatch",timer_minutes:25,size:"medium",backdrop:false,chroma:false,show_seconds:true,always_on_top:true,text_outline:true,window:Some((x:40.0,y:40.0)))' -NoNewline
  $env:CHRONODESK_CONFIG = $ron
  $proc = Start-Process $exe -ArgumentList '--instrument' -PassThru
  $env:CHRONODESK_CONFIG = $null
  foreach ($i in 1..40) { Start-Sleep -Milliseconds 250; if (Test-Path $portFile) { break } }
  if (-not (Test-Path $portFile)) { throw "no instrument port published" }
  $port = [int](Get-Content $portFile)
  Connect $port
  $results += Check "channel: app publishes a port and answers" ((Send "stats") -match 'frames=') "port $port"
  Send "passthrough on" | Out-Null

  # --- A: clock mode, nothing hovered: one frame per second -----------------
  Assert-Drawing 'A: clock idle'
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
  Assert-Drawing 'C: hover'
  # The window's size in points comes from the app: the desktop's pixels are
  # only the same thing at 100 % scaling.
  $state = WaitState 'win_w='
  $hx = [int]([double](Field $state 'win_w') / 2); $hy = [int]([double](Field $state 'win_h') / 4)
  Send "hover $hx $hy" | Out-Null
  $state = WaitState 'controls=1'
  $results += Check "hover: the controls are drawn" ($state -match 'controls=1') $state
  Start-Sleep 1
  Shot "instr_hover"
  Send "stats reset" | Out-Null
  Start-Sleep 8
  $hover = Send "stats"
  $results += Check "hover held: no repaint spin" ((Field $hover 'fps') -le 1.0) $hover

  # --- D: synthetic click on the start button -------------------------------
  # Aimed at the centre of the button as the app drew it, and judged by what
  # the stopwatch then says about itself; the frame rate is a separate matter.
  Send "click $(Text $state 'start_x') $(Text $state 'start_y')" | Out-Null
  $state = WaitState 'sw_running=1'
  $results += Check "click: the start button starts the stopwatch" ($state -match 'sw_running=1') $state
  Start-Sleep 2
  Shot "instr_running"
  Send "stats reset" | Out-Null
  Start-Sleep 10
  $running = Send "stats"
  $results += Check "running: stopwatch draws at ~10 fps" ((Field $running 'fps') -ge 9.5 -and (Field $running 'fps') -le 10.5) $running
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

  # --- G: appearance changes never re-measure text ---------------------------
  Assert-Drawing 'G: appearance'
  # Colours, night mode, 12-hour clock and the date line are all outside the
  # layout key, so the rebuild count must not move. Size is the one thing
  # that does rebuild, and it is checked separately to prove the counter works.
  $before = Field (Send "stats") 'rebuilds'
  foreach ($c in 'palette:warm', 'palette:cool', 'palette:amber', 'night:on', 'night:auto', '12h', 'date') {
    Send "cmd $c" | Out-Null
    Start-Sleep -Milliseconds 300
  }
  Start-Sleep 2
  Shot "instr_appearance"
  $after = Field (Send "stats") 'rebuilds'
  $results += Check "appearance: no layout rebuilds" ($before -eq $after) "rebuilds $before -> $after"
  # Every reply queues a repaint; let the one from the read above land
  # before the reset, or it is counted as a frame in the window.
  Start-Sleep 1
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $clock12 = Send "stats"
  $results += Check "appearance: 12h clock still ~1 fps" ((Field $clock12 'fps') -ge 0.95 -and (Field $clock12 'fps') -le 1.2) $clock12
  Send "cmd size:large" | Out-Null
  Start-Sleep 1
  $large = Field (Send "stats") 'rebuilds'
  $results += Check "size change: exactly one rebuild" ($large -eq ($after + 1)) "rebuilds $after -> $large"
  Send "cmd size:medium" | Out-Null
  Start-Sleep 1

  # --- I: the digital face is one rebuild, then as cheap as the typeface ------
  foreach ($c in 'night:off', 'palette:default', '12h', 'date') { Send "cmd $c" | Out-Null }
  Start-Sleep 1
  $beforeFont = Field (Send "stats") 'rebuilds'
  Send "cmd digital" | Out-Null
  Start-Sleep 2
  Shot "instr_digital"
  $digitalOn = Field (Send "stats") 'rebuilds'
  $results += Check "digital font: exactly one rebuild" ($digitalOn -eq ($beforeFont + 1)) "rebuilds $beforeFont -> $digitalOn"
  # The small size is where the segment gaps approach a pixel; keep a shot
  # of each size for inspection.
  Send "cmd size:small" | Out-Null
  Start-Sleep 1
  Shot "instr_digital_small"
  Send "cmd size:large" | Out-Null
  Start-Sleep 1
  Shot "instr_digital_large"
  Send "cmd size:medium" | Out-Null
  Start-Sleep 1
  $digitalOn = Field (Send "stats") 'rebuilds'
  Start-Sleep 1
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $digitalClock = Send "stats"
  $results += Check "digital clock idle: ~1 fps" ((Field $digitalClock 'fps') -ge 0.95 -and (Field $digitalClock 'fps') -le 1.2) $digitalClock
  $results += Check "digital clock idle: ui pass under 2 ms" ((Field $digitalClock 'mean_ms') -lt 2.0) ("mean " + (Field $digitalClock 'mean_ms') + " ms, max " + (Field $digitalClock 'max_ms') + " ms")
  Send "cmd backdrop" | Out-Null
  Start-Sleep 1
  Shot "instr_digital_backdrop"
  Send "cmd backdrop" | Out-Null
  Send "cmd mode:stopwatch" | Out-Null
  Start-Sleep 2
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $digitalIdle = Send "stats"
  $results += Check "digital, stopwatch paused: no timed frames" ((Field $digitalIdle 'tick') -eq 0) $digitalIdle
  Send "cmd digital" | Out-Null
  Start-Sleep 1
  $digitalOff = Field (Send "stats") 'rebuilds'
  $results += Check "typeface again: exactly one rebuild" ($digitalOff -eq ($digitalOn + 1)) "rebuilds $digitalOn -> $digitalOff"
  Send "cmd mode:clock" | Out-Null

  # --- H: night mode auto keeps an idle stopwatch asleep ---------------------
  # The next schedule boundary is hours away, so the only wake-up it adds
  # must be that far out: no timed frames in a 12 s window.
  Send "cmd mode:stopwatch" | Out-Null
  Send "cmd night:auto" | Out-Null
  Start-Sleep 2
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $nightIdle = Send "stats"
  $results += Check "night auto, stopwatch paused: no timed frames" ((Field $nightIdle 'tick') -eq 0) $nightIdle

  # --- J: the market board ----------------------------------------------------
  Assert-Drawing 'J: market board'
  # Mode is outside the layout key (both faces are measured on every rebuild),
  # so switching to the board and back must not re-measure anything.
  foreach ($c in 'night:off', 'mode:clock') { Send "cmd $c" | Out-Null }
  Start-Sleep 1
  $beforeBoard = Field (Send "stats") 'rebuilds'
  Send "cmd mode:market" | Out-Null
  Start-Sleep 2
  Shot "instr_market"
  $r1 = Rect $proc
  $afterBoard = Field (Send "stats") 'rebuilds'
  $results += Check "market: mode switch rebuilds nothing" ($afterBoard -eq $beforeBoard) "rebuilds $beforeBoard -> $afterBoard"
  Start-Sleep 1
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $market = Send "stats"
  $results += Check "market with seconds: ~1 fps" ((Field $market 'fps') -ge 0.95 -and (Field $market 'fps') -le 1.2) $market
  $results += Check "market: every frame is a scheduled tick" ((Field $market 'tick') -eq (Field $market 'frames')) ""
  $results += Check "market: ui pass under 2 ms" ((Field $market 'mean_ms') -lt 2.0) ("mean " + (Field $market 'mean_ms') + " ms, max " + (Field $market 'max_ms') + " ms")
  # Columns and the caption line are sized for their widest values, so the
  # window must not have changed size while the clocks ticked.
  $r2 = Rect $proc
  $sameSize = (($r1.R - $r1.L) -eq ($r2.R - $r2.L)) -and (($r1.B - $r1.T) -eq ($r2.B - $r2.T))
  $results += Check "market: window size stable across ticks" $sameSize ("{0}x{1} -> {2}x{3}" -f ($r1.R - $r1.L), ($r1.B - $r1.T), ($r2.R - $r2.L), ($r2.B - $r2.T))
  # Without seconds the board changes once a minute: at most one tick in
  # 12 s, on top of the two frames every reply on this channel costs (the
  # ones a paused stopwatch reports as `other=2`; here they are classed as
  # ticks because a wake-up is scheduled).
  Send "cmd seconds" | Out-Null
  Start-Sleep 2
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $marketMin = Send "stats"
  $results += Check "market without seconds: at most one tick in 12 s" ((Field $marketMin 'frames') -le 3) $marketMin
  Shot "instr_market_minutes"
  Send "cmd backdrop" | Out-Null; Start-Sleep 1; Shot "instr_market_backdrop"; Send "cmd backdrop" | Out-Null
  Send "cmd 12h" | Out-Null; Start-Sleep 1; Shot "instr_market_12h"; Send "cmd 12h" | Out-Null
  Send "cmd digital" | Out-Null; Start-Sleep 1; Shot "instr_market_digital"; Send "cmd digital" | Out-Null
  Send "cmd chroma" | Out-Null; Start-Sleep 1; Shot "instr_market_chroma"; Send "cmd chroma" | Out-Null
  Send "cmd seconds" | Out-Null
  Start-Sleep 1
  # Adding an exchange from the menu adds a row; it lays out one new label
  # and rebuilds nothing.
  $beforeRow = Field (Send "stats") 'rebuilds'
  Send "cmd market:hong-kong" | Out-Null
  Start-Sleep 1
  Shot "instr_market_six"
  $afterRow = Field (Send "stats") 'rebuilds'
  $results += Check "market: adding a row rebuilds nothing" ($afterRow -eq $beforeRow) "rebuilds $beforeRow -> $afterRow"
  Send "cmd market:hong-kong" | Out-Null
  Send "cmd size:small" | Out-Null; Start-Sleep 1; Shot "instr_market_small"
  Send "cmd size:large" | Out-Null; Start-Sleep 1; Shot "instr_market_large"
  Send "cmd size:medium" | Out-Null
  # And a paused stopwatch still sleeps after a visit to the board.
  Send "cmd mode:stopwatch" | Out-Null
  Start-Sleep 2
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $afterMarket = Send "stats"
  $results += Check "after market: stopwatch paused, no timed frames" ((Field $afterMarket 'tick') -eq 0) $afterMarket

  # --- K: board arrangement and labels -----------------------------------------
  # One line and exchange codes are outside the layout key; each costs a few
  # new labels and no rebuild, and the strip is sized once like the stack.
  Send "cmd mode:market" | Out-Null
  Start-Sleep 1
  $beforeStrip = Field (Send "stats") 'rebuilds'
  Send "cmd horizontal" | Out-Null
  Start-Sleep 2
  Shot "instr_market_strip"
  $s1 = Rect $proc
  Start-Sleep 1
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $strip = Send "stats"
  $s2 = Rect $proc
  $results += Check "strip: ~1 fps, all ticks" ((Field $strip 'fps') -ge 0.95 -and (Field $strip 'fps') -le 1.2 -and (Field $strip 'tick') -eq (Field $strip 'frames')) $strip
  $results += Check "strip: window size stable across ticks" ((($s1.R - $s1.L) -eq ($s2.R - $s2.L)) -and (($s1.B - $s1.T) -eq ($s2.B - $s2.T))) ("{0}x{1} -> {2}x{3}" -f ($s1.R - $s1.L), ($s1.B - $s1.T), ($s2.R - $s2.L), ($s2.B - $s2.T))
  Send "cmd codes" | Out-Null
  Start-Sleep 1
  Shot "instr_market_strip_codes"
  Send "cmd horizontal" | Out-Null
  Start-Sleep 1
  Shot "instr_market_codes"
  $afterStrip = Field (Send "stats") 'rebuilds'
  $results += Check "strip and codes: no layout rebuilds" ($afterStrip -eq $beforeStrip) "rebuilds $beforeStrip -> $afterStrip"
  Send "cmd codes" | Out-Null

  # --- L: the studio ring and the industrial palettes ---------------------------
  Assert-Drawing 'L: studio ring'
  # The ring adds a band, not a rebuild. With seconds hidden the digits change
  # once a minute but the ring every second, so the clock is back to ~1 fps;
  # a paused stopwatch has nothing moving and stays asleep, ring or not.
  Send "cmd mode:clock" | Out-Null
  Send "cmd seconds" | Out-Null
  Start-Sleep 1
  $beforeRing = Field (Send "stats") 'rebuilds'
  Send "cmd ring" | Out-Null
  Start-Sleep 2
  Shot "instr_ring"
  $afterRing = Field (Send "stats") 'rebuilds'
  $results += Check "ring: no layout rebuild" ($afterRing -eq $beforeRing) "rebuilds $beforeRing -> $afterRing"
  Start-Sleep 1
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $ringClock = Send "stats"
  $results += Check "ring, minutes only: ~1 fps for the ring" ((Field $ringClock 'fps') -ge 0.95 -and (Field $ringClock 'fps') -le 1.2) $ringClock
  $results += Check "ring: every frame is a scheduled tick" ((Field $ringClock 'tick') -eq (Field $ringClock 'frames')) ""
  $results += Check "ring: ui pass under 2 ms" ((Field $ringClock 'mean_ms') -lt 2.0) ("mean " + (Field $ringClock 'mean_ms') + " ms, max " + (Field $ringClock 'max_ms') + " ms")
  Send "cmd backdrop" | Out-Null; Start-Sleep 1; Shot "instr_ring_backdrop"
  foreach ($p in 'green', 'red', 'yellow', 'studio') {
    Send "cmd palette:$p" | Out-Null
    Start-Sleep 1
    Shot "instr_palette_$p"
  }
  Send "cmd mode:stopwatch" | Out-Null; Start-Sleep 1; Shot "instr_studio_stopwatch"
  Send "cmd size:small" | Out-Null; Start-Sleep 1; Shot "instr_ring_small"
  Send "cmd size:large" | Out-Null; Start-Sleep 1; Shot "instr_ring_large"
  Send "cmd size:medium" | Out-Null
  Send "cmd backdrop" | Out-Null
  Start-Sleep 2
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $ringPaused = Send "stats"
  $results += Check "ring, stopwatch paused: no timed frames" ((Field $ringPaused 'tick') -eq 0) $ringPaused
  Send "cmd digital" | Out-Null; Send "cmd mode:clock" | Out-Null; Start-Sleep 1; Shot "instr_ring_digital"
  # The hardware dressing: ghost segments under the digits, LED sockets and
  # quarter markers on the ring, hairlines between the board's modules. All
  # of it is paint, none of it geometry, so the frame cost is what matters.
  Send "cmd backdrop" | Out-Null; Start-Sleep 1; Shot "instr_hardware_clock"
  Send "cmd mode:market" | Out-Null; Start-Sleep 1; Shot "instr_hardware_board"
  Send "cmd horizontal" | Out-Null; Start-Sleep 1; Shot "instr_hardware_strip"
  # The dot-matrix face is a third face in the layout key: one rebuild in,
  # one back, like the seven-segment one.
  $beforeMatrix = Field (Send "stats") 'rebuilds'
  Send "cmd font:matrix" | Out-Null; Start-Sleep 1; Shot "instr_matrix_strip"
  $matrixOn = Field (Send "stats") 'rebuilds'
  $results += Check "dot matrix: exactly one rebuild" ($matrixOn -eq ($beforeMatrix + 1)) "rebuilds $beforeMatrix -> $matrixOn"
  Send "cmd horizontal" | Out-Null; Start-Sleep 1; Shot "instr_matrix_board"
  Send "cmd mode:clock" | Out-Null; Start-Sleep 1; Shot "instr_matrix_clock"
  Send "cmd font:digital" | Out-Null; Start-Sleep 1
  Start-Sleep 1
  Send "stats reset" | Out-Null
  Start-Sleep 12
  $hardware = Send "stats"
  $results += Check "hardware dressing, digital + ring + backdrop: ~1 fps" ((Field $hardware 'fps') -ge 0.95 -and (Field $hardware 'fps') -le 1.2) $hardware
  $results += Check "hardware dressing: ui pass under 2 ms" ((Field $hardware 'mean_ms') -lt 2.0) ("mean " + (Field $hardware 'mean_ms') + " ms, max " + (Field $hardware 'max_ms') + " ms")
  Send "cmd backdrop" | Out-Null; Send "cmd digital" | Out-Null
  foreach ($c in 'palette:default', 'ring', 'seconds') { Send "cmd $c" | Out-Null }
}
catch { $results += "ABORT $_" }
finally {
  if ($writer) { try { Send "quit" | Out-Null } catch {} }
  if ($client) { $client.Close() }
  Start-Sleep 1
  if ($proc -and -not $proc.HasExited) { $proc.Kill() }
  Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item $portFile -ErrorAction SilentlyContinue
}
$results
"", ("{0} passed, {1} failed{2}" -f ($results | ? { $_ -like 'PASS*' }).Count, ($results | ? { $_ -like 'FAIL*' }).Count, $(if ($results -like 'ABORT*') { ', run aborted' } else { '' }))
