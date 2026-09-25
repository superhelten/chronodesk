# Measures how long a launch takes to put its first frame up.
#
# The app notes four points on the way (`startup` on the instrument channel),
# each in milliseconds since the OS created the process, so the time the
# loader spends before any of our code runs is included:
#
#   main         main() is entered
#   config       the settings are read
#   app          eframe has made the window and its OpenGL context
#   first_frame  the first frame is laid out and painted
#
# Each run starts a fresh instance against its own scratch config, reads the
# points once the first frame is in, and quits it. Prints every run and the
# median, and fails if the median first frame takes longer than -BudgetMs.
# Leaves a running overlay alone and makes no sound.
#
# Run after: cargo build --release --features instrument
param([int]$Runs = 8, [int]$BudgetMs = 1000)
$ErrorActionPreference = 'Stop'
$s = Split-Path $MyInvocation.MyCommand.Path
$exe = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$portFile = "$env:TEMP\chronodesk-instrument.port"
$scratch = Join-Path $env:TEMP 'chronodesk-startup-time'
$ron = Join-Path $scratch 'app.ron'
$points = 'main', 'config', 'app', 'first_frame'

New-Item -ItemType Directory -Force $scratch | Out-Null
$rows = @()
try {
  foreach ($run in 1..$Runs) {
    # A returning user's file, so the welcome card is not what gets drawn.
    Set-Content $ron '(schema_version:1,window:Some((x:40.0,y:40.0)))' -NoNewline
    Remove-Item $portFile -ErrorAction SilentlyContinue
    $env:CHRONODESK_CONFIG = $ron
    $proc = Start-Process $exe -ArgumentList '--instrument' -PassThru
    $env:CHRONODESK_CONFIG = $null
    foreach ($i in 1..80) { Start-Sleep -Milliseconds 100; if (Test-Path $portFile) { break } }
    if (-not (Test-Path $portFile)) { throw "no instrument port published" }
    $client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', [int](Get-Content $portFile))
    $stream = $client.GetStream()
    $reader = New-Object System.IO.StreamReader($stream)
    $writer = New-Object System.IO.StreamWriter($stream)
    $writer.AutoFlush = $true
    $line = ''
    foreach ($i in 1..50) {
      $writer.WriteLine('startup'); $line = $reader.ReadLine()
      if ($line -match 'first_frame_ms') { break }
      Start-Sleep -Milliseconds 100
    }
    $writer.WriteLine('quit'); $null = $reader.ReadLine(); $client.Close()
    if (-not $proc.WaitForExit(5000)) { $proc.Kill(); $proc.WaitForExit() }
    $row = [ordered]@{ run = $run }
    foreach ($p in $points) { $row[$p] = if ($line -match "${p}_ms=([0-9.]+)") { [double]$matches[1] } else { $null } }
    $rows += [pscustomobject]$row
    Start-Sleep -Milliseconds 500
  }
} finally {
  Get-Process chronodesk -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Resolve-Path $exe).Path } | Stop-Process -Force
  Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
}

$rows | Format-Table -AutoSize | Out-String | Write-Output
$median = @{}
foreach ($p in $points) {
  $values = @($rows | ForEach-Object { $_.$p } | Where-Object { $null -ne $_ } | Sort-Object)
  if ($values.Count -eq 0) { continue }
  $median[$p] = $values[[int][math]::Floor($values.Count / 2)]
  '{0,-12} median {1,7:n1} ms   min {2,7:n1}   max {3,7:n1}' -f $p, $median[$p], $values[0], $values[-1]
}
$ok = $median.ContainsKey('first_frame') -and $median['first_frame'] -le $BudgetMs
''
"{0} first frame within {1} ms" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $BudgetMs
if (-not $ok) { exit 1 }
