# Verifies the installer end to end, with the real exe doing the real work:
#
#  - a file named ChronoDesk-Setup.exe installs itself to the fixed path,
#    registers under Installed apps, writes a Start menu shortcut that points
#    at that path, and starts the installed copy (which greets a newcomer);
#  - running the setup again with the same version copies nothing, leaves the
#    running overlay running and asks it to show itself;
#  - so does simply starting the installed exe a second time, and that brings
#    the overlay back even from minimised, where it draws nothing and has no
#    taskbar button to be restored from;
#  - a "Start with Windows" entry that exists is repointed at the fixed path,
#    and one that does not exist is not invented;
#  - a setup with another version makes the running overlay quit, replaces the
#    exe and starts the new one;
#  - uninstalling removes all of it, the folder included, and keeps the settings,
#    also the way Windows' own Uninstall button does it: with a message box that
#    stays up for as long as the user takes to read it.
#
# The shortcut and the registry are read here with PowerShell rather than
# through the app, so the assertions check what Windows will see.
# CHRONODESK_INSTALL_ROOT and CHRONODESK_RUN_KEY (instrument builds only) point
# the run at a scratch folder and a scratch registry key, and CHRONODESK_CONFIG
# at a scratch file: neither %LOCALAPPDATA%\ChronoDesk, your Start menu, your
# registry nor a running overlay is touched, and the script checks that too.
#
# Run after: cargo build --release --features instrument
$ErrorActionPreference = 'Stop'
$s = Split-Path $MyInvocation.MyCommand.Path
$built = Join-Path (Split-Path $s) 'target\release\chronodesk.exe'
$portFile = "$env:TEMP\chronodesk-instrument.port"
$scratch = Join-Path $env:TEMP 'chronodesk-install-test'
$ron = Join-Path $scratch 'config\app.ron'
$setup = Join-Path $scratch 'downloads\ChronoDesk-Setup.exe'
$setup2 = Join-Path $scratch 'downloads\ChronoDesk-Setup (1).exe'
$dir = Join-Path $scratch 'install\app'
$installed = Join-Path $dir 'chronodesk.exe'
$link = Join-Path $scratch 'install\start menu\ChronoDesk.lnk'
$root = "Software\ChronoDesk-install-test-$PID"
$runKey = "HKCU:\$root\Run"
$uninstallKey = "HKCU:\$root\Uninstall\ChronoDesk"
$realRun = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$realUninstall = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ChronoDesk'
$realDir = Join-Path $env:LOCALAPPDATA 'ChronoDesk'

Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class N {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
}
"@ -ErrorAction SilentlyContinue
function Window($process) {
  $script:found = [IntPtr]::Zero
  [N]::EnumWindows({ param($h, $l) $owner = 0; [N]::GetWindowThreadProcessId($h, [ref]$owner) | Out-Null
    if ($owner -eq $process.Id) { $sb = New-Object System.Text.StringBuilder 64; [N]::GetClassName($h, $sb, 64) | Out-Null
      if ($sb.ToString() -eq "Window Class") { $script:found = $h } }
    return $true }, [IntPtr]::Zero) | Out-Null
  $script:found
}

$client = $null; $reader = $null; $writer = $null
function Connect {
  foreach ($i in 1..60) { Start-Sleep -Milliseconds 250; if (Test-Path $portFile) { break } }
  if (-not (Test-Path $portFile)) { throw "no instrument port published" }
  $script:client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', [int](Get-Content $portFile))
  $stream = $client.GetStream()
  $script:reader = New-Object System.IO.StreamReader($stream)
  $script:writer = New-Object System.IO.StreamWriter($stream)
  $script:writer.AutoFlush = $true
  Send "passthrough on" | Out-Null
  foreach ($i in 1..50) { if ((Send "state") -notlike 'err*') { break }; Start-Sleep -Milliseconds 100 }
}
function Disconnect { if ($client) { $client.Close() }; $script:client = $null; $script:writer = $null }
function Send([string]$line) { $script:writer.WriteLine($line); $script:reader.ReadLine() }
function Check($name, $ok, $detail) {
  "{0} {1}{2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $(if ($detail) { " — $detail" } else { '' })
}
# Runs an exe under the scratch environment and waits for it to finish.
function Run($exe, $arguments) {
  $env:CHRONODESK_CONFIG = $ron; $env:CHRONODESK_RUN_KEY = $root; $env:CHRONODESK_INSTALL_ROOT = (Join-Path $scratch 'install')
  try {
    $p = Start-Process $exe -ArgumentList $arguments -PassThru
    if (-not $p.WaitForExit(20000)) { $p.Kill(); throw "$exe $arguments did not finish" }
    $p.ExitCode
  } finally { $env:CHRONODESK_CONFIG = $null; $env:CHRONODESK_RUN_KEY = $null; $env:CHRONODESK_INSTALL_ROOT = $null }
}
function Overlays { @(Get-Process chronodesk -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $installed }) }
function Hash($path) { (Get-FileHash $path -Algorithm SHA256).Hash }
function Value($key, $name) { (Get-ItemProperty $key -ErrorAction SilentlyContinue).$name }
function Real-State {
  $realLink = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\ChronoDesk.lnk'
  "run='{0}' uninstall-key={1} folder={2} shortcut={3}" -f (Value $realRun 'ChronoDesk'), (Test-Path $realUninstall), (Test-Path $realDir), (Test-Path $realLink)
}

$results = @()
$realBefore = Real-State
try {
  Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item $portFile -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force (Split-Path $setup) | Out-Null
  Copy-Item $built $setup

  # --- A: a first installation ---------------------------------------------------
  $code = Run $setup '--instrument', '--quiet'
  $results += Check "install: the setup finishes cleanly" ($code -eq 0) "exit code $code"
  $results += Check "install: the exe is at the fixed path, byte for byte" ((Test-Path $installed) -and (Hash $installed) -eq (Hash $setup)) $installed
  $results += Check "install: an icon file sits beside it" (Test-Path (Join-Path $dir 'chronodesk.ico')) ""
  $shell = New-Object -ComObject WScript.Shell
  $target = if (Test-Path $link) { $shell.CreateShortcut($link).TargetPath } else { $null }
  $results += Check "install: the Start menu shortcut starts the installed exe" ($target -eq $installed) $target
  $results += Check "install: listed under Installed apps" `
    ((Value $uninstallKey 'DisplayName') -eq 'ChronoDesk' -and (Value $uninstallKey 'UninstallString') -eq "`"$installed`" --uninstall") `
    (Value $uninstallKey 'UninstallString')
  $results += Check "install: starting at login is not switched on behind the user's back" ($null -eq (Value $runKey 'ChronoDesk')) ""
  Connect
  $first = Overlays
  $results += Check "install: the installed copy is what is running" ($first.Count -eq 1) "$($first.Count) overlay(s) from $installed"
  $state = Send "state"
  $results += Check "install: a newcomer is greeted" ($state -match 'welcome=1') $state

  # --- B: the setup again, same version ------------------------------------------
  $code = Run $setup '--instrument', '--quiet'
  Start-Sleep 1
  $again = Overlays
  $state = Send "state"
  $results += Check "setup again: nothing is replaced and the overlay keeps running" `
    ($code -eq 0 -and $again.Count -eq 1 -and $again[0].Id -eq $first[0].Id) "pid $($first[0].Id) -> $($again.Id -join ',')"
  $results += Check "setup again: the running overlay shows itself" ($state -match 'attention=1') $state

  # --- C: the installed exe started a second time -----------------------------------
  Start-Sleep 4
  $quiet = Send "state"
  $code = Run $installed @('--instrument')
  Start-Sleep 1
  $state = Send "state"
  $results += Check "second launch: steps aside, and the first one shows itself" `
    ($code -eq 0 -and $quiet -match 'attention=0' -and $state -match 'attention=1' -and (Overlays).Count -eq 1) "$quiet => $state"

  # A minimised overlay draws no frames at all, so it cannot be asked anything
  # through the app; the second launch has to bring it back regardless.
  $hwnd = Window $first[0]
  [N]::ShowWindow($hwnd, 6) | Out-Null
  Start-Sleep 1
  $wasMinimised = [N]::IsIconic($hwnd)
  Run $installed @('--instrument') | Out-Null
  Start-Sleep 1
  $results += Check "second launch: brings a minimised overlay back" ($wasMinimised -and -not [N]::IsIconic($hwnd)) "minimised before: $wasMinimised, after: $([N]::IsIconic($hwnd))"

  # --- D: an entry left by a copy somewhere else follows the installation ---------
  New-Item $runKey -Force | Out-Null
  Set-ItemProperty $runKey ChronoDesk '"C:\dev\chronodesk\target\release\chronodesk.exe"'
  Run $setup '--instrument', '--quiet' | Out-Null
  $results += Check "autostart: an existing entry is repointed at the fixed path" ((Value $runKey 'ChronoDesk') -eq "`"$installed`"") (Value $runKey 'ChronoDesk')

  # --- E: another version ---------------------------------------------------------------
  # Bytes after the end of the image do not stop an exe from running, and make
  # this one a different file: what the next release looks like to the installer.
  Copy-Item $built $setup2
  [IO.File]::AppendAllText($setup2, 'a newer build')
  Disconnect
  Remove-Item $portFile -ErrorAction SilentlyContinue
  $code = Run $setup2 '--instrument', '--quiet'
  $gone = $first[0].WaitForExit(8000)
  $results += Check "upgrade: the running overlay is asked to quit, and does" ($code -eq 0 -and $gone) "exit code $code"
  $results += Check "upgrade: the new exe is in place and nothing is left over" `
    ((Hash $installed) -eq (Hash $setup2) -and -not (Test-Path "$installed.old") -and -not (Test-Path "$installed.new")) ""
  Connect
  $now = Overlays
  $results += Check "upgrade: and the new one is running" ($now.Count -eq 1 -and $now[0].Id -ne $first[0].Id) "pid $($first[0].Id) -> $($now.Id -join ',')"

  # --- F: uninstall -------------------------------------------------------------------------
  Disconnect
  $code = Run $installed '--uninstall', '--quiet'
  $gone = $now[0].WaitForExit(8000)
  $results += Check "uninstall: the overlay quits" ($code -eq 0 -and $gone) "exit code $code"
  $results += Check "uninstall: gone from Installed apps, the Start menu and the Run key" `
    (-not (Test-Path $uninstallKey) -and -not (Test-Path $link) -and $null -eq (Value $runKey 'ChronoDesk')) ""
  foreach ($i in 1..40) { if (-not (Test-Path $dir)) { break }; Start-Sleep -Milliseconds 250 }
  $results += Check "uninstall: the folder goes once the exe has exited" (-not (Test-Path $dir)) $dir
  $results += Check "uninstall: the settings are left alone" (Test-Path $ron) $ron

  # --- G: the way Settings > Installed apps runs it -----------------------------------------
  # No --quiet: a message box reports the result, and the exe stays alive, and
  # undeletable, until it is dismissed. The clean-up has to outlast that.
  Remove-Item $portFile -ErrorAction SilentlyContinue
  Run $setup '--instrument', '--quiet' | Out-Null
  Connect; Disconnect
  $env:CHRONODESK_CONFIG = $ron; $env:CHRONODESK_RUN_KEY = $root; $env:CHRONODESK_INSTALL_ROOT = (Join-Path $scratch 'install')
  try { $un = Start-Process $installed -ArgumentList '--uninstall' -PassThru }
  finally { $env:CHRONODESK_CONFIG = $null; $env:CHRONODESK_RUN_KEY = $null; $env:CHRONODESK_INSTALL_ROOT = $null }
  foreach ($i in 1..60) { $un.Refresh(); if ($un.MainWindowHandle -ne [IntPtr]::Zero) { break }; Start-Sleep -Milliseconds 250 }
  $box = $un.MainWindowHandle -ne [IntPtr]::Zero
  Start-Sleep 4
  $waiting = -not $un.HasExited -and (Test-Path $installed)
  if ($box) { $un.CloseMainWindow() | Out-Null }
  if (-not $un.WaitForExit(5000)) { $un.Kill() }
  foreach ($i in 1..60) { if (-not (Test-Path $dir)) { break }; Start-Sleep -Milliseconds 250 }
  $results += Check "uninstall from Settings: says so, and the folder still goes however long the message stays up" `
    ($box -and $waiting -and -not (Test-Path $dir)) "message box: $box, held open 4 s: $waiting, folder left: $(Test-Path $dir)"

  $results += Check "throughout: the real folder, Start menu entry and registry were never touched" ((Real-State) -eq $realBefore) (Real-State)
}
finally {
  Disconnect
  Overlays | ForEach-Object { $_.Kill() }
  Start-Sleep 1
  Remove-Item "HKCU:\$root" -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item $portFile -ErrorAction SilentlyContinue
}
$results
"", ("{0} passed, {1} failed" -f ($results | ? { $_ -like 'PASS*' }).Count, ($results | ? { $_ -like 'FAIL*' }).Count)
