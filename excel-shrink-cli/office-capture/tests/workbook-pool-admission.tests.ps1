$ErrorActionPreference = 'Stop'
$module = Import-Module (Join-Path $PSScriptRoot '../WorkbookPool.psm1') -Force -PassThru
& $module {
    $script:requests = @()
    function script:Invoke-WorkbookPoolRequest($Connection, $Path, $Body, $Lease, $TimeoutSeconds) {
        $script:requests += $Body.comparison_id
        if ($script:requests.Count -eq 1) {
            throw [Net.WebException]::new('Fixture admission timeout', $null, [Net.WebExceptionStatus]::Timeout, $null)
        }
        return @{comparison_id=$Body.comparison_id;slot=0;capture_port=12345;manifest_id='fixture';lease_token='fixture'}
    }
    $lease = Get-WorkbookPoolLease @{} 5
    if ($script:requests.Count -ne 2 -or $script:requests[0] -ne $script:requests[1]) {
        throw 'Timeout retry changed the admission identity.'
    }
    function script:Invoke-WorkbookPoolRequest {
        throw [Net.WebException]::new('Fixture connect failure', $null, [Net.WebExceptionStatus]::ConnectFailure, $null)
    }
    try { Get-WorkbookPoolLease @{} 5 | Out-Null }
    catch [Net.WebException] {
        if ($_.Exception.Status -ne [Net.WebExceptionStatus]::ConnectFailure) { throw }
        return
    }
    throw 'Connect failure was silently retried.'
}
Write-Output 'PASS bounded admission timeout retry keeps its identity and connection failure remains explicit'
