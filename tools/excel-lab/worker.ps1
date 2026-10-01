[CmdletBinding()]
param(
    [ValidateSet('build','measure','self-test')][string]$Action,
    [Parameter(Mandatory=$true)][string]$ToolRoot,
    [Parameter(Mandatory=$true)][string]$RunRoot,
    [Parameter(Mandatory=$true)][string]$StateFile,
    [Parameter(Mandatory=$true)][string]$OwnerToken,
    [string]$PlanFile,
    [string]$ProgressFile,
    [string]$CancelFile
)
Set-StrictMode -Version 2.0
$ErrorActionPreference='Stop'
$OutputEncoding=[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
. (Join-Path $ToolRoot 'ExcelInterop.ps1')
$session=$null; $hostBook=$null; $project=$null; $components=$null; $component=$null
$temp=Join-Path ([IO.Path]::GetTempPath()) ('xl-lab-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$exitCode=0
try {
    Write-LabStatus "Worker action=$Action; local staging=$temp"
    $session=Start-LabExcel -StateFile $StateFile -OwnerToken $OwnerToken
    $localHost=Join-Path $temp 'LabHost.xlsm'
    if ($Action -eq 'build') {
        $sources=@(Get-LabSources $ToolRoot)
        $signature=Get-LabSignature $sources
        Write-LabStatus ('Building modules: ' + (($sources | ForEach-Object {$_.name}) -join ', '))
        $hostBook=Invoke-LabComMethod -Target $session.Books -Method 'Add' -Arguments ([object[]]@([int]-4167))
        try {
            $project=$hostBook.VBProject
            if ($null -eq $project) { throw 'Workbook.VBProject returned null.' }
            $components=$project.VBComponents
            if ($null -eq $components) { throw 'VBProject.VBComponents returned null.' }
        } catch {
            throw ('Cannot access VBA project for importing the reviewed BAS sources. ' + (Format-LabException $_) +
                ' A disabled Trust access to the VBA project object model setting or an Office policy is one possible cause, not a diagnosis from this generic error alone. No registry/trust setting is changed. Enable project access explicitly for the build, then disable it again, or use a permitted developer workstation to build this host.')
        }
        $strictUtf8=[Text.UTF8Encoding]::new($false,$true)
        $ansi=[Text.Encoding]::GetEncoding([Text.Encoding]::Default.CodePage, [Text.EncoderFallback]::ExceptionFallback, [Text.DecoderFallback]::ExceptionFallback)
        foreach ($source in $sources) {
            $text=$strictUtf8.GetString([IO.File]::ReadAllBytes($source.path)).TrimStart([char]0xFEFF)
            if ($text -notmatch '(?im)^Attribute VB_Name\s*=') { $text='Attribute VB_Name = "'+$source.name+'"'+"`r`n"+$text }
            $localBas=Join-Path $temp ($source.name+'.bas')
            [IO.File]::WriteAllText($localBas, ($text.Replace("`r`n","`n").Replace("`n","`r`n")), $ansi)
            Write-LabStatus ('Importing ' + $source.name)
            $component=Invoke-LabComMethod -Target $components -Method 'Import' -Arguments ([object[]]@($localBas))
            if ([string]$component.Name -cne [string]$source.name) { throw 'VBA import renamed a module; name collision.' }
            Release-LabCom $component; $component=$null
        }
        $buildBas=Join-Path $temp 'LabBuild.bas'
        $buildSource='Attribute VB_Name = "LabBuild"'+"`r`n"+'Option Explicit'+"`r`n"+'Public Function Signature() As String'+"`r`n"+'    Signature = "'+$signature+'"'+"`r`n"+'End Function'+"`r`n"
        [IO.File]::WriteAllText($buildBas,$buildSource,[Text.Encoding]::ASCII)
        $component=Invoke-LabComMethod -Target $components -Method 'Import' -Arguments ([object[]]@($buildBas))
        Release-LabCom $component; $component=$null
        Release-LabCom $components; $components=$null; Release-LabCom $project; $project=$null
        $hostBook.SaveAs($localHost,52)
        $hostBook.Close($false); Release-LabCom $hostBook; $hostBook=$null
        $hostBook=Open-LabWorkbook -Excel $session.Excel -Books $session.Books -Path $localHost -Helper
        $prefix="'$($hostBook.Name.Replace("'","''"))'!"
        Write-LabStatus 'BEGIN VBA fixture self-tests: SHA256, typed values, bounds, row batches, owned pixel buffers'
        $answer=[string]$session.Excel.Run(($prefix+'LabSelfTest.Run'),$false)
        if ($answer -cne 'ok') { throw "VBA self-test did not return ok: $answer" }
        if ([string]$session.Excel.Run(($prefix+'LabBuild.Signature')) -cne $signature) { throw 'Persisted VBA host signature mismatch.' }
        if ([string]$session.Excel.Run(($prefix+'LabMain.Version')) -cne 'excel-lab-v2') { throw 'Persisted VBA host API mismatch.' }
        $hostBook.Close($false); Release-LabCom $hostBook; $hostBook=$null
        $destination=Join-Path $ToolRoot 'ExcelLabHost.xlsm'
        $metadata=Join-Path $ToolRoot 'ExcelLabHost.manifest.json'
        $backup=Join-Path $RunRoot 'previous-host'
        [void][IO.Directory]::CreateDirectory($backup)
        foreach ($path in @($destination,$metadata)) {
            if ([IO.File]::Exists($path)) { [IO.File]::Copy($path,(Join-Path $backup ([IO.Path]::GetFileName($path))),$false) }
        }
        $info=[ordered]@{schema='excel-lab-host-v1';signature=$signature;host_sha256=(Get-FileHash -LiteralPath $localHost -Algorithm SHA256).Hash.ToLowerInvariant();sources=$sources;excel_version=[string]$session.Excel.Version}
        # Publish only after the saved binary was successfully reopened. A crash
        # between these writes leaves a detectable hash mismatch, not silent trust.
        [IO.File]::Copy($localHost,$destination,$true)
        Write-LabJson $info $metadata
        Write-LabStatus "DONE host build: $destination signature=$signature"
    } else {
        $hostInfo=Get-LabHost $ToolRoot
        [IO.File]::Copy($hostInfo.Path,$localHost,$false)
        $hostBook=Open-LabWorkbook -Excel $session.Excel -Books $session.Books -Path $localHost -Helper
        $prefix="'$($hostBook.Name.Replace("'","''"))'!"
        if ([string]$session.Excel.Run(($prefix+'LabBuild.Signature')) -cne $hostInfo.Signature) { throw 'Loaded host signature mismatch.' }
        if ($Action -eq 'self-test') {
            $session.Excel.Visible=$true
            Write-LabStatus 'BEGIN real VBA/clipboard self-test using disposable workbooks (no user workbook)'
            if ([string]$session.Excel.Run(($prefix+'LabSelfTest.Run'),$true) -cne 'ok') { throw 'VBA/clipboard self-test failed.' }
            Write-LabStatus 'PASS VBA/clipboard self-test'
        } else {
            if (-not $PlanFile) { throw 'Measurement requires a plan file.' }
            $xml=[xml][IO.File]::ReadAllText($PlanFile)
            $localOutput=Join-Path $temp 'artifacts'
            [void][IO.Directory]::CreateDirectory($localOutput)
            $xml.DocumentElement.SetAttribute('output',$localOutput)
            foreach ($pair in $xml.SelectNodes('/excel-lab/pairs/pair')) {
                foreach ($variant in @('original','shrunk')) {
                    $source=[string]$pair.GetAttribute($variant)
                    if (-not $source) { continue }
                    $destination=Join-Path $temp ($variant+'-'+$pair.GetAttribute('id')+'.xlsx')
                    Test-LabWorkbookPackage -Path $source
                    $before=(Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
                    [IO.File]::Copy($source,$destination,$false)
                    if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -cne $before) { throw "Input copy hash mismatch: $source" }
                    $pair.SetAttribute($variant,$destination)
                }
            }
            $localPlan=Join-Path $temp 'request.xml'
            $xml.Save($localPlan)
            # The complete XML is read once by VBA. Only this one Run call crosses
            # the process boundary for all measurements and capture variants.
            $session.Excel.Visible=($xml.DocumentElement.GetAttribute('screenshots') -eq 'true' -or $xml.SelectNodes('/excel-lab/overflow/probe').Count -gt 0)
            Write-LabStatus 'BEGIN single VBA batch (progress is written to vba-progress.log)'
            if ([string]$session.Excel.Run(($prefix+'LabMain.RunBatch'),$localPlan,$ProgressFile,$CancelFile) -cne 'ok') { throw 'VBA batch did not complete.' }
            if (-not [IO.File]::Exists((Join-Path $localOutput 'measurement-complete.json'))) { throw 'Missing VBA completion marker.' }
            Write-LabStatus 'DONE VBA batch'
        }
    }
} catch {
    $exitCode=1
    [Console]::Error.WriteLine('EXCEL LAB WORKER ERROR: '+(Format-LabException $_))
    if ($_.ScriptStackTrace) { [Console]::Error.WriteLine($_.ScriptStackTrace) }
} finally {
    Release-LabCom $component; Release-LabCom $components; Release-LabCom $project
    if ($null -ne $hostBook) { try { $hostBook.Close($false) } catch { }; Release-LabCom $hostBook; $hostBook=$null }
    Close-LabExcel $session
    # Preserve even partial diagnostic artifacts, but never invent completion.
    $localOutput=Join-Path $temp 'artifacts'
    if ([IO.Directory]::Exists($localOutput)) {
        try {
            foreach ($item in (Get-ChildItem -LiteralPath $localOutput -Force)) { Copy-Item -LiteralPath $item.FullName -Destination $RunRoot -Recurse -Force }
        } catch { $exitCode=1; [Console]::Error.WriteLine('Artifact copy failed. Local files retained at '+$temp+': '+$_.Exception.Message) }
    }
    if ($exitCode -eq 0) {
        # Fixed, newly created private temp directory; not a path from the plan.
        try { [IO.Directory]::Delete($temp,$true) } catch { Write-LabStatus ('Temporary workdir retained: '+$temp) }
    } else { Write-LabStatus ('Failure diagnostics/local copies retained: '+$temp) }
}
exit $exitCode
