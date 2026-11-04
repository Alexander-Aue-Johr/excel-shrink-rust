$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot '../TargetedResident.psm1') -Force -DisableNameChecking
$module=Get-Module TargetedResident
& $module {
    foreach($name in @('We found a problem with some content in book.xlsx.','Wir haben ein Problem bei einigen Inhalten in one.xlsx erkannt.')) {
        if((Get-TargetedOpeningDialogKind @($name)) -ne 'repair_required'){throw 'Repair prompt not recognized.'}
    }
    if($null -ne (Get-TargetedOpeningDialogKind @('Save changes?','Ja','Nein'))){throw 'Unknown prompt classified as repair.'}
}
Add-Type -Path (Join-Path $PSScriptRoot '../TargetedOpeningDialogs.cs')
if([TargetedOpeningDialogs]::Find([int]::MaxValue).Length -ne 0){throw 'Unowned dialog returned.'}
'Opening dialog classification passed.'