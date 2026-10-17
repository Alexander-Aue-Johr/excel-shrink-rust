# Verify local staged web assets through the actual task-pane origin before Excel starts.
# No native compilation, certificate bypass, Office calls or external CDN checks.
function Get-VVRequiredWebAssets {
    param([string]$Experiment)
    $mode = switch ($Experiment) {
        'range-image' { 'modes/range-image.js' }
        'shape-bounds' { 'modes/shape-bounds.js' }
        'range-image-height-limit' { 'modes/range-image-height-limit.js' }
        'range-image-calibration' { 'modes/range-image-calibration.js' }
        default { throw ('Unsupported experiment for asset preflight: ' + $Experiment) }
    }
    $items = @(
        @{ route='/taskpane.html'; file='taskpane.html'; mime='text/html' },
        @{ route='/taskpane.css'; file='taskpane.css'; mime='text/css' },
        @{ route='/commands.html'; file='commands.html'; mime='text/html' },
        @{ route='/bootstrap.js'; file='bootstrap.js'; mime='application/javascript' },
        @{ route='/shell.js'; file='shell.js'; mime='application/javascript' }
    )
    if ($Experiment -eq 'range-image') {
        $items += @(
            @{ route='/capture-geometry.js'; file='capture-geometry.js'; mime='application/javascript' },
            @{ route='/safe-capture.js'; file='safe-capture.js'; mime='application/javascript' },
            @{ route='/context-validation.js'; file='context-validation.js'; mime='application/javascript' },
            @{ route='/capture-policy.js'; file='capture-policy.js'; mime='application/javascript' },
            @{ route='/selective-overflow.js'; file='selective-overflow.js'; mime='application/javascript' },
            @{ route='/capture-options.json'; file='capture-options.json'; mime='application/json' }
        )
    }
    if ($Experiment -in @('range-image-height-limit','range-image-calibration')) {
        $items += @{ route='/render-diagnostics.js'; file='render-diagnostics.js'; mime='application/javascript' }
    }
    if ($Experiment -eq 'range-image-calibration') {
        $items += @{ route='/row-height-diagnostics.js'; file='row-height-diagnostics.js'; mime='application/javascript' }
    }
    $items += @{ route='/mode.js'; file=$mode; mime='application/javascript' }
    return $items
}

function Get-VVAssetHash {
    param([byte[]]$Bytes)
    $hash = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hash.ComputeHash($Bytes)).Replace('-','').ToLowerInvariant() }
    finally { $hash.Dispose() }
}

function Read-VVAssetResponse {
    param([string]$Uri, [int]$MaxBytes, [int]$TimeoutMs)
    # Redirects and proxies are disabled: the bytes must come from this loopback host.
    # Normal OS TLS validation remains active for the .NET HTTPS endpoint.
    $request = [Net.HttpWebRequest]::Create($Uri)
    $request.Method = 'GET'
    $request.AllowAutoRedirect = $false
    $request.Proxy = $null
    $request.Timeout = $TimeoutMs
    $request.ReadWriteTimeout = $TimeoutMs
    $request.Headers['Cache-Control'] = 'no-cache'
    $request.KeepAlive = $false
    $response = $null; $stream = $null; $output = $null
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        $response = $request.GetResponse()
        if ([int]$response.StatusCode -ne 200) {
            throw ('Expected HTTP 200, received ' + [int]$response.StatusCode)
        }
        $stream = $response.GetResponseStream()
        $output = [IO.MemoryStream]::new()
        $buffer = [byte[]]::new(16384)
        while ($true) {
            if ($watch.ElapsedMilliseconds -ge $TimeoutMs) { throw 'Asset body read deadline exceeded.' }
            $n = $stream.Read($buffer,0,$buffer.Length)
            if ($n -eq 0) { break }
            if ($output.Length + $n -gt $MaxBytes) { throw 'Asset response exceeds its byte budget.' }
            $output.Write($buffer,0,$n)
        }
        return [pscustomobject]@{
            status = [int]$response.StatusCode
            content_type = [string]$response.ContentType
            body = [byte[]]$output.ToArray()
        }
    } finally {
        if ($null -ne $stream) { $stream.Dispose() }
        if ($null -ne $output) { $output.Dispose() }
        if ($null -ne $response) { $response.Close() }
        $request.Abort()
    }
}

function Assert-VVHostIdentity($Config,[string]$LaunchId,[string]$Experiment,[string]$HostKind) {
    if ([string]$Config.host_run_id -cne $LaunchId -or [string]$Config.experiment -cne $Experiment -or [string]$Config.host_kind -cne $HostKind) {
        throw 'Host identity does not match this launch, experiment and implementation.'
    }
    if ($Experiment -eq 'range-image' -and [string]$Config.capture_run_id -cne $LaunchId) {
        throw 'Capture identity does not match this launch.'
    }
    if ($Experiment -in @('range-image-height-limit','range-image-calibration') -and
        [string]$Config.diagnostic_run_id -cnotmatch '^[a-f0-9]{32}$') {
        throw 'Diagnostic session identity is missing or malformed.'
    }
}

function Assert-VVWebAssets {
    param(
        [Parameter(Mandatory=$true)][string]$StaticRoot,
        [Parameter(Mandatory=$true)][string]$Experiment,
        [Parameter(Mandatory=$true)][string]$HostKind,
        [Parameter(Mandatory=$true)][string]$CaptureRunId,
        [Parameter(Mandatory=$true)][string]$ReportRoot
    )
    $origin = if ($HostKind -eq 'rust') { 'http://localhost:3000' } elseif ($HostKind -eq 'dotnet') { 'https://localhost:3000' } else { throw 'Unknown asset-preflight host.' }
    $record = [ordered]@{
        schema='vv-local-assets-v1'; scope='local-web-assets'; status='running'; code=$null
        experiment=$Experiment; host_kind=$HostKind; origin=$origin; capture_run_id=$CaptureRunId
        started_utc=[DateTime]::UtcNow.ToString('o'); completed_utc=$null; phase='local-files'; current=$null
        assets=@(); error=$null; duration_ms=$null
        note='Local HTTP reachability and staged-byte equality only; no Excel or external Office.js CDN validation.'
    }
    $reportPath = Join-Path $ReportRoot 'asset-preflight.json'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        $entries = @(Get-VVRequiredWebAssets $Experiment)
        # Check the whole required set locally before any network request.
        foreach ($entry in $entries) {
            $record.current = $entry.route
            $path = Join-Path $StaticRoot $entry.file
            if (-not [IO.File]::Exists($path)) { throw ('Staged asset missing: ' + $entry.file) }
            $bytes = [IO.File]::ReadAllBytes($path)
            if ($bytes.Length -eq 0 -or $bytes.Length -gt 4194304) {
                throw ('Staged asset is empty or exceeds 4 MiB: ' + $entry.file)
            }
            $record.assets += [pscustomobject]@{
                route=$entry.route; file=$entry.file; expected_type=$entry.mime
                expected_bytes=$bytes.Length; expected_sha256=(Get-VVAssetHash $bytes)
                status='pending'; http_status=$null; content_type=$null; received_bytes=$null; received_sha256=$null
            }
        }
        Write-VVJson $reportPath $record
        $record.phase = 'host-identity'; $record.current = '/config'
        $reply = Read-VVAssetResponse -Uri ($origin+'/config?preflight='+$CaptureRunId) -MaxBytes 1048576 -TimeoutMs 8000
        $cfg = ([Text.UTF8Encoding]::new($false,$true)).GetString($reply.body) | ConvertFrom-Json
        Assert-VVHostIdentity -Config $cfg -LaunchId $CaptureRunId -Experiment $Experiment -HostKind $HostKind
        $record.host_identity=@{host_run_id=$cfg.host_run_id;capture_run_id=$cfg.capture_run_id;diagnostic_run_id=$cfg.diagnostic_run_id}
        $record.phase = 'http-assets'
        foreach ($asset in $record.assets) {
            if ($watch.Elapsed.TotalSeconds -gt 40) { throw 'Local asset-preflight total deadline exceeded.' }
            $record.current = $asset.route
            try {
                $reply = Read-VVAssetResponse -Uri ($origin+$asset.route+'?preflight='+$CaptureRunId) -MaxBytes ($asset.expected_bytes+1) -TimeoutMs 8000
                $asset.http_status = $reply.status
                $asset.content_type = $reply.content_type
                $asset.received_bytes = $reply.body.Length
                $asset.received_sha256 = Get-VVAssetHash $reply.body
                $mime = ([string]$reply.content_type).Split(';')[0].Trim().ToLowerInvariant()
                if ($mime -ne $asset.expected_type) { throw ('Unexpected Content-Type: '+$reply.content_type) }
                if ($asset.received_bytes -ne $asset.expected_bytes -or $asset.received_sha256 -cne $asset.expected_sha256) {
                    throw 'HTTP payload does not match the staged file bytes.'
                }
                $asset.status = 'complete'
            } catch {
                $asset.status = 'failed'
                $cause = $_.Exception
                while ($null -ne $cause) {
                    $responseProperty = $cause.PSObject.Properties['Response']
                    if ($null -ne $responseProperty -and $null -ne $responseProperty.Value) {
                        try { $asset.http_status = [int]$responseProperty.Value.StatusCode } catch {}
                        break
                    }
                    $cause = $cause.InnerException
                }
                throw
            }
        }
        $record.status = 'complete'; $record.phase = 'complete'; $record.current = $null
        Write-ExperimentLog ('Local web assets verified: '+$record.assets.Count+' files; staged SHA-256 matches HTTP; Excel launch not started.')
        Write-VVEvent 'web_assets_verified' @{host=$HostKind;files=$record.assets.Count;report='asset-preflight.json'}
    } catch {
        $record.status = 'failed'; $record.code = 'VV_ASSET_PREFLIGHT_FAILED'; $record.error = $_.Exception.Message
        Write-VVEvent 'web_assets_failed' @{code=$record.code;host=$HostKind;phase=$record.phase;route=$record.current;message=$record.error;report='asset-preflight.json'}
        throw ('VV_ASSET_PREFLIGHT_FAILED: '+$record.phase+' '+$record.current+' - '+$record.error+'. No workbook was launched by the runner; see '+$reportPath)
    } finally {
        $record.duration_ms = $watch.Elapsed.TotalMilliseconds
        $record.completed_utc = [DateTime]::UtcNow.ToString('o')
        Write-VVJson $reportPath $record
    }
}
