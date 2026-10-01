param(
    [Parameter(Mandatory=$true)]
    [string]$FileListPath,

    [Parameter(Mandatory=$true)]
    [string]$OutputPath,

    [Parameter(Mandatory=$true)]
    [string]$ImageOutputDir,

    [ValidateSet('shape-bounds', 'range-image')]
    [string]$Experiment = 'range-image',

    [ValidateSet('A', 'B', 'C', 'D', 'E')]
    [string]$Strategy = 'A',

    [ValidateSet('dotnet', 'rust')]
    [string]$HostKind = 'dotnet',

    [ValidateSet('excel-x', 'rust-com')]
    [string]$ExcelLaunchMode = 'excel-x',

    [ValidateRange(1, 64)]
    [int]$BatchSize = 8,

    [ValidateRange(64, 8192)]
    [int]$OverflowProbeWidthPx = 1920,

    [ValidateRange(64, 8192)]
    [int]$OverflowProbeHeightPx = 1080,

    [ValidateRange(10, 600)]
    [int]$StartupTimeoutSeconds = 180,

    [ValidateRange(10, 600)]
    [int]$InactivityTimeoutSeconds = 120,

    [ValidateRange(60, 3600)]
    [int]$AbsoluteWorkbookTimeoutSeconds = 900,

    [ValidateRange(1, 10)]
    [int]$AttemptsPerWorkbook = 5,

    [ValidateRange(0, 60)]
    [int]$RetryDelaySeconds = 10,

    [switch]$TemporaryDeveloperRegistration
)

$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'powershell\common.ps1')

$FileListPath = [IO.Path]::GetFullPath($FileListPath)
$OutputPath = [IO.Path]::GetFullPath($OutputPath)
$ImageOutputDir = [IO.Path]::GetFullPath($ImageOutputDir)

if (-not (Test-Path -LiteralPath $FileListPath)) {
    throw "File list does not exist: $FileListPath"
}

$paths = @(
    [IO.File]::ReadAllLines($FileListPath, $script:StrictUtf8) |
    ForEach-Object { $_.Trim() } |
    Where-Object { $_.Length -gt 0 }
)

if ($paths.Count -eq 0) {
    throw "File list is empty: $FileListPath"
}

if ($HostKind -eq 'dotnet' -and $ExcelLaunchMode -eq 'rust-com') {
    throw 'ExcelLaunchMode rust-com is only valid with HostKind rust.'
}

$RepoRoot = [IO.Path]::GetFullPath(
    (Join-Path $PSScriptRoot '..\..\..\..\..')
)
$WebRoot = Join-Path $PSScriptRoot 'web'
$ManifestSource = Join-Path $WebRoot 'manifest.xml'
$LocalRoot = Join-Path `
    $env:LOCALAPPDATA `
    'excel-shrink-rust\visual-validation-experiments'
$BuildCache = Join-Path $LocalRoot 'build-cache'
$RegistrationRoot = Join-Path $LocalRoot 'office-addin'
$RuntimeRoot = Join-Path `
    $env:TEMP `
    ('excel-shrink-visual-validation-' + [Guid]::NewGuid().ToString('N'))
$StaticStage = Join-Path $RuntimeRoot 'web'
$DocumentRoot = Join-Path $RuntimeRoot 'documents'
$HostOutput = Join-Path $RuntimeRoot 'result.json'
$HostImageRoot = Join-Path $RuntimeRoot 'images'
$HostStdout = Join-Path $RuntimeRoot 'host.stdout.txt'
$HostStderr = Join-Path $RuntimeRoot 'host.stderr.txt'
$HostFileList = Join-Path $RuntimeRoot 'files.txt'

New-Item -ItemType Directory -Path $BuildCache -Force | Out-Null
New-Item -ItemType Directory -Path $RegistrationRoot -Force | Out-Null
New-Item -ItemType Directory -Path $StaticStage -Force | Out-Null
New-Item -ItemType Directory -Path $DocumentRoot -Force | Out-Null
New-Item -ItemType Directory -Path $HostImageRoot -Force | Out-Null

Write-ExperimentLog "Experiment: $Experiment"
Write-ExperimentLog "Strategy: $Strategy"
Write-ExperimentLog "Office.js host: $HostKind"
Write-ExperimentLog "Excel launch mode: $ExcelLaunchMode"
Write-ExperimentLog "Batch size: $BatchSize"

Write-ExperimentLog 'Staging Office add-in web files...'
Copy-Item `
    -Path (Join-Path $WebRoot '*') `
    -Destination $StaticStage `
    -Recurse `
    -Force
Copy-Item -LiteralPath $FileListPath -Destination $HostFileList -Force

# Source Context patch: restore repo-local .NET 8 fallback.
function Test-DotNet8Sdk([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $false
    }

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $false
    }

    try {
        $sdks = @(& $Path --list-sdks 2>$null)

        if ($LASTEXITCODE -ne 0) {
            return $false
        }

        return @($sdks | Where-Object { $_ -match '^\s*8\.' }).Count -gt 0
    }
    catch {
        return $false
    }
}

function Get-StableStringSha256([string]$Value) {
    $sha256 = [Security.Cryptography.SHA256]::Create()

    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
        $hash = $sha256.ComputeHash($bytes)

        return (
            [BitConverter]::ToString($hash)
        ).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

function Get-RepoDotNetPaths {
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw @"
LOCALAPPDATA is not available.

The visual-validation ASP.NET host needs a writable per-user directory for its
repo-local Windows .NET 8 SDK fallback. No administrator rights are required.
"@
    }

    $toolchainId = (
        Get-StableStringSha256 $RepoRoot
    ).Substring(0, 16)

    $root = Join-Path `
        $env:LOCALAPPDATA `
        "excel-shrink-rust\repo-toolchains\$toolchainId"

    $dotnetRoot = Join-Path $root 'dotnet-win-x64'

    return [pscustomobject]@{
        ToolchainRoot = $root
        DotNetRoot = $dotnetRoot
        DotNetExe = (Join-Path $dotnetRoot 'dotnet.exe')
        InstallScript = (Join-Path $root 'dotnet-install.ps1')
    }
}

function Find-DotNet8 {
    $candidates = New-Object 'System.Collections.Generic.List[string]'

    $global = Get-Command dotnet.exe -ErrorAction SilentlyContinue
    if ($null -ne $global) {
        $candidates.Add([string]$global.Source)
    }

    if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        $repoToolchains = Join-Path `
            $env:LOCALAPPDATA `
            'excel-shrink-rust\repo-toolchains'

        # Also accept the short-lived un-hashed layout used by one experiment
        # revision, then scan the older repo-hashed caches.
        $candidates.Add(
            (Join-Path $repoToolchains 'dotnet-win-x64\dotnet.exe')
        )

        if (Test-Path -LiteralPath $repoToolchains -PathType Container) {
            foreach ($candidate in @(
                Get-ChildItem `
                    -LiteralPath $repoToolchains `
                    -Recurse `
                    -File `
                    -Filter 'dotnet.exe' `
                    -ErrorAction SilentlyContinue |
                Where-Object {
                    $_.FullName -like '*\dotnet-win-x64\dotnet.exe'
                } |
                Sort-Object FullName
            )) {
                $candidates.Add($candidate.FullName)
            }
        }
    }

    $seen = @{}

    foreach ($candidate in $candidates) {
        if ([string]::IsNullOrWhiteSpace($candidate)) {
            continue
        }

        $key = $candidate.ToLowerInvariant()
        if ($seen.ContainsKey($key)) {
            continue
        }
        $seen[$key] = $true

        if (Test-DotNet8Sdk $candidate) {
            return $candidate
        }
    }

    return $null
}

function Install-RepoLocalDotNet8Sdk {
    $paths = Get-RepoDotNetPaths

    New-Item `
        -ItemType Directory `
        -Path $paths.ToolchainRoot `
        -Force | Out-Null

    if (
        (Test-Path -LiteralPath $paths.DotNetRoot) -and
        -not (Test-DotNet8Sdk $paths.DotNetExe)
    ) {
        Write-ExperimentLog (
            'Removing incomplete repo-local .NET SDK cache: ' +
            $paths.DotNetRoot
        ) 'WARN'

        Remove-Item `
            -LiteralPath $paths.DotNetRoot `
            -Recurse `
            -Force
    }

    New-Item `
        -ItemType Directory `
        -Path $paths.DotNetRoot `
        -Force | Out-Null

    if (-not (Test-Path -LiteralPath $paths.InstallScript -PathType Leaf)) {
        Write-ExperimentLog (
            'Downloading Microsoft dotnet-install.ps1 to ' +
            $paths.InstallScript
        )

        $previousSecurityProtocol = [Net.ServicePointManager]::SecurityProtocol

        try {
            [Net.ServicePointManager]::SecurityProtocol = `
                [Net.SecurityProtocolType]::Tls12

            Invoke-WebRequest `
                -UseBasicParsing `
                -Uri 'https://dot.net/v1/dotnet-install.ps1' `
                -OutFile $paths.InstallScript
        }
        finally {
            [Net.ServicePointManager]::SecurityProtocol = `
                $previousSecurityProtocol
        }
    }
    else {
        Write-ExperimentLog (
            'Reusing cached Microsoft dotnet-install.ps1: ' +
            $paths.InstallScript
        )
    }

    Unblock-File `
        -LiteralPath $paths.InstallScript `
        -ErrorAction SilentlyContinue

    Write-ExperimentLog (
        'Installing repo-local Windows .NET 8 SDK without admin rights: ' +
        $paths.DotNetRoot
    )

    $windowsPowerShell = Join-Path $PSHOME 'powershell.exe'

    & $windowsPowerShell `
        -NoLogo `
        -NoProfile `
        -ExecutionPolicy Bypass `
        -File $paths.InstallScript `
        -Channel '8.0' `
        -Quality 'GA' `
        -Architecture 'x64' `
        -InstallDir $paths.DotNetRoot `
        -NoPath `
        -Verbose

    $installExitCode = $LASTEXITCODE

    if ($installExitCode -ne 0) {
        throw (
            'Repo-local .NET 8 SDK installation failed with exit code ' +
            "$installExitCode. Install directory: $($paths.DotNetRoot)"
        )
    }

    if (-not (Test-DotNet8Sdk $paths.DotNetExe)) {
        throw @"
The repo-local .NET installer completed, but no usable .NET 8 SDK was found.

Expected executable:
  $($paths.DotNetExe)
"@
    }

    return $paths.DotNetExe
}

function Get-DotNet8 {
    $dotnet = Find-DotNet8

    if ($null -ne $dotnet) {
        Write-ExperimentLog "Using Windows .NET 8 SDK: $dotnet"
        return $dotnet
    }

    Write-ExperimentLog (
        'No usable Windows .NET 8 SDK was found; using the per-user ' +
        'repo-local fallback.'
    )

    return Install-RepoLocalDotNet8Sdk
}

function Ensure-DotNetHost {
    $dotnet = Get-DotNet8

    # The direct dotnet.exe invocation already resolves its SDK relative to itself,
    # but DOTNET_ROOT is also inherited by the cached Kestrel host process.
    $env:DOTNET_ROOT = Split-Path -Parent $dotnet
    $env:DOTNET_CLI_UI_LANGUAGE = 'en-us'
    $env:DOTNET_NOLOGO = '1'
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
    $env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = '1'

    $source = Join-Path $PSScriptRoot 'hosts\dotnet'
    $cache = Join-Path $BuildCache 'dotnet-host'
    $sourceCache = Join-Path $cache 'src'
    $fingerprintPath = Join-Path $cache 'source.sha256'
    $fingerprint = Get-SourceFingerprint @($source)
    $current = if (Test-Path -LiteralPath $fingerprintPath) {
        [IO.File]::ReadAllText($fingerprintPath).Trim()
    } else { '' }

    $dll = Join-Path `
        $sourceCache `
        'bin\Release\net8.0\VisualValidationExperimentHost.dll'

    if (
        $current -ne $fingerprint -or
        -not (Test-Path -LiteralPath $dll)
    ) {
        Write-ExperimentLog 'ASP.NET host source changed; building cached Release host...'
        Remove-Item -LiteralPath $sourceCache -Recurse -Force -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Path $sourceCache -Force | Out-Null
        Copy-Item -Path (Join-Path $source '*') -Destination $sourceCache -Recurse
        & $dotnet build `
            (Join-Path $sourceCache 'VisualValidationExperimentHost.csproj') `
            -c Release `
            --nologo
        if ($LASTEXITCODE -ne 0) {
            throw "ASP.NET experiment host build failed with exit code $LASTEXITCODE"
        }
        Write-Utf8NoBom $fingerprintPath ($fingerprint + "`n")
    }
    else {
        Write-ExperimentLog 'Reusing cached ASP.NET host.'
    }

    Write-ExperimentLog 'Checking trusted ASP.NET Core localhost certificate...'
    & $dotnet dev-certs https --check --trust --quiet *> $null
    if ($LASTEXITCODE -ne 0) {
        Write-ExperimentLog 'Creating/trusting ASP.NET Core localhost certificate...'
        & $dotnet dev-certs https --trust
        if ($LASTEXITCODE -ne 0) {
            throw 'Could not create/trust the ASP.NET Core HTTPS development certificate.'
        }
    }

    return [pscustomobject]@{
        Exe = $dotnet
        Args = @($dll)
    }
}

function Ensure-RustHost {
    $wsl = Convert-WslUncToLinuxPath $PSScriptRoot
    if ($null -eq $wsl) {
        throw (
            'The Rust experiment host build currently expects this repository to live ' +
            'inside WSL and be accessed through \\wsl.localhost.'
        )
    }

    $buildLinux = $wsl.LinuxPath + '/build-rust-host.sh'
    Write-ExperimentLog 'Checking cached Rust/MinGW Office host...'

    # Keep the Windows -> WSL bridge deliberately simple. The build script
    # itself owns Rust PATH setup, so this also works when Windows PowerShell
    # was launched from a VS Code task running inside WSL.
    & wsl.exe `
        -d $wsl.Distribution `
        -- bash `
        $buildLinux

    if ($LASTEXITCODE -ne 0) {
        throw "Rust Office host build/check failed with exit code $LASTEXITCODE"
    }

    $repoWsl = Convert-WslUncToLinuxPath $RepoRoot
    $builtLinux =
        $repoWsl.LinuxPath +
        '/.tmp/visual-validation/experiment-rust-office-host-target/' +
        'x86_64-pc-windows-gnu/release/visual-validation-office-host.exe'

    $builtWindows = (
        & wsl.exe `
            -d $repoWsl.Distribution `
            -- wslpath -w $builtLinux
    ).Trim()

    if (-not (Test-Path -LiteralPath $builtWindows)) {
        throw "Rust host build did not produce: $builtWindows"
    }

    $cache = Join-Path $BuildCache 'rust-host'
    New-Item -ItemType Directory -Path $cache -Force | Out-Null
    $exe = Join-Path $cache 'visual-validation-office-host.exe'
    Copy-Item -LiteralPath $builtWindows -Destination $exe -Force

    $certRoot = Join-Path $LocalRoot 'rust-host-certificate'
    New-Item -ItemType Directory -Path $certRoot -Force | Out-Null

    Write-ExperimentLog 'Ensuring Rust-host localhost certificate...'
    & $exe ensure-cert $certRoot
    if ($LASTEXITCODE -ne 0) {
        throw 'Rust host failed to create/reuse its localhost certificate.'
    }

    $certDer = Join-Path $certRoot 'localhost-cert.der'
    $certPem = Join-Path $certRoot 'localhost-cert.pem'
    $keyPem = Join-Path $certRoot 'localhost-key.pem'

    $certificate = New-Object `
        Security.Cryptography.X509Certificates.X509Certificate2 `
        $certDer
    $trustedPath = 'Cert:\CurrentUser\Root\' + $certificate.Thumbprint

    if (-not (Test-Path -LiteralPath $trustedPath)) {
        Write-ExperimentLog 'Trusting Rust-host localhost certificate for the current user...'
        Import-Certificate `
            -FilePath $certDer `
            -CertStoreLocation 'Cert:\CurrentUser\Root' | Out-Null
    }
    else {
        Write-ExperimentLog 'Rust-host localhost certificate is already trusted.'
    }

    return [pscustomobject]@{
        Exe = $exe
        Args = @('serve', $certPem, $keyPem)
    }
}

$hostCommand = if ($HostKind -eq 'rust') {
    Ensure-RustHost
}
else {
    Ensure-DotNetHost
}

$oldEnvironment = @{}
foreach ($name in @(
    'VV_EXPERIMENT',
    'VV_STRATEGY',
    'VV_HOST_KIND',
    'VV_STATIC_ROOT',
    'VV_FILE_LIST',
    'VV_OUTPUT_PATH',
    'VV_IMAGE_ROOT',
    'VV_BATCH_SIZE',
    'VV_OVERFLOW_WIDTH_PX',
    'VV_OVERFLOW_HEIGHT_PX'
)) {
    $oldEnvironment[$name] = [Environment]::GetEnvironmentVariable($name)
}

$env:VV_EXPERIMENT = $Experiment
$env:VV_STRATEGY = $Strategy
$env:VV_HOST_KIND = $HostKind
$env:VV_STATIC_ROOT = $StaticStage
$env:VV_FILE_LIST = $HostFileList
$env:VV_OUTPUT_PATH = $HostOutput
$env:VV_IMAGE_ROOT = $HostImageRoot
$env:VV_BATCH_SIZE = [string]$BatchSize
$env:VV_OVERFLOW_WIDTH_PX = [string]$OverflowProbeWidthPx
$env:VV_OVERFLOW_HEIGHT_PX = [string]$OverflowProbeHeightPx

$hostProcess = $null
$registrationState = $null
$excelExecutable = Find-ExcelExecutable

try {
    Write-ExperimentLog "Starting $HostKind Office.js host..."
    $hostProcess = Start-Process `
        -FilePath $hostCommand.Exe `
        -ArgumentList $hostCommand.Args `
        -WorkingDirectory $StaticStage `
        -RedirectStandardOutput $HostStdout `
        -RedirectStandardError $HostStderr `
        -WindowStyle Hidden `
        -PassThru

    $ready = $false
    for ($attempt = 0; $attempt -lt 300; $attempt++) {
        Start-Sleep -Milliseconds 100
        if ($hostProcess.HasExited) { break }
        try {
            $health = Invoke-RestMethod `
                -Uri 'http://127.0.0.1:3001/health' `
                -Method Get
            if ($health.ok) {
                $ready = $true
                break
            }
        }
        catch {
        }
    }

    if (-not $ready) {
        $stderr = if (Test-Path -LiteralPath $HostStderr) {
            [IO.File]::ReadAllText($HostStderr)
        } else { '' }
        throw "Experiment host did not become ready. $stderr"
    }

    Write-ExperimentLog 'Experiment host is healthy.'

    $registrationManifest = Join-Path $RegistrationRoot 'manifest.xml'
    Copy-Item -LiteralPath $ManifestSource -Destination $registrationManifest -Force
    $addInId = Get-ManifestId $registrationManifest
    $addInVersion = Get-ManifestVersion $registrationManifest
    $registrationState = Register-DeveloperAddIn $addInId $registrationManifest
    Write-ExperimentLog "Office developer add-in registered: $addInId"

    if ([string]::IsNullOrWhiteSpace($excelExecutable) -and $ExcelLaunchMode -eq 'excel-x') {
        throw 'Could not locate EXCEL.EXE for excel-x launch mode.'
    }

    $workbookOrdinal = 0

    foreach ($sourcePath in $paths) {
        $workbookOrdinal++
        $workbookName = [IO.Path]::GetFileName($sourcePath)
        $completed = $false

        for (
            $attemptNumber = 1;
            $attemptNumber -le $AttemptsPerWorkbook -and -not $completed;
            $attemptNumber++
        ) {
            if ($attemptNumber -gt 1 -and $RetryDelaySeconds -gt 0) {
                Write-ExperimentLog (
                    "Retry delay $RetryDelaySeconds s before attempt " +
                    "$attemptNumber/$AttemptsPerWorkbook for $workbookName"
                )
                Start-Sleep -Seconds $RetryDelaySeconds
            }

            Refresh-DeveloperAddInRegistration `
                -AddInId $addInId `
                -ManifestPath $registrationManifest

            $attemptDir = Join-Path `
                $DocumentRoot `
                ('workbook-{0:D2}\attempt-{1:D2}' -f $workbookOrdinal, $attemptNumber)
            New-Item -ItemType Directory -Path $attemptDir -Force | Out-Null
            $documentCopy = Join-Path $attemptDir $workbookName

            Write-ExperimentLog "[$workbookOrdinal/$($paths.Count)] Copying $workbookName"
            Copy-Item -LiteralPath $sourcePath -Destination $documentCopy -Force

            Write-ExperimentLog "Injecting Office.AutoShowTaskpaneWithDocument into $workbookName"
            Add-OfficeAddInAutoOpenToWorkbook `
                -WorkbookPath $documentCopy `
                -AddInId $addInId `
                -AddInVersion $addInVersion

            Invoke-RestMethod `
                -Uri 'http://127.0.0.1:3001/reset-diagnostics' `
                -Method Post | Out-Null

            $launchProcess = $null
            $launchStarted = [Diagnostics.Stopwatch]::StartNew()

            Write-ExperimentLog (
                "Launching $workbookName attempt " +
                "$attemptNumber/$AttemptsPerWorkbook"
            )

            if ($HostKind -eq 'rust' -and $ExcelLaunchMode -eq 'rust-com') {
                Invoke-JsonPost `
                    'http://127.0.0.1:3001/control/open' `
                    @{ path = $documentCopy } | Out-Null
                Write-ExperimentLog 'Rust COM host returned from Workbooks.Open.'
            }
            else {
                $launchProcess = Start-ExcelX $excelExecutable $documentCopy
                Write-ExperimentLog ("Excel launcher PID: " + $launchProcess.Id)
            }

            $lastSequence = 0L
            $sawActivity = $false
            $lastActivityLocal = [DateTimeOffset]::UtcNow
            $absoluteStarted = [DateTimeOffset]::UtcNow

            while ($true) {
                Start-Sleep -Milliseconds 250

                try {
                    $encoded = [Uri]::EscapeDataString($workbookName)
                    $status = Invoke-RestMethod `
                        -Uri (
                            'http://127.0.0.1:3001/status?workbook=' +
                            $encoded +
                            '&after=' +
                            $lastSequence
                        ) `
                        -Method Get

                    foreach ($event in @($status.events)) {
                        $sawActivity = $true
                        $lastActivityLocal = [DateTimeOffset]::UtcNow
                        $lastSequence = [Math]::Max(
                            $lastSequence,
                            [long]$event.sequence
                        )

                        $detail = if ($null -ne $event.detail) {
                            ($event.detail | ConvertTo-Json -Depth 8 -Compress)
                        }
                        else {
                            ''
                        }

                        Write-ExperimentLog (
                            "Excel/Add-in: $($event.stage)" +
                            $(if ($detail) { " $detail" } else { '' })
                        )
                    }

                    if (@($status.errors).Count -gt 0) {
                        throw (
                            'Experiment host reported: ' +
                            (@($status.errors) -join ' | ')
                        )
                    }

                    if ($status.done) {
                        $completed = $true
                        Write-ExperimentLog (
                            "$workbookName completed in " +
                            ('{0:0.00}' -f $launchStarted.Elapsed.TotalSeconds) +
                            ' s; closing immediately.'
                        )
                        break
                    }
                }
                catch {
                    if ($_.Exception.Message -like 'Experiment host reported:*') {
                        throw
                    }
                }

                $now = [DateTimeOffset]::UtcNow
                $absoluteSeconds = ($now - $absoluteStarted).TotalSeconds
                $inactiveSeconds = ($now - $lastActivityLocal).TotalSeconds

                if ($absoluteSeconds -gt $AbsoluteWorkbookTimeoutSeconds) {
                    Write-ExperimentLog (
                        "Absolute workbook timeout after " +
                        "$AbsoluteWorkbookTimeoutSeconds s."
                    ) 'WARN'
                    break
                }

                if (
                    -not $sawActivity -and
                    $launchStarted.Elapsed.TotalSeconds -gt $StartupTimeoutSeconds
                ) {
                    Write-ExperimentLog (
                        "No add-in heartbeat within startup timeout " +
                        "$StartupTimeoutSeconds s."
                    ) 'WARN'
                    break
                }

                if (
                    $sawActivity -and
                    $inactiveSeconds -gt $InactivityTimeoutSeconds
                ) {
                    Write-ExperimentLog (
                        "No add-in activity for $InactivityTimeoutSeconds s."
                    ) 'WARN'
                    break
                }
            }

            if ($HostKind -eq 'rust' -and $ExcelLaunchMode -eq 'rust-com') {
                try {
                    Invoke-RestMethod `
                        -Uri 'http://127.0.0.1:3001/control/close' `
                        -Method Post | Out-Null
                }
                catch {
                    Write-ExperimentLog (
                        'Rust COM close reported: ' + $_.Exception.Message
                    ) 'WARN'
                }
            }
            else {
                Stop-BenchmarkExcelProcess $launchProcess
                if ($null -ne $launchProcess) {
                    try { $launchProcess.Dispose() } catch {}
                }
            }
        }

        if (-not $completed) {
            throw (
                "Workbook did not complete after $AttemptsPerWorkbook attempt(s): " +
                $workbookName
            )
        }
    }

    if (-not (Test-Path -LiteralPath $HostOutput)) {
        throw "Host completed without result file: $HostOutput"
    }

    New-Item `
        -ItemType Directory `
        -Path (Split-Path -Parent $OutputPath) `
        -Force | Out-Null
    Copy-Item -LiteralPath $HostOutput -Destination $OutputPath -Force

    Remove-Item `
        -LiteralPath $ImageOutputDir `
        -Recurse `
        -Force `
        -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Path $ImageOutputDir -Force | Out-Null

    foreach ($item in Get-ChildItem -LiteralPath $HostImageRoot -Force) {
        Copy-Item `
            -LiteralPath $item.FullName `
            -Destination $ImageOutputDir `
            -Recurse `
            -Force
    }

    Write-ExperimentLog "Result: $OutputPath"
    Write-ExperimentLog "Images: $ImageOutputDir"
}
finally {
    if ($TemporaryDeveloperRegistration -and $null -ne $registrationState) {
        Restore-DeveloperAddIn $addInId $registrationState
    }

    if ($null -ne $hostProcess -and -not $hostProcess.HasExited) {
        try { Stop-Process -Id $hostProcess.Id -Force } catch {}
    }

    foreach ($name in $oldEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $oldEnvironment[$name])
        Set-Item -Path ("Env:" + $name) -Value $oldEnvironment[$name] -ErrorAction SilentlyContinue
        if ($null -eq $oldEnvironment[$name]) {
            Remove-Item -Path ("Env:" + $name) -ErrorAction SilentlyContinue
        }
    }

    Remove-Item `
        -LiteralPath $RuntimeRoot `
        -Recurse `
        -Force `
        -ErrorAction SilentlyContinue
}
