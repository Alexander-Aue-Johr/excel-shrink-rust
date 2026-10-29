function Get-ExcelSlotLimit([string]$Setting, [int]$Default, [int]$Maximum) {
    $controlPath = [IO.Path]::Combine($env:EXCEL_SHRINK_EXCEL_SLOT_ROOT, 'control.json')
    if (-not [IO.File]::Exists($controlPath)) { return $Default }
    try {
        $control = [IO.File]::ReadAllText($controlPath) | ConvertFrom-Json
        $property = $control.PSObject.Properties[$Setting]
        if ($null -ne $property -and $property.Value -ge 0 -and $property.Value -le $Maximum) {
            return [Math]::Max(1, [int]$property.Value)
        }
    } catch [IO.IOException] {
        # A concurrent atomic control update can briefly hide the file.
    }
    return $Default
}

function Enter-ExcelSlot([string]$Prefix, [string]$Setting, [int]$Default, [int]$Maximum, [int]$Count = 1) {
    if (-not $env:EXCEL_SHRINK_EXCEL_SLOT_ROOT) { return }
    if ($Count -lt 1 -or $Count -gt $Maximum) { throw "Invalid $Prefix slot count: $Count" }

    $root = $env:EXCEL_SHRINK_EXCEL_SLOT_ROOT
    [void][IO.Directory]::CreateDirectory($root)
    while ($true) {
        # The admission lock makes a group's multi-slot claim atomic across PowerShell workers.
        $admission = $null
        try {
            $admission = [IO.File]::Open(
                ([IO.Path]::Combine($root, "$Prefix-admission.lock")),
                [IO.FileMode]::OpenOrCreate,
                [IO.FileAccess]::ReadWrite,
                [IO.FileShare]::None
            )
        } catch [IO.IOException] {
            Start-Sleep -Milliseconds 100
            continue
        }
        try {
            $limit = Get-ExcelSlotLimit $Setting $Default $Maximum
            $occupied = 0
            $available = New-Object 'System.Collections.Generic.List[string]'
            for ($index = 0; $index -lt $Maximum; $index++) {
                $path = [IO.Path]::Combine($root, ('{0}-{1:D2}.lock' -f $Prefix, $index))
                if (-not [IO.File]::Exists($path)) {
                    if ($index -lt $limit -and $available.Count -lt $Count) { $available.Add($path) }
                    continue
                }
                try {
                    $probe = [IO.File]::Open($path, [IO.FileMode]::Open,
                        [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
                    $probe.Dispose()
                    if ($index -lt $limit -and $available.Count -lt $Count) { $available.Add($path) }
                } catch [IO.IOException] {
                    $occupied++
                }
            }
            if ($Count -gt $limit) { throw "$Prefix needs $Count slots, but the configured limit is $limit" }
            if ($occupied + $Count -le $limit -and $available.Count -eq $Count) {
                $claimed = New-Object 'System.Collections.Generic.List[IO.FileStream]'
                try {
                    foreach ($path in $available) {
                        $claimed.Add([IO.File]::Open($path, [IO.FileMode]::OpenOrCreate,
                            [IO.FileAccess]::ReadWrite, [IO.FileShare]::None))
                    }
                    return $claimed
                } catch {
                    foreach ($slot in $claimed) { $slot.Dispose() }
                    throw
                }
            }
        } finally {
            $admission.Dispose()
        }
        Start-Sleep -Milliseconds 200
    }
}

function Enter-ExcelProcessSlot {
    return Enter-ExcelSlot 'excel' 'excel_process_limit' 8 32 1
}

function Enter-ExcelProcessSlots([int]$Count) {
    return Enter-ExcelSlot 'excel' 'excel_process_limit' 8 32 $Count
}

function Enter-CaptureGroupSlot {
    $default = 1
    $configured = 0
    if ([int]::TryParse($env:EXCEL_SHRINK_CAPTURE_GROUP_LIMIT, [ref]$configured)) {
        $default = [Math]::Max(1, [Math]::Min(128, $configured))
    }
    return Enter-ExcelSlot 'capture' 'target_groups' $default 128 1
}

function Exit-ExcelProcessSlot($Slot) {
    if ($null -ne $Slot) { $Slot.Dispose() }
}
