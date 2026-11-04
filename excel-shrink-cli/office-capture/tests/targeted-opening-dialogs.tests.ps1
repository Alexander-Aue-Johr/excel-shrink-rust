$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot '../TargetedResident.psm1') -Force -DisableNameChecking
$module=Get-Module TargetedResident
& $module {
    foreach($name in @('We found a problem with some content in book.xlsx.','Wir haben ein Problem bei einigen Inhalten in one.xlsx erkannt.')) {
        if((Get-TargetedOpeningDialogKind @($name)) -ne 'repair_required'){throw 'Repair prompt not recognized.'}
    }
    $cases = @(
        @{text='Password required';kind='password_required'},
        @{text='Microsoft Excel - Compatibility Checker';kind='compatibility_confirmation_required'},
        @{text=('Kompatibilit'+[char]0x00e4+'tspr'+[char]0x00fc+'fung');kind='compatibility_confirmation_required'},
        @{text='File is locked for editing by another user';kind='file_locked'},
        @{text='Open read-only?';kind='read_only_prompt'},
        @{text=('Datei ist schreibgesch'+[char]0x00fc+'tzt');kind='read_only_prompt'},
        @{text='This workbook contains links to other data sources';kind='external_links_prompt'},
        @{text='The file format and extension do not match';kind='format_mismatch'},
        @{text='There are circular references';kind='circular_reference'},
        @{text='Cannot open this workbook';kind='file_open_error'},
        @{text='Security warning';kind='security_prompt'},
        @{text='Not enough memory';kind='resource_error'}
    )
    foreach($case in $cases) {
        if((Get-TargetedOpeningDialogKind @($case.text)) -ne $case.kind){throw "Wrong dialog category: $($case.text)"}
    }
    if((Get-TargetedOpeningDialogKind @('Microsoft Excel') $true) -ne 'password_required'){throw 'A native password input must be recognized without reading it.'}
    if($null -ne (Get-TargetedOpeningDialogKind @('Save changes?','Ja','Nein'))){throw 'Unknown prompt classified as repair.'}
}
Add-Type -Path (Join-Path $PSScriptRoot '../TargetedOpeningDialogs.cs')
if([TargetedOpeningDialogs]::Find([int]::MaxValue).Length -ne 0){throw 'Unowned dialog returned.'}
'Opening dialog classification passed.'