# Native legacy-object bounds in large Destatis plans

## Retained cases

The Elterngeld workbook's retained measured plan contains 104,836 shared
capture regions, including many near row one million on `Erläuterung`. That
worksheet has only 18 serialized cells and dimension `A3:I6`. Its visible PDF
icon is an embedded OLE object with an explicit cached EMF preview and an
object-property anchor from column 1/row 7 to column 2/row 14, using zero-based
coordinates plus offsets. The source is
`elterngeld-leistungsbezuege-wohnsitz-kreisebene-j-5229211187005.xlsx`.

The Baumschulerhebung `Vorbemerkung` sheet similarly has a cached preview of an
embedded Word document. Its object-property anchor covers approximately A1:H51.
The current static drawing scanner marks OLE relationships and DrawingML
`AlternateContent` as requiring fallback. It does not extract this OLE envelope.

The separate `zusammen-eheschliessungen-geborene-gestorbene-5126102217005.xlsx`
case contains a geometric text box with identical from/to markers and a zero
transform extent. Its explicit line width triggers the current drawing fallback.

## Independent native observations

Three bounded reference attempts collected native object inventories before
qualification. Each retained its expected qualification failure rather than
silently accepting an unsupported object kind.

| Case | Native name/type | Left/top, points | Width/height, points |
| --- | --- | --- | --- |
| Ost VZ text box | Text 1 / GeometricShape | 759.75 / 1510.5 | 0 / 0 |
| Word preview | Object 3 / Unsupported | 0 / 0 | 443.25 / 642 |
| PDF icon | Object 2 / Unsupported | 65.25 / 114.75 | 114 / 85.5 |

The attempts completed with qualification failures after 9.690, 10.203 and
12.372 seconds respectively. These are reference-qualification limitations,
not workbook-opening failures and not evidence that Excel cannot render them.

Two additional explicit reference captures deliberately omitted object
qualification. `Vorbemerkung!A1:J60` visibly contains the Word document preview.
`Erläuterung!A1:K20` visibly contains the PDF icon and its label. The retained
original and Shrink images have equal dimensions and zero changed overlapping
RGBA pixels in both cases.

| Case | PNG dimensions | Elapsed seconds | Reference report SHA-256 |
| --- | --- | --- | --- |
| Word preview | 757 × 1009 | 11.3855488 | `5d87287681fb80b12e0c4e6e2da271395cf562c956b0e137a32b9c4c8b4653af` |
| PDF icon | 883 × 361 | 10.993296 | `5b860824c0a5e3716feebf8d4a8faed45c44ef9657dce8973f66be5a1cdb2db9` |

The embedded object payload and cached `image3.emf` were also byte-identical
between the retained rebuilt Original and Shrink files in each case. The Word
payload hash is `f3b559ff8673ad2385c81ae786890b58ddcf44753e3937ef50078da2da4ae176`,
with preview hash `39c6b25389fda2a00d3f28739d2848de5406aca53ee7cdc40c043e2275a06892`.
The PDF OLE payload hash is `f99ae0042035326156a1219b2a89290e3c7041f82ebded193021c323984b2720`,
with preview hash `c6874330b3a15900e100d95349861b7596f2cb9cd1a7539f0ae600286db63b5e`.

All plans, inventories, failures, rebuilt copies and images remain under
`G:\deltasieve-work\native-object-bounds-20261109`.

## Consequences and remaining acceptance

`Unsupported` from the Office.js shape type must not be interpreted as an
unrenderable or invalid workbook. The range renderer includes these two OLE
previews. The native frame observations support a finite-envelope investigation,
but the unqualified samples do not establish whole-workbook coverage or change
the current corpus verdicts.

A supported cached-OLE envelope needs to bind the worksheet object, shape ID,
preview relationship, local embedded payload and compatible DrawingML/VML
representations. Native names and measured frames must agree with the supported
package interpretation. Missing previews, links, mismatched identities,
unsupported effects and unaccounted objects must retain explicit fallback.
Moved-frame and changed-preview controls are required before enabling this in
production. Zero-area geometric shapes require a separate visibility proof.
