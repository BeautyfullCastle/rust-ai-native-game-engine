param([Parameter(Mandatory = $true)][string]$EvidenceDirectory)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'This focused gate requires Windows.' }
$repoPath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$evidencePath = [IO.Path]::GetFullPath($EvidenceDirectory)
if ($evidencePath.Equals($repoPath, [StringComparison]::OrdinalIgnoreCase) -or
    $evidencePath.StartsWith($repoPath + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Keep evidence outside the source checkout.'
}
if (Test-Path -LiteralPath $evidencePath) { throw 'Choose a fresh evidence directory; earlier logs are preserved.' }
New-Item -ItemType Directory -Path $evidencePath | Out-Null
Push-Location $repoPath
try {
    $dirty = git status --porcelain
    if ($LASTEXITCODE -ne 0 -or $dirty) { throw 'Commit the exact candidate before running this gate.' }
    git rev-parse HEAD 'HEAD^{tree}' | Set-Content -LiteralPath (Join-Path $evidencePath 'source.txt') -Encoding utf8
    if ($LASTEXITCODE -ne 0) { throw 'Could not record source identity.' }
    rustc --version --verbose | Set-Content -LiteralPath (Join-Path $evidencePath 'rustc.txt') -Encoding utf8
    if ($LASTEXITCODE -ne 0) { throw 'Could not record toolchain identity.' }
    $features = 'coop-defense-controls,coop-defense-attack-clip,coop-defense-run-save'
    function Invoke-FocusedCheck([string]$Name, [string[]]$CommandArgs, [string]$Summary) {
        $logPath = Join-Path $evidencePath ($Name + '.log')
        $priorPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            & cargo @CommandArgs 2>&1 | Tee-Object -FilePath $logPath
            $nativeExit = $LASTEXITCODE
        } finally { $ErrorActionPreference = $priorPreference }
        if ($nativeExit -ne 0) { throw "$Name failed; original output preserved in $logPath" }
        if ($Summary -and -not ([IO.File]::ReadAllText($logPath).Contains($Summary))) { throw "$Name did not execute the expected test inventory." }
    }
    Invoke-FocusedCheck 'controls-model' @('test','--locked','-p','orr_sample','--features',$features,'--lib','defense_controls::tests::','--','--test-threads=1') 'test result: ok. 13 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'controls-ui' @('test','--locked','-p','orr_sample','--features',$features,'--lib','defense_ui::controls_ui_tests::','--','--test-threads=1') 'test result: ok. 9 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'controls-app' @('test','--locked','-p','orr_sample','--features',$features,'--lib','defense_app::controls_tests::','--','--test-threads=1') 'test result: ok. 5 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'audio-storage' @('test','--locked','-p','orr_sample','--features',$features,'--lib','audio_settings','--','--test-threads=1') 'test result: ok. 21 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'pause' @('test','--locked','-p','orr_sample','--features',$features,'--lib','defense_app::pause_tests::','--','--test-threads=1') 'test result: ok. 14 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'modifiers' @('test','--locked','-p','orr_sample','--features',$features,'--lib','defense_ui::modifier_tests::','--','--test-threads=1') 'test result: ok. 9 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'legacy-ui' @('test','--locked','-p','orr_sample','--features','coop-defense-controls','--lib','defense_ui::tests::','--','--test-threads=1') 'test result: ok. 4 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'bank-app' @('test','--locked','-p','orr_sample','--features',$features,'--lib','windows_bank_tests','--','--test-threads=1') 'test result: ok. 3 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'feature-off' @('test','--locked','-p','orr_sample','--features','coop-defense','--lib','controls_flag_feature_off','--','--test-threads=1') 'test result: ok. 1 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'clippy' @('clippy','--locked','-p','orr_sample','--features',$features,'--lib','--bin','coop_defense','--','-D','warnings') ''
    Invoke-FocusedCheck 'native-clippy' @('clippy','--locked','-p','orr_sample','--features',($features+',audio-native'),'--lib','--bin','coop_defense','--','-D','warnings') ''
    Invoke-FocusedCheck 'audio-only-clippy' @('clippy','--locked','-p','orr_sample','--features','coop-defense-audio-settings,coop-defense-run-save,audio-native','--lib','--bin','coop_defense','--','-D','warnings') ''
    'PASS: focused controls filesystem/CPU gate; no native window/device acceptance.' |
        Set-Content -LiteralPath (Join-Path $evidencePath 'result.txt') -Encoding utf8
} finally { Pop-Location }
