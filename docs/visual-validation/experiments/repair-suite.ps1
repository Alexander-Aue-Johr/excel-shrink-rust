param([string]$RunRoot,[switch]$NoOpen,[switch]$SkipPixels)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '_shared\reporting.ps1')
. (Join-Path $PSScriptRoot '_shared\suite\metrics.ps1')
Initialize-VVReportIO
$repo=Get-VVRepo $PSScriptRoot
$reports=Join-Path $repo 'docs\visual-validation\results'
if([string]::IsNullOrWhiteSpace($RunRoot)) {
    $candidate=@(Get-VVRunList $reports | Where-Object {
        $_.Meta.experiment -eq 'comparison-suite' -and -not $_.Active -and $_.Meta.status -ne 'running'
    } | Sort-Object Name -Descending | Select-Object -First 1)
    if($candidate.Count -eq 0){throw 'No completed, inactive comparison suite was found.'}
    $RunRoot=$candidate[0].Path
}
$RunRoot=[VVReportIO]::FullPath($RunRoot)
$expectedParent=[VVReportIO]::FullPath((Join-Path $reports 'runs'))
if((Split-Path -Parent $RunRoot) -ne $expectedParent){throw 'Repair is restricted to managed suite directories in this repository.'}
$meta=Read-VVJson (Join-Path $RunRoot 'run.json')
$state=Read-VVJson (Join-Path $RunRoot 'suite-state.json')
if($null -eq $meta -or $meta.schema -ne 'vv-report-v1' -or $meta.experiment -ne 'comparison-suite' -or $null -eq $state){
    throw 'Not a managed comparison-suite result.'
}
# Keep other report tools out while the existing evidence is being projected.
$lock=[IO.File]::Open((Join-Path $RunRoot '.active.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
$utf8=[Text.UTF8Encoding]::new($false)
$stamp=[DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,8)
$backup=Join-Path $repo ('.tmp\visual-validation\report-repair\'+$stamp)
[IO.Directory]::CreateDirectory($backup)|Out-Null
$record=[ordered]@{schema='vv-suite-repair-v1';run_id=$state.run_id;started_utc=[DateTime]::UtcNow.ToString('o');
    backup=$backup;capture_rerun=$false;cases=@();errors=@();
    note='Only report projections, offline verification and bundles are regenerated. Capture payloads, PNGs and workload_ms are not rewritten.'}
function Save-VVRepairOriginal([string]$Path,[string]$Relative) {
    if([VVReportIO]::FileExists($Path)) {
        $out=Join-Path $backup $Relative
        [IO.Directory]::CreateDirectory((Split-Path -Parent $out))|Out-Null
        $inputStream=[VVReportIO]::OpenRead($Path);$output=[IO.File]::Create($out)
        try{$null=[VVReportIO]::CopyAndHash($inputStream,$output)}finally{$output.Dispose();$inputStream.Dispose()}
    }
}
try {
    Write-Host ('REPAIRING EXISTING SUITE: '+$RunRoot)
    Write-Host 'No Excel process will be started; capture measurements will not be rerun.'
    foreach($name in @('suite-state.json','case-results.csv','result.json','run.json','summary.txt','bundle-error.txt','bundle-error.json')){
        Save-VVRepairOriginal (Join-Path $RunRoot $name) $name
    }
    $decoderAvailable=$false
    if(-not $SkipPixels) {
        try {
            Add-Type -AssemblyName System.Drawing -ErrorAction Stop
            if($null -eq ('VVSuitePixels' -as [type])){
                Add-Type -Path (Join-Path $PSScriptRoot '_shared\suite\SuitePixels.cs') -ReferencedAssemblies @('System.dll','System.Drawing.dll') -ErrorAction Stop
            }
            $decoderAvailable=$true
        }catch{$record.errors+=('Offline decoder unavailable: '+$_.Exception.Message)}
    }
    foreach($job in @($state.jobs)){
        if($job.id -notmatch '^\d{3}-[a-z0-9-]+$'){throw 'Invalid case ID.'}
        $caseRoot=Join-Path $RunRoot ('cases\'+$job.id)
        $caseLock=$null
        $item=[ordered]@{case=$job.id;status='pending';verified_pixels=0;unverified_pixels=0}
        try {
            $caseLock=[IO.File]::Open((Join-Path $caseRoot '.active.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
            $old=Read-VVJson (Join-Path $caseRoot 'case-metrics.json')
            if($null -eq $old){throw 'Case metrics are missing; capture is not rerun by this task.'}
            foreach($name in @('case-metrics.json','run.json','result.json','case-report-error.txt')){
                Save-VVRepairOriginal (Join-Path $caseRoot $name) ('cases\'+$job.id+'\'+$name)
            }
            $want=$decoderAvailable -and $job.phase -eq 'measured' -and $job.candidate.kind -eq 'capture' -and [bool]$state.config.execution.verify_final_pixels
            $new=Get-VVCaseMetrics -CaseRoot $caseRoot -Job $job -VerifyPixels:$want
            # Preserve historical execution status and elapsed workload, independently of this analysis.
            $new.status=$old.status
            foreach($field in @('workload_ms','invocation_error')){
                $new|Add-Member -NotePropertyName $field -NotePropertyValue $old.$field -Force
            }
            $new|Add-Member -NotePropertyName reanalysis_utc -NotePropertyValue ([DateTime]::UtcNow.ToString('o')) -Force
            if($job.phase -eq 'measured' -and [bool]$state.config.execution.verify_final_pixels -and -not $want) {
                $new.pixel_verification_requested=$true;$new.pixel_verification_status='unverified'
                $new.pixel_unverified_captures=@($new.captures).Count
            }
            Write-VVJson (Join-Path $caseRoot 'case-metrics.json') $new
            Complete-VVReport $caseRoot
            Connect-VVCaseReport $caseRoot
            $publication=[ordered]@{status='ready';repaired_utc=[DateTime]::UtcNow.ToString('o')}
            $new|Add-Member -NotePropertyName report_publication -NotePropertyValue $publication -Force
            Write-VVJson (Join-Path $caseRoot 'case-metrics.json') $new
            Export-VVMetadataBundle $caseRoot
            $job.metrics=$new
            $item.status='ready';$item.verified_pixels=$new.pixel_verified_captures;$item.unverified_pixels=$new.pixel_unverified_captures
            Write-Host ('REPAIRED: '+$job.id+' | pixel check: '+$new.pixel_verification_status)
        }catch {
            $item.status='failed';$item.error=$_.Exception.Message
            $record.errors+=($job.id+': '+$_.Exception.Message)
            try{Set-VVBundleStatus $caseRoot 'failed' $_.Exception.Message}catch{}
            Write-Warning ('Repair failed for '+$job.id+': '+$_.Exception.Message)
        }finally{if($null -ne $caseLock){$caseLock.Dispose()}}
        $record.cases+= $item
    }
    $state|Add-Member -NotePropertyName report_repaired_utc -NotePropertyValue ([DateTime]::UtcNow.ToString('o')) -Force
    Write-VVJson (Join-Path $RunRoot 'suite-state.json') $state
    $record.completed_utc=[DateTime]::UtcNow.ToString('o')
    Write-VVJson (Join-Path $RunRoot 'repair-summary.json') $record
    Complete-VVSuiteReport $RunRoot
    Export-VVMetadataBundle $RunRoot
    Update-VVIndex $reports
    Write-Host ('REPORT: '+(Join-Path $RunRoot 'report.html'))
    Write-Host ('REVIEW BUNDLE (NO IMAGES): '+(Join-Path $RunRoot 'review-metadata.zip'))
    Write-Host ('PREVIOUS METADATA BACKUP: '+$backup)
} finally {$lock.Dispose()}
if(-not $NoOpen){Invoke-Item -LiteralPath (Join-Path $RunRoot 'report.html')}
