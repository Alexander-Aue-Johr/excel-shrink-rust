$script:VVDecisionExtrasRoot=$PSScriptRoot
# Optional offline decision projection. Existing capture measurements are not changed.
function Add-VVDecisionEvidence($Metrics,[string]$CaseRoot,$Spec) {
    $canonical=@()
    if($Spec.job.candidate.kind -eq 'capture' -and $Spec.job.phase -ne 'warmup') {
        try {
            Add-Type -AssemblyName System.Drawing -ErrorAction Stop
            if($null -eq ('VVStripDigest' -as [type])) {
                Add-Type -Path (Join-Path $script:VVDecisionExtrasRoot 'StripDigest.cs') -ReferencedAssemblies @('System.dll','System.Drawing.dll') -ErrorAction Stop
            }
            foreach($group in @($Metrics.captures | Group-Object workbook,sheet_index,sheet)) {
                $first=$group.Group[0]
                $result=[ordered]@{sheet=$first.sheet;sheet_index=$first.sheet_index;workbook=$first.workbook;status='unverified';algorithm=$null;sha256=$null;width=$null;height=$null;error=$null;tiles=$group.Count;overlap_rows_checked=$null}
                try {
                    $strips=[Collections.Generic.List[VVStripSpec]]::new()
                    foreach($tile in $group.Group) {
                        if($null -eq $tile.pixel_digest -or $tile.pixel_issue){throw 'Individual tile pixel verification did not complete.'}
                        if([string]::IsNullOrWhiteSpace($tile.path) -or $tile.path -match '(^[\\/]|:|(^|[\\/])\.\.([\\/]|$))'){throw 'Unsafe tile evidence path'}
                        foreach($value in @($tile.offset.x,$tile.offset.y,$tile.width,$tile.height)){
                            if($null -eq $value -or [double]$value -ne [Math]::Truncate([double]$value)){throw 'Non-integer tile raster geometry'}
                        }
                        $s=[VVStripSpec]::new();$s.Path=Join-Path $CaseRoot $tile.path;$s.Sha256=$tile.encoded_sha256
                        $s.X=[int]$tile.offset.x;$s.Y=[int]$tile.offset.y;$s.Width=[int]$tile.width;$s.Height=[int]$tile.height
                        $strips.Add($s)
                    }
                    $d=[VVStripDigest]::Read($strips.ToArray(),64000000)
                    $result.status='complete';$result.algorithm=$d.Algorithm;$result.sha256=$d.Sha256
                    $result.width=$d.Width;$result.height=$d.Height;$result.overlap_rows_checked=$d.OverlapRowsChecked
                } catch {$result.error=$_.Exception.Message}
                $canonical += [pscustomobject]$result
            }
        } catch {
            $canonical+= [pscustomobject]@{sheet='';sheet_index=0;status='unverified';error=$_.Exception.Message}
        }
    }
    $Metrics|Add-Member -NotePropertyName canonical_sheets -NotePropertyValue @($canonical) -Force
    if($Spec.job.candidate.kind -like 'lab-*') {
        $summary=Read-VVJson (Join-Path $CaseRoot 'native-summary.json')
        if($null -eq $summary){$Metrics.status='failed';$Metrics.errors+= 'Missing native summary'}
        else{$Metrics|Add-Member -NotePropertyName native_summary -NotePropertyValue $summary -Force;$Metrics.status=$summary.status}
    }
    $reference=@()
    if($Spec.job.candidate.kind -like 'shape-*') {
        foreach($file in @(Get-VVChildItem -LiteralPath (Join-Path $CaseRoot 'data') -File -Recurse -Filter '*.json')){
            if($file.Name -in @('timing-summary.json','geometry-validation.json')){
                $reference += [pscustomobject]@{path=(Get-VVRelative $CaseRoot $file.FullName);name=$file.Name;data=(Read-VVJson $file.FullName)}
            }
        }
    }
    $Metrics|Add-Member -NotePropertyName reference_details -NotePropertyValue @($reference) -Force
    return $Metrics
}
