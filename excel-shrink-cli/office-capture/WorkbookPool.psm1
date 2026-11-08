# Authenticated, bounded transport to the run-owned warm workbook pool.
$ErrorActionPreference = 'Stop'

function Read-WorkbookPoolConnection {
    param([Parameter(Mandatory=$true)][string]$Path)
    $connection = [IO.File]::ReadAllText($Path) | ConvertFrom-Json
    $address = [Uri]$connection.url
    if (-not $address.IsAbsoluteUri -or $address.Scheme -ne 'http' -or
        $address.Host -notin @('localhost', '127.0.0.1') -or $address.Port -le 0 -or
        $address.AbsolutePath -ne '/' -or $address.Query -or $address.Fragment -or $address.UserInfo) {
        throw 'Workbook pool requires a plain loopback HTTP origin.'
    }
    $identity = [Guid]::Empty
    if (-not [Guid]::TryParse([string]$connection.token, [ref]$identity)) {
        throw 'Invalid workbook pool credential.'
    }
    return $connection
}

function Invoke-WorkbookPoolRequest {
    param($Connection, [string]$Path, $Body, $Lease, [int]$TimeoutSeconds = 120)
    $headers = @{'X-Pool-Token'=[string]$Connection.token}
    if ($Lease) { $headers['X-Lease-Token'] = [string]$Lease.lease_token }
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Body | ConvertTo-Json -Depth 12 -Compress))
    Invoke-RestMethod -Method Post ($Connection.url.TrimEnd('/') + $Path) -Headers $headers `
        -ContentType 'application/json; charset=utf-8' -Body $bytes -TimeoutSec $TimeoutSeconds
}

function Get-WorkbookPoolLease {
    param($Connection, [int]$TimeoutSeconds)
    $owner = [Diagnostics.Process]::GetCurrentProcess()
    try {
        $request = @{
            pid=$owner.Id
            started_ms=([DateTimeOffset]$owner.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
            comparison_id=[Guid]::NewGuid().ToString()
        }
    } finally { $owner.Dispose() }
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($clock.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        try {
            # A retry retains the same comparison identity, preventing duplicate leases.
            $lease = Invoke-WorkbookPoolRequest $Connection '/acquire' $request $null 10
            if ($lease.comparison_id -ne $request.comparison_id -or
                $lease.slot -lt 0 -or $lease.capture_port -le 0 -or
                -not $lease.manifest_id -or -not $lease.lease_token) {
                throw 'Workbook pool returned an invalid lease identity.'
            }
            return $lease
        } catch [Net.WebException] {
            if ($null -eq $_.Exception.Response -or [int]$_.Exception.Response.StatusCode -ne 409) { throw }
            Start-Sleep -Milliseconds 100
        }
    }
    throw 'Workbook pool admission timed out.'
}

function Invoke-WorkbookPoolCommand {
    param($Connection, $Lease, [string]$Kind, [string]$Path, [int]$TimeoutSeconds = 120, $Details = @{})
    $id = [Guid]::NewGuid().ToString()
    $request = @{id=$id;kind=$Kind;path=$Path}
    foreach ($key in $Details.Keys) {
        if ($request.ContainsKey($key)) { throw 'Pool command details cannot replace transport identity.' }
        $request[$key] = $Details[$key]
    }
    $reply = Invoke-WorkbookPoolRequest $Connection ('/command/' + $Lease.slot) `
        $request $Lease $TimeoutSeconds
    if (-not $reply.ok -or $reply.id -ne $id -or $reply.pid -ne $Lease.process.pid) {
        throw ('Workbook pool command failed: ' + [string]$reply.error)
    }
    return $reply
}

function Invoke-ResidentPoolCommand {
    param($Entry, [string]$Kind, [string]$Path, $Details = @{})
    $connection = Read-WorkbookPoolConnection $env:EXCEL_SHRINK_WORKBOOK_POOL_CONNECTION
    $lease = [IO.File]::ReadAllText((Join-Path $Entry.session 'pool-lease.json')) | ConvertFrom-Json
    Invoke-WorkbookPoolCommand $connection $lease $Kind $Path 120 $Details
}

function Remove-WorkbookPoolLease {
    param($Connection, $Lease)
    $reply = Invoke-WorkbookPoolRequest $Connection ('/release/' + $Lease.slot) @{} $Lease 10
    if ($reply.ok -ne $true) { throw 'Workbook pool did not confirm release.' }
}

Export-ModuleMember -Function Read-WorkbookPoolConnection, Get-WorkbookPoolLease, Invoke-WorkbookPoolCommand, Remove-WorkbookPoolLease, Invoke-ResidentPoolCommand
