# Real preparation helper + game argv/exit probe. No game window or settings UI claim.
param(
    [Parameter(Mandatory=$true)][string]$PreparationHelper,
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory
)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath($EvidenceDirectory)
if($env:OS -ne 'Windows_NT' -or (Test-Path -LiteralPath $root)){throw 'Windows and fresh evidence required'}
New-Item -ItemType Directory -Path $root | Out-Null
$game=Join-Path $root 'game [bridge] & space'
$cwd=Join-Path $root 'other cwd'
New-Item -ItemType Directory -Path $game,$cwd | Out-Null
$runtime=Join-Path $game 'coop_defense.exe'
$helper=Join-Path $game 'orr_prepare_stormglass_settings.exe'
$launcher=Join-Path $game 'run-game.ps1'
Copy-Item -LiteralPath $PreparationHelper -Destination $helper
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'stormglass_windows/run-game.ps1') -Destination $launcher
& rustc --edition=2021 (Join-Path $PSScriptRoot 'stormglass_windows/launcher_probe.rs') -o $runtime
if($LASTEXITCODE -ne 0){throw 'Probe build failed'}
[IO.File]::WriteAllText((Join-Path $game 'game.scene.yaml'),'probe only; never parsed as game scene')
$script:caseNumber=0
function Run([string]$HostPath,[string[]]$Arguments,[int]$GameExit=0){
    $script:caseNumber++
    $record=Join-Path $root "game-argv-$script:caseNumber.bin"
    $s=[Diagnostics.ProcessStartInfo]::new()
    $s.FileName=$HostPath;$s.WorkingDirectory=$cwd;$s.UseShellExecute=$false;$s.CreateNoWindow=$true
    $s.RedirectStandardOutput=$true;$s.RedirectStandardError=$true
    foreach($a in @('-NoProfile','-NonInteractive','-File',$launcher)+$Arguments){$s.ArgumentList.Add($a)}
    $s.Environment['ORR_LAUNCHER_PROBE_RECORD']=$record
    $s.Environment['ORR_LAUNCHER_PROBE_EXIT']=[string]$GameExit
    $s.Environment['ORR_LAUNCHER_PROBE_STDERR']='game probe diagnostic'
    $p=[Diagnostics.Process]::new();$p.StartInfo=$s
    if(-not $p.Start()){throw 'Start failed'}
    $out=$p.StandardOutput.ReadToEndAsync();$err=$p.StandardError.ReadToEndAsync()
    if(-not $p.WaitForExit(15000)){$p.Kill($true);$p.WaitForExit();throw 'Owned bridge test timed out'}
    $o=$out.GetAwaiter().GetResult();$e=$err.GetAwaiter().GetResult();$code=$p.ExitCode;$p.Dispose()
    [IO.File]::WriteAllText((Join-Path $root "case-$script:caseNumber.stdout"),$o)
    [IO.File]::WriteAllText((Join-Path $root "case-$script:caseNumber.stderr"),$e)
    $argsReceived=if(Test-Path -LiteralPath $record){[IO.File]::ReadAllText($record)}else{$null}
    [pscustomobject]@{Code=$code;Args=$argsReceived;Error=$e}
}
function Expected([string]$Directory){
    @('--scene',(Join-Path $game 'game.scene.yaml'),'--audio','off','--controls-directory',$Directory,'--audio-settings-directory',$Directory) -join [char]0
}
function Refused($Result,[string]$Directory){
    if($Result.Code -eq 0 -or $null -ne $Result.Args -or (Test-Path -LiteralPath $Directory)){throw 'Refused preparation launched game or created directory'}
}
$hosts=@((Get-Process -Id $PID).Path,(Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe')) | Select-Object -Unique
$hostNumber=0
foreach($hostPath in $hosts){
    $hostNumber++
    $unprepared=Join-Path $root "unprepared-$hostNumber"
    $r=Run $hostPath @('-UserSettingsDirectory',$unprepared)
    if($r.Code -ne 0 -or $r.Args -cne (Expected $unprepared) -or (Test-Path -LiteralPath $unprepared)){throw 'Existing-only path created directory or forwarded wrong arguments'}
    $settings=Join-Path $root "private & [settings] $hostNumber"
    $r=Run $hostPath @('-UserSettingsDirectory',$settings,'-PrepareUserSettings')
    if($r.Code -ne 0 -or $r.Args -cne (Expected $settings)){throw 'Prepare-to-launch bridge failed'}
    if(@(Get-ChildItem -LiteralPath $settings -Force).Count -ne 0){throw 'Preferences created implicitly'}
    $acl=Get-Acl -LiteralPath $settings
    if(-not $acl.AreAccessRulesProtected -or $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne [Security.Principal.WindowsIdentity]::GetCurrent().User.Value){throw 'Prepared directory not private/user owned'}
    [IO.File]::WriteAllText((Join-Path $settings 'sentinel'),'preserved')
    $r=Run $hostPath @('-UserSettingsDirectory',$settings,'-PrepareUserSettings')
    if($r.Code -eq 0 -or $null -ne $r.Args -or (Get-Acl -LiteralPath $settings).Sddl -cne $acl.Sddl -or [IO.File]::ReadAllText((Join-Path $settings 'sentinel')) -cne 'preserved'){throw 'Failed existing preparation modified data or launched game'}
    $retained=Join-Path $root "retained-$hostNumber"
    $r=Run $hostPath @('-UserSettingsDirectory',$retained,'-PrepareUserSettings') 37
    if($r.Code -ne 37 -or $r.Args -cne (Expected $retained) -or -not (Test-Path -LiteralPath $retained) -or -not $r.Error.Contains('game probe diagnostic')){throw 'Game failure lost status or rolled back created directory'}
    $blocked=Join-Path $root "headless-$hostNumber"
    Refused (Run $hostPath @('-Headless','-UserSettingsDirectory',$blocked,'-PrepareUserSettings')) $blocked
    $blocked=Join-Path $root "missing-helper-$hostNumber"
    Move-Item -LiteralPath $helper -Destination (Join-Path $game 'real-helper.hidden')
    Refused (Run $hostPath @('-UserSettingsDirectory',$blocked,'-PrepareUserSettings')) $blocked
    Move-Item -LiteralPath (Join-Path $game 'real-helper.hidden') -Destination $helper
    $blocked=Join-Path $root "missing-game-$hostNumber"
    Move-Item -LiteralPath $runtime -Destination (Join-Path $game 'game.hidden')
    Refused (Run $hostPath @('-UserSettingsDirectory',$blocked,'-PrepareUserSettings')) $blocked
    Move-Item -LiteralPath (Join-Path $game 'game.hidden') -Destination $runtime
    $blocked=Join-Path $root "invalid-helper-$hostNumber"
    Move-Item -LiteralPath $helper -Destination (Join-Path $game 'real-helper.hidden')
    [IO.File]::WriteAllText($helper,'not executable')
    Refused (Run $hostPath @('-UserSettingsDirectory',$blocked,'-PrepareUserSettings')) $blocked
    Move-Item -LiteralPath $helper -Destination (Join-Path $game "invalid-helper-$hostNumber.retained")
    Move-Item -LiteralPath (Join-Path $game 'real-helper.hidden') -Destination $helper
}
[pscustomobject]@{schema=1;cases=$script:caseNumber;hosts=$hosts;preparation_helper='real unchanged native executable';helper_sha256=(Get-FileHash -LiteralPath $PreparationHelper).Hash;game='argv/exit probe only';window_device_ui_persistence='UNRUN'} |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'result.json') -Encoding utf8
Write-Output "PASS real helper + game probe integration cases=$script:caseNumber; window/device/UI persistence UNRUN."
