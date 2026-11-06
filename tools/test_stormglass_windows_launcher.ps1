param([Parameter(Mandatory=$true)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Windows contract tests required.' }
$root=[IO.Path]::GetFullPath($EvidenceDirectory)
if (Test-Path -LiteralPath $root) { throw 'Choose a fresh evidence directory.' }
New-Item -ItemType Directory -Path $root | Out-Null
$game=Join-Path $root 'game [one] & space'
$elsewhere=Join-Path $root 'other cwd'
New-Item -ItemType Directory -Path $game,$elsewhere | Out-Null
$launcher=Join-Path $game 'run-game.ps1'
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'stormglass_windows/run-game.ps1') -Destination $launcher
$runtime=Join-Path $game 'coop_defense.exe'
& rustc --edition=2021 (Join-Path $PSScriptRoot 'stormglass_windows/launcher_probe.rs') -o $runtime
if ($LASTEXITCODE -ne 0) { throw 'Probe compilation failed.' }
[IO.File]::WriteAllText((Join-Path $game 'game.scene.yaml'),'test-only scene; probe never parses it')
$hosts=@((Get-Process -Id $PID).Path,(Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe')) | Select-Object -Unique
$script:caseNumber=0
$script:successCount=0
function Run-Launcher([string]$HostPath,[string[]]$Arguments,[int]$ProbeExit=0,[string]$ProbeError='') {
    $script:caseNumber++
    $record=Join-Path $root ("argv-{0}.bin" -f $script:caseNumber)
    $start=[Diagnostics.ProcessStartInfo]::new()
    $start.FileName=$HostPath;$start.WorkingDirectory=$elsewhere;$start.UseShellExecute=$false;$start.CreateNoWindow=$true
    $start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    foreach($arg in @('-NoProfile','-NonInteractive','-File',$launcher)+$Arguments) {$start.ArgumentList.Add($arg)}
    $start.Environment['ORR_LAUNCHER_PROBE_RECORD']=$record
    $start.Environment['ORR_LAUNCHER_PROBE_EXIT']=[string]$ProbeExit
    $start.Environment['ORR_LAUNCHER_PROBE_STDERR']=$ProbeError
    $process=[Diagnostics.Process]::new();$process.StartInfo=$start
    if (-not $process.Start()) {throw 'Could not start PowerShell test process.'}
    $stdout=$process.StandardOutput.ReadToEndAsync();$stderr=$process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(15000)) { $process.Kill($true);$process.WaitForExit();throw 'Owned launcher test timed out.' }
    $out=$stdout.GetAwaiter().GetResult();$err=$stderr.GetAwaiter().GetResult();$code=$process.ExitCode;$process.Dispose()
    [IO.File]::WriteAllText((Join-Path $root ("case-{0}.stdout" -f $script:caseNumber)),$out)
    [IO.File]::WriteAllText((Join-Path $root ("case-{0}.stderr" -f $script:caseNumber)),$err)
    $received=if(Test-Path -LiteralPath $record){@([IO.File]::ReadAllText($record).Split([char]0))}else{$null}
    if($null -ne $received -and $code -ne $ProbeExit){throw "Launcher exit $code differs from probe $ProbeExit."}
    [pscustomobject]@{Code=$code;Args=$received;Error=$err}
}
function Equal-Arguments($Actual,[string[]]$Expected) {
    if ($null -eq $Actual -or ($Actual -join [char]0) -cne ($Expected -join [char]0)) {throw "Argument boundary mismatch: $($Actual -join ' | ')"}
    $script:successCount++
}
function Refused($Result) {
    if ($Result.Code -eq 0 -or $null -ne $Result.Args) {throw 'Invalid launch reached the runtime or returned success.'}
    $script:successCount++
}
foreach($hostPath in $hosts) {
    $result=Run-Launcher $hostPath @('-Headless','-Ticks','60')
    if($result.Code -ne 0){throw $result.Error}
    Equal-Arguments $result.Args @('--scene',(Join-Path $game 'game.scene.yaml'),'--audio','off','--headless','--ticks','60')
    $result=Run-Launcher $hostPath @('-Headless','-WithAudioBank','-Ticks','0')
    Equal-Arguments $result.Args @('--scene',(Join-Path $game 'game.scene.yaml'),'--audio','off','--audio-bank',(Join-Path $game 'audio-bank'),'--headless','--ticks','0')
    $result=Run-Launcher $hostPath @('-Audio','AUTO')
    Equal-Arguments $result.Args @('--scene',(Join-Path $game 'game.scene.yaml'),'--audio','auto','--audio-bank',(Join-Path $game 'audio-bank'))
    $controlPath=Join-Path $root 'private & $(throw 123) [controls]'
    $audioPath=Join-Path $root 'private audio'
    $result=Run-Launcher $hostPath @('-ControlsDirectory',$controlPath,'-AudioSettingsDirectory',$audioPath)
    Equal-Arguments $result.Args @('--scene',(Join-Path $game 'game.scene.yaml'),'--audio','off','--controls-directory',$controlPath,'--audio-settings-directory',$audioPath)
    if((Test-Path -LiteralPath $controlPath) -or (Test-Path -LiteralPath $audioPath)){throw 'Launcher created user settings implicitly.'}
    foreach($exitCode in @(0,37)) {
        $result=Run-Launcher $hostPath @('-Headless') $exitCode 'probe stderr is not a launch exception'
        if($result.Code -ne $exitCode -or -not $result.Error.Contains('probe stderr')){throw 'Native exit/stderr was not preserved.'}
        $script:successCount++
    }
    foreach($bad in @(
        @('-Ticks','1'),@('-Headless','-Ticks','36001'),@('-Headless','-Ticks','-1'),@('-Headless','-Ticks','1.5'),
        @('-Headless','-Audio','required'),@('-Headless','-ControlsDirectory',$controlPath),@('-Headless','-AudioSettingsDirectory',$audioPath),
        @('-ControlsDirectory','relative'),@('-AudioSettingsDirectory','C:relative'),@('-ControlsDirectory',$game),
        @('-ControlsDirectory',(Join-Path $game 'nested')),@('-Audio','unknown'),@('-UnknownOption'),@('-Headless','-Ticks','1','-Ticks','2')
    )) { Refused (Run-Launcher $hostPath $bad) }
    $scene=Join-Path $game 'game.scene.yaml';Move-Item -LiteralPath $scene -Destination (Join-Path $game 'scene.hidden')
    Refused (Run-Launcher $hostPath @('-Headless'))
    Move-Item -LiteralPath (Join-Path $game 'scene.hidden') -Destination $scene
}
# Test folders are known descendants of this fresh evidence root; no recursive move/delete.
$moved=Join-Path $root 'moved game'
foreach($path in @($game,$moved)){
    if(-not [IO.Path]::GetFullPath($path).StartsWith($root+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Move escaped evidence root'}
}
Move-Item -LiteralPath $game -Destination $moved
$game=$moved;$launcher=Join-Path $game 'run-game.ps1'
$result=Run-Launcher $hosts[0] @('-Headless','-WithAudioBank','-Ticks','60')
Equal-Arguments $result.Args @('--scene',(Join-Path $game 'game.scene.yaml'),'--audio','off','--audio-bank',(Join-Path $game 'audio-bank'),'--headless','--ticks','60')
# Failed process creation must not reuse a stale success code.
[IO.File]::WriteAllText((Join-Path $game 'coop_defense.exe'),'not an executable')
Refused (Run-Launcher $hosts[0] @('-Headless'))
[pscustomobject]@{schema=1;contract_assertions=$script:successCount;hosts=$hosts;kind='probe argv/exit only';real_game='UNRUN';window_device='UNRUN'} |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'result.json') -Encoding utf8
Write-Output "PASS launcher contract assertions=$script:successCount; real game/window/device UNRUN."
