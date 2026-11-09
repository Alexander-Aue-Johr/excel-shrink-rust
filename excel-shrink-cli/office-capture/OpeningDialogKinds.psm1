function Get-TargetedOpeningDialogKind {
    <# .SYNOPSIS Classify visible dialog text; worksheet protection or security bars are not dialogs. #>
    param([string[]]$Names, [bool]$HasPasswordInput = $false)
    $text = $Names -join " "
    if (@($Names | Where-Object { $_ -match '^(?:Name Conflict|Namenskonflikt)$' }).Count -gt 0) { return 'name_conflict' }
    if ($text -match 'AADSTS50020|does not exist in tenant') { return 'tenant_access_denied' }
    if ($text -match '\bAADSTS\d+\b|trouble signing you in|sign in to (?:your account|access)' -or
        @($Names | Where-Object { $_ -match '^(?:Microsoft\s+)?(?:Sign in|Anmelden)$' }).Count -gt 0) { return 'authentication_required' }
    if ($HasPasswordInput -or $text -match 'password|kennwort|passwort') { return 'password_required' }
    if ($text -match 'found a problem with some content|problem.*(?:einigen )?inhalten|unreadable content|unlesbare.*inhalte|repair.*workbook|arbeitsmappe.*repar') { return 'repair_required' }
    if ($text -match 'compatibility checker|kompatibilit[a\u00e4]tspr[u\u00fc]fung') { return 'compatibility_confirmation_required' }
    if ($text -match 'locked for editing|f[u\u00fc]r die bearbeitung gesperrt|zur bearbeitung gesperrt|being used by another') { return 'file_locked' }
    if ($text -match 'read.only|schreibgesch[u\u00fc]tzt|schreibschutz') { return 'read_only_prompt' }
    if ($text -match 'contains links|update.*links|aktualisieren.*verkn[u\u00fc]pf|verkn[u\u00fc]pf.*aktualisieren') { return 'external_links_prompt' }
    if ($text -match 'file format and extension|dateiformat.*dateierweiterung') { return 'format_mismatch' }
    if ($text -match 'circular reference|zirkelbez[u\u00fc]g') { return 'circular_reference' }
    if ($text -match 'cannot open|could not open|couldn.t open|kann.*nicht.*ge[o\u00f6]ffnet|datei.*besch[a\u00e4]digt') { return 'file_open_error' }
    if ($text -match 'security warning|sicherheitswarnung|protected view|gesch[u\u00fc]tzte ansicht') { return 'security_prompt' }
    if ($text -match 'not enough memory|out of memory|nicht gen[u\u00fc]gend.*speicher') { return 'resource_error' }
    return $null
}

Export-ModuleMember -Function Get-TargetedOpeningDialogKind
