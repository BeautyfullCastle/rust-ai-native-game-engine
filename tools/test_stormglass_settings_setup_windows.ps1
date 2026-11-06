param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory
)
$ErrorActionPreference='Stop'
if($env:OS -ne 'Windows_NT'){throw 'Windows required'}
$root=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $root){throw 'Choose fresh evidence'}
New-Item -ItemType Directory -Path $root | Out-Null
$game=Join-Path $root 'game [settings] & space'
$cwd=Join-Path $root 'other cwd'
New-Item -ItemType Directory -Path $game,$cwd | Out-Null
$copied=Join-Path $game 'orr_prepare_stormglass_settings.exe'
Copy-Item -LiteralPath $Executable -Destination $copied
$script:caseNumber=0
function Run([string[]]$Arguments){
    $script:caseNumber++
    $s=[Diagnostics.ProcessStartInfo]::new()
    $s.FileName=$copied;$s.WorkingDirectory=$cwd;$s.UseShellExecute=$false;$s.CreateNoWindow=$true
    $s.RedirectStandardOutput=$true;$s.RedirectStandardError=$true
    foreach($a in $Arguments){$s.ArgumentList.Add($a)}
    $p=[Diagnostics.Process]::new();$p.StartInfo=$s
    if(-not $p.Start()){throw 'Start failed'}
    $o=$p.StandardOutput.ReadToEndAsync();$e=$p.StandardError.ReadToEndAsync()
    if(-not $p.WaitForExit(15000)){$p.Kill($true);$p.WaitForExit();throw 'Owned setup process timed out'}
    $stdout=$o.GetAwaiter().GetResult();$stderr=$e.GetAwaiter().GetResult();$code=$p.ExitCode;$p.Dispose()
    [IO.File]::WriteAllText((Join-Path $root "case-$script:caseNumber.stdout"),$stdout)
    [IO.File]::WriteAllText((Join-Path $root "case-$script:caseNumber.stderr"),$stderr)
    $code
}
if((Run @('--help')) -ne 0){throw 'Help failed'}
$settings=Join-Path $root 'private & [controls]'
if((Run @('--directory',$settings)) -ne 0){throw 'Private creation failed'}
if(@(Get-ChildItem -LiteralPath $settings -Force).Count -ne 0){throw 'Setup created settings data'}
$acl=Get-Acl -LiteralPath $settings
$user=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$owner=$acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
if($owner -ne $user -or -not $acl.AreAccessRulesProtected){throw 'Owner/inheritance mismatch'}
$rules=@($acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier]))
if($rules.Count -ne 3){throw 'Unexpected ACL inventory'}
foreach($r in $rules){
    if($r.IdentityReference.Value -notin @($user,'S-1-5-18','S-1-5-32-544') -or
        $r.AccessControlType -ne 'Allow' -or $r.IsInherited -or
        $r.FileSystemRights -ne [Security.AccessControl.FileSystemRights]::FullControl){throw 'Unexpected ACL grant'}
}
$before=$acl.Sddl
[IO.File]::WriteAllText((Join-Path $settings 'sentinel.txt'),'preserve existing bytes')
if((Run @('--directory',$settings)) -eq 0){throw 'Existing directory accepted'}
if((Get-Acl -LiteralPath $settings).Sddl -cne $before -or [IO.File]::ReadAllText((Join-Path $settings 'sentinel.txt')) -cne 'preserve existing bytes'){throw 'Existing directory modified'}
$existing=Join-Path $root 'existing broad directory'
New-Item -ItemType Directory -Path $existing | Out-Null
$broadAcl=(Get-Acl -LiteralPath $existing).Sddl
if((Run @('--directory',$existing)) -eq 0 -or (Get-Acl -LiteralPath $existing).Sddl -cne $broadAcl){throw 'Existing broad ACL repaired'}
$file=Join-Path $root 'existing-file'
[IO.File]::WriteAllText($file,'unchanged')
if((Run @('--directory',$file)) -eq 0 -or [IO.File]::ReadAllText($file) -cne 'unchanged'){throw 'Existing file modified'}
$missing=Join-Path $root 'missing'
$inside=Join-Path $game 'must-not-create'
foreach($argsForCase in @(
    @('--directory','relative'),@('--directory','C:relative'),@('--directory',([IO.Path]::GetPathRoot($root))),
    @('--directory',$inside),@('--directory',(Join-Path $missing 'child')),
    @('--unknown',$settings),@('--directory',$settings,'--directory',$settings)
)){
    if((Run $argsForCase) -eq 0){throw 'Invalid request accepted'}
}
if((Test-Path -LiteralPath $missing) -or (Test-Path -LiteralPath $inside)){throw 'Refused request created a directory'}
[pscustomobject]@{schema=1;cases=$script:caseNumber;kind='real Windows CLI and private NTFS ACL';executable_sha256=(Get-FileHash -LiteralPath $Executable).Hash;private_owner=$owner;acl=$before;window_device='UNRUN'} |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'result.json') -Encoding utf8
Write-Output "PASS settings setup CLI cases=$script:caseNumber; private empty creation and existing ACL/data preserved; window/device UNRUN."
