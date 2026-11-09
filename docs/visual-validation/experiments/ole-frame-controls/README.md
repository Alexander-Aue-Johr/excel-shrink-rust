# Cached OLE preview and frame mutation controls

## Scope

This fixture-specific experiment checks whether native range images expose a
changed OLE frame and a changed cached preview. The retained Elterngeld PDF-icon
worksheet is `Erläuterung`. The plan is fixed at `A1:K24`, independently of the
production selector. Object qualification is deliberately disabled because the
current guard rejects Excel's `Unsupported` shape type for this object.

These results support the future bounded-OLE qualification work. They do not
qualify the current workbook or establish complete corpus coverage.

## Reproduce

`prepare.py` accepts the retained PDF-icon workbook and a different retained
workbook supplying a valid EMF preview. It creates a new output directory,
retains SHA-256 identities and refuses unexpected fixture fields. It does not
update either source workbook.

```powershell
python prepare.py --pdf-source 'G:\inputs\pdf.xlsx' `
    --preview-source 'G:\inputs\word.xlsx' --output 'G:\controls\inputs'

& tools/run-reference-comparison.ps1 -Binary 'G:\runtime\excel-shrink-cli.exe' `
    -Original 'G:\inputs\pdf.xlsx' -Default 'G:\controls\inputs\moved.xlsx' `
    -Plan 'G:\controls\inputs\plan.json' -Output 'G:\controls\moved' `
    -OperationTimeoutSeconds 45 -TotalTimeoutSeconds 120
```

Repeat with `preview.xlsx` and a new result directory. Use the matching capture
assets for the tested binary. The shared launcher owns its private desktop and
process cleanup. `test_prepare.py` checks source preservation across two variant
writes and rejection of absent or ambiguous fixture fields.

## Observed results on 2026-11-09

Source SHA-256 is
`ca98c91acf82c4da372010d014a238f9f6944f160a101dfe21f21b73d3bc46ca`.
The preview donor SHA-256 is
`195643acd5fe3ce4ce7f3ddad3770e81f6872bd77335440abd89b4d2c4200e35`.
Both variants rebuilt and rendered successfully. All images are 883 × 429 pixels.
The differing-pixel counts use exact RGBA comparisons across the complete images.

| Mutation | Changed package parts | Changed pixels | Seconds |
| --- | --- | ---: | ---: |
| Frame moved down three rows | Worksheet OLE anchor, DrawingML anchor, VML frame | 6,473 | 10.769487 |
| Cached preview replaced | `xl/media/image3.emf` only | 5,242 | 10.395005 |

The rendered images were inspected. The first shows the PDF icon and label lower
on the sheet. The second shows the replacement Word-page preview in the original
frame. Both compared pairs retain identical capture geometry. Package-member
comparison confirmed precisely the changed parts listed above. The source file
hash remained equal to its preparation receipt, and both output ZIP CRC checks
passed. Both generator unit tests passed.

An initial preparation attempt failed before native execution because Python's
ZIP writer mutated the source `ZipInfo` metadata when reusing it. The generator
now copies each entry descriptor before writing. The two-variant regression test
covers that failure. The partial attempt remains separate from accepted inputs.

Evidence is retained privately under
`G:\deltasieve-work\native-object-bounds-20261109`, with inputs in
`mutation-inputs-retry` and outputs in `moved-frame-control` and
`changed-preview-control`. Reference receipt hashes are respectively
`0b8f044312a244b1e7291d9e6ef298e896dc3df98eba6f77bf88ac9eeaff4ff4`
and `1d66b969c74c8308aaa086f4ef6bd87ff4ad1944b237a2b49b6e52fb9666a3b1`.
