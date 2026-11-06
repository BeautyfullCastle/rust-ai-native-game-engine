# Copy beside a trusted coop_defense.exe and game.scene.yaml. Not a package authenticator.
[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$Headless,
    [ValidatePattern('^[0-9]{1,5}$')][string]$Ticks = '0',
    [ValidateSet('off','auto','required')][string]$Audio = 'off',
    [switch]$WithAudioBank,
    [string]$AudioBankDirectory,
    [string]$UserSettingsDirectory,
    [switch]$PrepareUserSettings,
    [string]$ControlsDirectory,
    [string]$AudioSettingsDirectory
)
$ErrorActionPreference = 'Stop'
try {
    if ($env:OS -ne 'Windows_NT') { throw 'This launcher requires Windows.' }
    if ([int]$Ticks -gt 36000) { throw 'Ticks must be in 0..36000.' }
    if (-not $Headless -and $PSBoundParameters.ContainsKey('Ticks')) { throw 'Ticks requires Headless.' }
    $audioMode = $Audio.ToLowerInvariant()
    $sharedSettings = $PSBoundParameters.ContainsKey('UserSettingsDirectory')
    if ($PrepareUserSettings -and -not $sharedSettings) { throw 'PrepareUserSettings requires UserSettingsDirectory.' }
    if ($sharedSettings -and ($PSBoundParameters.ContainsKey('ControlsDirectory') -or $PSBoundParameters.ContainsKey('AudioSettingsDirectory'))) {
        throw 'Choose UserSettingsDirectory or the individual settings directories, not both.'
    }
    $externalBank = $PSBoundParameters.ContainsKey('AudioBankDirectory')
    if ($externalBank -and $WithAudioBank) { throw 'Choose AudioBankDirectory or WithAudioBank, not both.' }
    if ($externalBank -and ([string]::IsNullOrWhiteSpace($AudioBankDirectory) -or $AudioBankDirectory -notmatch '^[A-Za-z]:\\')) {
        throw 'AudioBankDirectory needs an explicit absolute local drive directory.'
    }
    if ($Headless -and $audioMode -eq 'required') { throw 'Headless does not support required audio.' }
    if ($Headless -and ($sharedSettings -or $PSBoundParameters.ContainsKey('ControlsDirectory') -or $PSBoundParameters.ContainsKey('AudioSettingsDirectory'))) {
        throw 'User settings require interactive play.'
    }
    $root = [IO.Path]::GetFullPath($PSScriptRoot)
    $runtime = Join-Path $root 'coop_defense.exe'
    $scene = Join-Path $root 'game.scene.yaml'
    foreach ($inputPath in @($runtime,$scene)) {
        $item = Get-Item -LiteralPath $inputPath -Force
        if ($null -eq $item -or $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'The fixed runtime and scene must be regular files, not reparse points.'
        }
    }
    $nativeArgs = @('--scene',$scene,'--audio',$audioMode)
    if ($externalBank) {
        # Forward the literal selection, including missing/invalid paths and Off mode.
        # Do not normalize aliases, create/copy files, or fall back to the sibling bank.
        $nativeArgs += @('--audio-bank',$AudioBankDirectory)
    } elseif ($WithAudioBank -or $audioMode -ne 'off') {
        # Always forward an explicitly selected bank, even when missing/invalid or audio is Off.
        # The native bounded admission remains authoritative; it never follows manifest paths.
        $nativeArgs += @('--audio-bank',(Join-Path $root 'audio-bank'))
    }
    foreach ($setting in @(@('ControlsDirectory','--controls-directory'),@('AudioSettingsDirectory','--audio-settings-directory'))) {
        if ($sharedSettings -or $PSBoundParameters.ContainsKey($setting[0])) {
            $value = if ($sharedSettings) { $UserSettingsDirectory } else { [string]$PSBoundParameters[$setting[0]] }
            if ([string]::IsNullOrWhiteSpace($value) -or $value -notmatch '^[A-Za-z]:\\') {
                throw 'User settings need an explicit absolute local drive directory.'
            }
            $full = [IO.Path]::GetFullPath($value)
            if ($full.Equals($root,[StringComparison]::OrdinalIgnoreCase) -or
                $full.StartsWith($root + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) {
                throw 'Keep user settings outside the game folder.'
            }
            # Do not rewrite the supplied path: the runtime checks privacy, aliases and no-follow ancestry.
            $nativeArgs += @($setting[1],$value)
        }
    }
    if ($PrepareUserSettings) {
        # This explicit opt-in is the only creation path. The helper never repairs an existing object.
        $helper = Join-Path $root 'orr_prepare_stormglass_settings.exe'
        $item = Get-Item -LiteralPath $helper -Force
        if ($null -eq $item -or $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'The settings helper must be a regular sibling executable, not a reparse point.'
        }
        $priorErrorPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $PSNativeCommandUseErrorActionPreference = $false
            $global:LASTEXITCODE = $null
            & $helper '--directory' $UserSettingsDirectory
            $setupExit = $global:LASTEXITCODE
        } finally { $ErrorActionPreference = $priorErrorPreference }
        if ($null -eq $setupExit) { throw 'The settings helper could not be started.' }
        if ($setupExit -ne 0) { exit $setupExit }
        # Creation is not rolled back if native admission or the game subsequently fails.
    }
    if ($Headless) { $nativeArgs += @('--headless','--ticks',$Ticks) }
    # Literal executable path plus an argument array. No command-string evaluation or cwd changes.
    $priorErrorPreference = $ErrorActionPreference
    try {
        # Native stderr is not a PowerShell exception. Preserve it and the real exit status.
        $ErrorActionPreference = 'Continue'
        $PSNativeCommandUseErrorActionPreference = $false
        $global:LASTEXITCODE = $null
        & $runtime @nativeArgs
        $nativeExit = $global:LASTEXITCODE
    } finally { $ErrorActionPreference = $priorErrorPreference }
    if ($null -eq $nativeExit) { throw 'The runtime could not be started.' }
    exit $nativeExit
} catch {
    [Console]::Error.WriteLine('Stormglass launcher: ' + $_.Exception.Message)
    exit 1
}
