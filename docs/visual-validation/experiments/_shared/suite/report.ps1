$script:VVSuiteAssetRoot=$PSScriptRoot
function Connect-VVCaseReport([string]$CaseRoot) {
    $asset=Join-Path $CaseRoot 'report-assets\inspector.js'
    if(Test-Path -LiteralPath $asset) {
        $text=[IO.File]::ReadAllText($asset)
        $text=$text.Replace("link('Full evidence','review.zip','button'),",'')
        $text=$text.Replace("el('a','button','All runs')","el('a','button','Suite overview')").Replace("overviewLink.href='../../index.html'","overviewLink.href='../../report.html'")
        $text=$text.Replace('A/B uses separate working copies. Fixed A-then-B order does not control cache/order effects.','Part of a configured comparison suite. See its overview for input, round and order controls.')
        [IO.File]::WriteAllText($asset,$text,[Text.UTF8Encoding]::new($false))
    }
}
