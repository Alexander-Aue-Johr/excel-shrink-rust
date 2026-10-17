# Focused diagnostics only. Never attach to Excel via COM or dismiss a dialog.
function New-VVFocusObserver([string]$Root,[string]$Workbook,$Budgets) {
    return @{Root=$Root;LastSaved=[DateTime]::MinValue;Process=$null;LastProcessSnapshot=$null;DetachedSnapshot=$null;State=[ordered]@{
        schema='vv-focus-lifecycle-v1';workbook=$Workbook;phase='setup';phase_started_utc=[DateTime]::UtcNow.ToString('o');
        updated_utc=[DateTime]::UtcNow.ToString('o');milestones=[ordered]@{};budgets=$Budgets;
        last_event=$null;last_sheet=$null;last_address=$null;last_native_start=$null;
        normal_images_received=0;calibration_images_received=0;timeout_kind=$null;error=$null;process=$null;capture_status=$null;result_transport_status='pending'}}
}
function Get-VVFocusProcessSnapshot($Process) {
    if($null -eq $Process){return @{available=$false;reason='No owned Excel launcher process receipt'}}
    try {
        $Process.Refresh()
        $snapshot=[ordered]@{available=$true;pid=$Process.Id;has_exited=$Process.HasExited;sampled_utc=[DateTime]::UtcNow.ToString('o')}
        if(-not $Process.HasExited){
            $snapshot.name=$Process.ProcessName
            $snapshot.start_ticks=$Process.StartTime.ToUniversalTime().Ticks.ToString()
            $snapshot.cpu_seconds=$Process.TotalProcessorTime.TotalSeconds
            $snapshot.working_set_bytes=$Process.WorkingSet64
            $snapshot.private_bytes=$Process.PrivateMemorySize64
            $snapshot.main_window_handle=$Process.MainWindowHandle.ToInt64().ToString()
            $snapshot.main_window_title=$Process.MainWindowTitle
            # Responding/MainWindowTitle are OS process properties, not Excel automation.
            $snapshot.responding=$Process.Responding
        }
        return $snapshot
    } catch {return @{available=$false;reason=$_.Exception.Message}}
}
function Save-VVFocusObserver($Observer,[switch]$Force) {
    if($null -eq $Observer){return}
    $now=[DateTime]::UtcNow
    if(-not $Force -and ($now-$Observer.LastSaved).TotalSeconds -lt 15){return}
    $Observer.State.updated_utc=$now.ToString('o')
    if($null -ne $Observer.Process){
        $sample=Get-VVFocusProcessSnapshot $Observer.Process
        if($sample.available){$Observer.LastProcessSnapshot=$sample}
        $Observer.State.process=$sample
    } elseif($null -ne $Observer.DetachedSnapshot){$Observer.State.process=$Observer.DetachedSnapshot}
    else{$Observer.State.process=Get-VVFocusProcessSnapshot $null}
    Write-VVJson (Join-Path $Observer.Root 'capture-lifecycle.json') $Observer.State
    # Append small snapshots separately; no screenshots or unrelated window contents.
    [IO.File]::AppendAllText((Join-Path $Observer.Root 'capture-lifecycle.jsonl'),
        ($Observer.State|ConvertTo-Json -Depth 12 -Compress)+"`n",[Text.UTF8Encoding]::new($false))
    $Observer.LastSaved=$now
}
function Detach-VVFocusProcess($Observer) {
    if($null -eq $Observer){return}
    if($null -ne $Observer.Process){
        $sample=Get-VVFocusProcessSnapshot $Observer.Process
        if($sample.available){$Observer.LastProcessSnapshot=$sample}
        $Observer.DetachedSnapshot=@{available=$sample.available;status=$(if($sample.available -and $sample.has_exited){'process-exited'}else{'process-handle-released'});
            observed_utc=[DateTime]::UtcNow.ToString('o');last_successful_sample=$Observer.LastProcessSnapshot;final_sample=$sample}
    }
    $Observer.Process=$null
    Save-VVFocusObserver $Observer -Force
}
function Complete-VVFocusResult($Observer,[string]$CaptureStatus) {
    if($null -eq $Observer){return}
    $Observer.State.capture_status=$CaptureStatus
    $Observer.State.result_transport_status='received'
    Set-VVFocusPhase $Observer 'result-received'
}
function Set-VVFocusPhase($Observer,[string]$Phase,[string]$Reason='') {
    if($null -eq $Observer){return}
    $now=[DateTime]::UtcNow.ToString('o')
    if($Observer.State.phase -cne $Phase){$Observer.State.phase=$Phase;$Observer.State.phase_started_utc=$now}
    $Observer.State.milestones[$Phase]=$now
    if($Reason){$Observer.State.error=$Reason}
    Save-VVFocusObserver $Observer -Force
}
function Update-VVFocusEvent($Observer,$Event) {
    if($null -eq $Observer){return}
    $stage=[string]$Event.stage;$detail=$Event.detail
    $Observer.State.last_event=@{stage=$stage;received_utc=[DateTime]::UtcNow.ToString('o');sequence=$Event.sequence}
    if($null -ne $detail){
        if($detail.sheet){$Observer.State.last_sheet=[string]$detail.sheet}
        if($detail.address){$Observer.State.last_address=[string]$detail.address}
        if($stage -eq 'image_batch_persisted'){
            foreach($c in @($detail.captures)){
                if([string]$c.id -like 'cal*'){$Observer.State.calibration_images_received++}
                elseif([string]$c.id -match '^c\d+-p\d+-t'){$Observer.State.normal_images_received++}
            }
        }
        if($stage -in @('tile_render_started','axis_geometry_started','calibration_render_started','sparse_render_started','context_reference_render_started')){
            $Observer.State.last_native_start=@{stage=$stage;detail=$detail;received_utc=[DateTime]::UtcNow.ToString('o')}
        }
    }
    if($stage -in @('bootstrap_loaded','office_ready','shell_loaded','manual_start_waiting','measurement_started','result_received')){
        $Observer.State.milestones[$stage]=[DateTime]::UtcNow.ToString('o')
        if($stage -eq 'measurement_started' -and $Observer.State.phase -eq 'startup'){Set-VVFocusPhase $Observer 'capture'}
        else{Save-VVFocusObserver $Observer -Force}
    }
}
function Stop-VVFocusForTimeout($Observer,[string]$Kind,[string]$Message) {
    if($null -eq $Observer){return}
    $Observer.State.timeout_kind=$Kind
    Set-VVFocusPhase $Observer 'timeout' $Message
}
