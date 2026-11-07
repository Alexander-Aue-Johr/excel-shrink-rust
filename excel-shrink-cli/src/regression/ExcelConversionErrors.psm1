function Get-LegacyConversionFailureKind {
    <# .SYNOPSIS
    Keeps native conversion failures consistent across fresh and pooled sessions.
    #>
    param([string]$Message, [string]$Phase)
    if ($Message -match '(?i)password|kennwort|passwort') { return 'password_required' }
    if ($Message -match '(?i)trust center|vertrauensstellungscenter|file block|dateiblock') { return 'trust_center_blocked' }
    if ($Message -match '(?i)permission|berechtigung|access denied|zugriff verweigert|rights management') { return 'access_denied' }
    if ($Message -match '(?i)corrupt|repair|beschädigt|repar') { return 'repair_required' }
    if ($Phase -eq 'saving XLSX conversion') { return 'conversion_save_failed' }
    return 'excel_automation_failed'
}

Export-ModuleMember -Function Get-LegacyConversionFailureKind
