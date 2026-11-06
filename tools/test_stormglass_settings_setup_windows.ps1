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
if((Run @('--check',$settings)) -ne 0){throw 'Empty private settings admission failed'}
if(-not ([IO.File]::ReadAllText((Join-Path $root "case-$script:caseNumber.stdout"))).Contains('audio=defaults controls=defaults')){throw 'Default profile report missing'}
if(@(Get-ChildItem -LiteralPath $settings -Force).Count -ne 0){throw 'Read-only check created files'}
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
if((Run @('--check',$settings)) -ne 0 -or (Get-Acl -LiteralPath $settings).Sddl -cne $before -or [IO.File]::ReadAllText((Join-Path $settings 'sentinel.txt')) -cne 'preserve existing bytes'){throw 'Read-only check modified unrelated data/ACL'}
$existing=Join-Path $root 'existing broad directory'
New-Item -ItemType Directory -Path $existing | Out-Null
$world=[Security.Principal.SecurityIdentifier]::new('S-1-1-0')
$broad=Get-Acl -LiteralPath $existing
$broad.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($world,[Security.AccessControl.FileSystemRights]::FullControl,[Security.AccessControl.AccessControlType]::Allow))
Set-Acl -LiteralPath $existing -AclObject $broad
$broadAcl=(Get-Acl -LiteralPath $existing).Sddl
if((Run @('--directory',$existing)) -eq 0 -or (Get-Acl -LiteralPath $existing).Sddl -cne $broadAcl){throw 'Existing broad ACL repaired'}
if((Run @('--check',$existing)) -eq 0 -or (Get-Acl -LiteralPath $existing).Sddl -cne $broadAcl){throw 'Broad ACL admitted or repaired by check'}
$file=Join-Path $root 'existing-file'
[IO.File]::WriteAllText($file,'unchanged')
if((Run @('--directory',$file)) -eq 0 -or [IO.File]::ReadAllText($file) -cne 'unchanged'){throw 'Existing file modified'}
$missing=Join-Path $root 'missing'
if((Run @('--check',$missing)) -eq 0 -or (Test-Path -LiteralPath $missing)){throw 'Check created a missing directory'}
$inside=Join-Path $game 'must-not-create'
foreach($argsForCase in @(
    @('--directory','relative'),@('--directory','C:relative'),@('--directory',([IO.Path]::GetPathRoot($root))),
    @('--directory',$inside),@('--directory',(Join-Path $missing 'child')),
    @('--unknown',$settings),@('--directory',$settings,'--directory',$settings)
)){
    if((Run $argsForCase) -eq 0){throw 'Invalid request accepted'}
}
if((Test-Path -LiteralPath $missing) -or (Test-Path -LiteralPath $inside)){throw 'Refused request created a directory'}
function Write-PrivateFixture([string]$Path,[string]$Text){
    $fileAcl=[Security.AccessControl.FileSecurity]::new()
    # Create a fresh fixture with a private DACL from the outset, without Set-Acl.
    $fileAcl.SetSecurityDescriptorSddlForm($before,[Security.AccessControl.AccessControlSections]::Access)
    # Elevated Windows tokens may default ownership to Administrators. Match production's
    # explicit current-user owner at creation, rather than weakening native admission.
    $fileAcl.SetOwner([Security.Principal.WindowsIdentity]::GetCurrent().User)
    $stream=[IO.FileSystemAclExtensions]::Create([IO.FileInfo]::new($Path),[IO.FileMode]::CreateNew,[Security.AccessControl.FileSystemRights]::FullControl,[IO.FileShare]::Read,4096,[IO.FileOptions]::None,$fileAcl)
    try {$bytes=[Text.Encoding]::UTF8.GetBytes($Text);$stream.Write($bytes,0,$bytes.Length)} finally {$stream.Dispose()}
    if((Get-Acl -LiteralPath $Path).GetOwner([Security.Principal.SecurityIdentifier]).Value -ne $user){throw 'Fixture owner does not match current user'}
}
$audioFile=Join-Path $settings 'audio-settings.json'
Write-PrivateFixture $audioFile '{"schema":1,"profile":"stormglass-audio-v1","gain_percent":37,"muted":false}'
$validHash=(Get-FileHash -LiteralPath $audioFile).Hash
if((Run @('--check',$settings)) -ne 0 -or (Get-FileHash -LiteralPath $audioFile).Hash -cne $validHash){throw 'Valid saved audio admission failed or modified file'}
if(-not ([IO.File]::ReadAllText((Join-Path $root "case-$script:caseNumber.stdout"))).Contains('audio=valid-file controls=defaults')){throw 'Saved/default distinction missing'}
Move-Item -LiteralPath $audioFile -Destination (Join-Path $settings 'audio-valid.retained')
foreach($profile in @('audio','controls')){
    $path=Join-Path $settings "$profile-settings.json"
    Write-PrivateFixture $path '{malformed private profile'
    $oldHash=(Get-FileHash -LiteralPath $path).Hash;$oldAcl=(Get-Acl -LiteralPath $path).Sddl
    $oldNames=@(Get-ChildItem -LiteralPath $settings -Force | Select-Object -ExpandProperty Name | Sort-Object) -join "`n"
    if((Run @('--check',$settings)) -eq 0){throw 'Malformed profile accepted'}
    if(-not ([IO.File]::ReadAllText((Join-Path $root "case-$script:caseNumber.stderr"))).Contains($profile)){throw 'Invalid profile not identified'}
    $newNames=@(Get-ChildItem -LiteralPath $settings -Force | Select-Object -ExpandProperty Name | Sort-Object) -join "`n"
    if((Get-FileHash -LiteralPath $path).Hash -cne $oldHash -or (Get-Acl -LiteralPath $path).Sddl -cne $oldAcl -or $newNames -cne $oldNames){throw 'Read-only check changed bytes/ACL/inventory'}
    # Preserve deliberate malformed fixtures under unrelated names before the next profile.
    Move-Item -LiteralPath $path -Destination (Join-Path $settings "$profile-malformed.retained")
}
[pscustomobject]@{schema=1;cases=$script:caseNumber;kind='real Windows CLI and private NTFS ACL';executable_sha256=(Get-FileHash -LiteralPath $Executable).Hash;private_owner=$owner;acl=$before;window_device='UNRUN'} |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'result.json') -Encoding utf8
Write-Output "PASS settings setup/check CLI cases=$script:caseNumber; private creation and read-only saved/default admission, ACL/data preservation; window/device UNRUN."
