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
    if ($LASTEXITCODE -ne 0) { throw 'Could not record Rust version.' }
    $features = 'coop-defense-audio-settings,coop-defense-attack-clip,coop-defense-run-save'
    function Invoke-FocusedCheck([string]$Name, [string[]]$CommandArgs, [string]$Summary) {
        $logPath = Join-Path $evidencePath ($Name + '.log')
        $priorPreference = $ErrorActionPreference
        try {
            # Windows PowerShell represents native stderr as ErrorRecord even on success.
            # Capture it verbatim, then require the actual native exit status.
            $ErrorActionPreference = 'Continue'
            & cargo @CommandArgs 2>&1 | Tee-Object -FilePath $logPath
            $nativeExit = $LASTEXITCODE
        } finally { $ErrorActionPreference = $priorPreference }
        if ($nativeExit -ne 0) { throw "$Name failed; original output preserved in $logPath" }
        if ($Summary -and -not ([IO.File]::ReadAllText($logPath).Contains($Summary))) {
            throw "$Name did not execute the expected test inventory."
        }
    }
    Invoke-FocusedCheck 'settings' @('test','--locked','-p','orr_sample','--features',$features,'--lib','audio_settings','--','--test-threads=1') 'test result: ok. 21 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'ui' @('test','--locked','-p','orr_sample','--features',$features,'--lib','defense_ui::audio_ui_tests','--','--test-threads=1') 'test result: ok. 6 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'app' @('test','--locked','-p','orr_sample','--features',$features,'--lib','defense_app::audio_tests','--','--test-threads=1') 'test result: ok. 5 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'pcm' @('test','--locked','-p','orr_sample','--features',$features,'--lib','defense_audio::tests','--','--test-threads=1') 'test result: ok. 5 passed; 0 failed; 0 ignored;'
    Invoke-FocusedCheck 'clippy' @('clippy','--locked','-p','orr_sample','--features',$features,'--lib','--bin','coop_defense','--','-D','warnings') ''
    Invoke-FocusedCheck 'default' @('check','--locked','-p','orr_sample','--features','coop-defense','--lib') ''
    'PASS: focused CPU/filesystem gate; no native window or hardware audio acceptance.' |
        Set-Content -LiteralPath (Join-Path $evidencePath 'result.txt') -Encoding utf8
} finally { Pop-Location }
