# Real CPU/headless runtime smoke; never opens a window or audio device.
param(
    [Parameter(Mandatory=$true)][string]$Runtime,
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory
)
$ErrorActionPreference='Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Windows required.' }
$root=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $root){throw 'Choose fresh evidence.'}
New-Item -ItemType Directory -Path $root | Out-Null
$game=Join-Path $root 'game [real] & space'
$cwd=Join-Path $root 'other cwd'
New-Item -ItemType Directory -Path $game,$cwd | Out-Null
Copy-Item -LiteralPath $Runtime -Destination (Join-Path $game 'coop_defense.exe')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'stormglass_windows/run-game.ps1') -Destination (Join-Path $game 'run-game.ps1')
[IO.File]::WriteAllText((Join-Path $game 'game.scene.yaml'),"schema: orr.scene/1`nentities: {}`nsingletons: {CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}}`n")
# Test-only fixed bank, same two-frame samples/schema as coop_audio_admission_tests.
# This does not exercise the Linux-only authoring CLI or establish media rights.
$bank=Join-Path $game 'audio-bank'
New-Item -ItemType Directory -Path $bank | Out-Null
$pcm=[byte[]](128,187,0,0,2,0,0,0,0,224,0,32)
$sha=[Security.Cryptography.SHA256]::Create()
$digest=([BitConverter]::ToString($sha.ComputeHash($pcm))).Replace('-','').ToLowerInvariant();$sha.Dispose()
$cues=@('attack-light','attack-heavy','attack-magic','impact-light','impact-heavy','summon','merge','ui-confirm','ui-reject','death')
$records=for($i=0;$i -lt $cues.Count;$i++){
    $file=$cues[$i]+'.pcm';[IO.File]::WriteAllBytes((Join-Path $bank $file),$pcm)
    @{cue=$cues[$i];frames=2;assetId=('a_000000005444{0:x4}' -f ($i+1));file=$file;bytes=$pcm.Length;sha256=$digest}
}
[IO.File]::WriteAllText((Join-Path $bank 'bank.json'),(@{version=1;sampleRate=48000;cues=@($records)} | ConvertTo-Json -Depth 5))
function Snapshot([string]$Directory){
    @(Get-ChildItem -LiteralPath $Directory -File -Recurse | ForEach-Object {
        $_.FullName.Substring($Directory.Length) + ':' + (Get-FileHash -LiteralPath $_.FullName).Hash
    } | Sort-Object) -join "`n"
}
$original=Snapshot $game
$script:caseNumber=0
function Run([string]$Executable,[string[]]$Arguments){
    $script:caseNumber++
    $p=[Diagnostics.Process]::new();$s=[Diagnostics.ProcessStartInfo]::new()
    $s.FileName=$Executable;$s.WorkingDirectory=$cwd;$s.UseShellExecute=$false;$s.CreateNoWindow=$true
    $s.RedirectStandardOutput=$true;$s.RedirectStandardError=$true
    foreach($a in $Arguments){$s.ArgumentList.Add($a)}
    $p.StartInfo=$s
    if(-not $p.Start()){throw 'Start failed'}
    $out=$p.StandardOutput.ReadToEndAsync();$err=$p.StandardError.ReadToEndAsync()
    if(-not $p.WaitForExit(30000)){$p.Kill($true);$p.WaitForExit();throw 'Owned runtime timed out'}
    $o=$out.GetAwaiter().GetResult();$e=$err.GetAwaiter().GetResult();$code=$p.ExitCode;$p.Dispose()
    [IO.File]::WriteAllText((Join-Path $root "case-$script:caseNumber.stdout"),$o)
    [IO.File]::WriteAllText((Join-Path $root "case-$script:caseNumber.stderr"),$e)
    [pscustomobject]@{Code=$code;Output=$o;Error=$e}
}
$direct=Run (Join-Path $game 'coop_defense.exe') @('--scene',(Join-Path $game 'game.scene.yaml'),'--audio','off','--audio-bank',$bank,'--headless','--ticks','60')
if($direct.Code -ne 0 -or [string]::IsNullOrWhiteSpace($direct.Output)){throw "Direct runtime failed: $($direct.Error)"}
$hosts=@((Get-Process -Id $PID).Path,(Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe')) | Select-Object -Unique
foreach($hostPath in $hosts){
    $run=Run $hostPath @('-NoProfile','-NonInteractive','-File',(Join-Path $game 'run-game.ps1'),'-Headless','-Ticks','60','-WithAudioBank')
    if($run.Code -ne 0 -or $run.Output -cne $direct.Output){throw "Copied launcher differs from direct runtime: $($run.Error)"}
}
if((Snapshot $game) -cne $original){throw 'Normal copied execution modified the game folder'}
# Both absolute targets are checked descendants of our fresh evidence directory.
$moved=Join-Path $root 'relocated game'
foreach($path in @($game,$moved)){
    if(-not [IO.Path]::GetFullPath($path).StartsWith($root+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Move escaped evidence root'}
}
Move-Item -LiteralPath $game -Destination $moved
$game=$moved
foreach($hostPath in $hosts){
    $run=Run $hostPath @('-NoProfile','-NonInteractive','-File',(Join-Path $game 'run-game.ps1'),'-Headless','-Ticks','60','-WithAudioBank')
    if($run.Code -ne 0 -or $run.Output -cne $direct.Output){throw "Relocated launcher differs: $($run.Error)"}
}
if((Snapshot $game) -cne $original){throw 'Normal relocated execution modified the game folder'}
# Explicit selection while Off must still reach real native bank admission.
[IO.File]::WriteAllText((Join-Path $game 'audio-bank/bank.json'),'invalid test manifest')
$bad=Run $hosts[0] @('-NoProfile','-NonInteractive','-File',(Join-Path $game 'run-game.ps1'),'-Headless','-WithAudioBank')
if($bad.Code -eq 0){throw 'Explicit invalid bank accepted while Off'}
[pscustomobject]@{schema=1;kind='real runtime CPU/headless';cases=$script:caseNumber;hosts=$hosts;runtime_sha256=(Get-FileHash -LiteralPath $Runtime).Hash;summary=$direct.Output;normal_game_inventory_hashes_unchanged=$true;window_device='UNRUN';settings_persistence='UNRUN by launcher smoke'} |
    ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $root 'result.json') -Encoding utf8
Write-Output "PASS real runtime cases=$script:caseNumber; copied/relocated headless 60; invalid selected bank refused; window/device UNRUN."
