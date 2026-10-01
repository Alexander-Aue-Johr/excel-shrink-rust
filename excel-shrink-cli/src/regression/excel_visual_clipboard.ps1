# Clipboard metadata only: never log clipboard text or image contents.
if (-not ('ExcelShrinkVisualClipboardNative' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ExcelShrinkVisualClipboardNative {
    [DllImport("user32.dll")] public static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] public static extern IntPtr GetClipboardOwner();
    [DllImport("user32.dll")] public static extern IntPtr GetOpenClipboardWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] public static extern int CountClipboardFormats();
    public static uint ProcessFor(IntPtr window) {
        uint process = 0;
        if (window != IntPtr.Zero) GetWindowThreadProcessId(window, out process);
        return process;
    }
}
'@
}
function Get-VisualClipboardSnapshot {
    return [pscustomobject]@{
        sequence = [ExcelShrinkVisualClipboardNative]::GetClipboardSequenceNumber()
        owner_pid = [ExcelShrinkVisualClipboardNative]::ProcessFor([ExcelShrinkVisualClipboardNative]::GetClipboardOwner())
        open_pid = [ExcelShrinkVisualClipboardNative]::ProcessFor([ExcelShrinkVisualClipboardNative]::GetOpenClipboardWindow())
        formats = [ExcelShrinkVisualClipboardNative]::CountClipboardFormats()
    }
}
function Test-VisualClipboardImageIdentity($beforeCopy, $beforeRead, $afterRead, [uint32]$expectedProcess) {
    # A change during GetImage can be Excel's delayed rendering, not another app.
    # Retry the READ against the new sequence instead of mislabelling it interference.
    return ($expectedProcess -ne 0 -and $beforeRead.owner_pid -eq $expectedProcess -and
        $afterRead.owner_pid -eq $expectedProcess -and $beforeRead.sequence -eq $afterRead.sequence -and
        $afterRead.sequence -ne $beforeCopy.sequence)
}
function Write-VisualClipboardState([string]$label, [int]$attempt, [string]$stage, $state, [string]$errorMessage = '') {
    Write-ProgressLine "clipboard diagnostic label='$label' attempt=$attempt stage=$stage sequence=$($state.sequence) owner_pid=$($state.owner_pid) open_pid=$($state.open_pid) formats=$($state.formats) error='$errorMessage'"
}
function Test-VisualRangeCaptureSplittableError([string]$message) {
    if ([string]::IsNullOrWhiteSpace($message)) { return $false }
    if ($message.Contains('[excel-shrink:range-copy-picture]')) { return $true }
    return $message -match '(?i)(memory|speicher|ressource|resource|not enough|nicht genug|nicht genügend|out of|8007000e|e_outofmemory)'
}

function Invoke-VisualClipboardImage([scriptblock]$copyAction, [string]$label, [uint32]$expectedProcess) {
    $lastError = 'No verified Excel image was available.'
    $total = [Diagnostics.Stopwatch]::StartNew()
    $consecutiveCopyPictureErrors = 0
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        $beforeCopy = Get-VisualClipboardSnapshot
        Write-VisualClipboardState $label $attempt 'before_copy' $beforeCopy
        try {
            Write-ProgressLine "copy picture $label attempt=$attempt"
            [void](& $copyAction)
            $consecutiveCopyPictureErrors = 0
        } catch {
            $lastError = $_.Exception.Message
            $code = '0x{0:X8}' -f ($_.Exception.HResult -band 0xffffffffL)
            $consecutiveCopyPictureErrors++
            Write-VisualClipboardState $label $attempt 'copy_picture_exception' (Get-VisualClipboardSnapshot) "$code $lastError"
            if ($consecutiveCopyPictureErrors -ge 3) {
                throw "[excel-shrink:range-copy-picture] CopyPicture failed for '$label' after $consecutiveCopyPictureErrors consecutive attempts: $code $lastError"
            }
            Start-Sleep -Milliseconds ([Math]::Min(1000, 250 + $attempt * 100))
            continue
        }
        # Poll delayed clipboard rendering without rerunning CopyPicture immediately.
        for ($poll = 0; $poll -lt 10; $poll++) {
            Start-Sleep -Milliseconds 100
            $beforeRead = Get-VisualClipboardSnapshot
            if ($beforeRead.owner_pid -ne 0 -and $beforeRead.owner_pid -ne $expectedProcess) {
                $lastError = 'Clipboard owner is not this Excel instance.'
                Write-VisualClipboardState $label $attempt 'foreign_owner' $beforeRead $lastError
                break
            }
            if ($beforeRead.owner_pid -eq 0) {
                $lastError = 'Clipboard owner could not be verified.'
                if ($poll -eq 9) { Write-VisualClipboardState $label $attempt 'unknown_owner' $beforeRead $lastError }
                continue
            }
            $image = $null
            try {
                $image = [System.Windows.Forms.Clipboard]::GetImage()
                $afterRead = Get-VisualClipboardSnapshot
                if ($null -ne $image -and (Test-VisualClipboardImageIdentity $beforeCopy $beforeRead $afterRead $expectedProcess)) {
                    Write-VisualClipboardState $label $attempt 'verified_image' $afterRead
                    Write-ProgressLine "visual step phase=excel.clipboard_capture duration_ms=$($total.ElapsedMilliseconds) label='$label' retries=$attempt"
                    return ,$image
                }
                $stage = if ($null -eq $image) { 'empty_image' } else { 'sequence_changed_during_read' }
                Write-VisualClipboardState $label $attempt $stage $afterRead
            } catch {
                $lastError = $_.Exception.Message
                Write-VisualClipboardState $label $attempt 'clipboard_read_exception' (Get-VisualClipboardSnapshot) $lastError
            }
            if ($null -ne $image) { $image.Dispose() }
        }
        Write-ProgressLine "clipboard retry label='$label' attempt=$attempt reason='$lastError'"
        Start-Sleep -Milliseconds 150
    }
    throw "Clipboard image verification failed for '$label' after 20 attempts: $lastError"
}
