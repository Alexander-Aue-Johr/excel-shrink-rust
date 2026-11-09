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

## Schema references for the implementation gate

The `objectPr` element describes visual, positional and cell-linkage properties
and contains its object anchor. This supports inspecting its from/to markers
separately from the hidden DrawingML compatibility placeholder observed in these
files. [Microsoft EmbeddedObjectProperties documentation](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.embeddedobjectproperties?view=openxml-3.0.1)

The `oleObject` shape ID identifies its DrawingML counterpart, and its
relationship identifies the persisted object part. Automatic loading and linked
object updates have distinct schema attributes. The local observations above
do not establish a general guarantee about arbitrary linked or automatically
loaded objects. [Microsoft OleObject documentation](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.oleobject?view=openxml-3.0.1)
## OLE metadata observation acceptance

The reference guard now retains local OLE payload and cached-preview identities,
worksheet anchors and Choice/Fallback object records beside the native inventory.
The worksheet is scanned as bytes with the existing streaming scanner. This
observation step leaves object qualification and production capture planning
unchanged. Namespace declarations are diagnostic observations, without a claim
of resolved markup-compatibility semantics.

Three unit tests cover the retained metadata, external targets without access,
missing relationships and malformed or duplicate coordinates. A debug build
and three isolated native attempts completed under private desktops. Evidence
is retained under `G:\deltasieve-work\native-object-bounds-20261109`.

| Case | Seconds | Result | Reference receipt SHA-256 |
| --- | ---: | --- | --- |
| `word-metadata-observation` | 9.734 | Unsupported object rejected, two OLE records retained | `d93de9b8f620b51169ddb8d4ba6c3ce0b551e89b3ca246e2c11259deb973df1c` |
| `pdf-metadata-observation` | 9.697 | Unsupported object rejected, two OLE records retained | `15a4ca58af213449b81cbe0f6bdd15ac676a16b1df61e9e4d340d25f5987565b` |
| `chart-metadata-control` | 9.885 | Existing native chart reference completed | `26e2dee58ac1b1637eab97a0203d460dfca5418282f70dafa232a153d16c7459` |

The Word and PDF payload and preview hashes matched the prior package inspection.
Both receipts preserved all eight anchor fields and the separate fallback record.
The successful chart control contained no OLE records. Bounded cached-OLE frame
qualification still requires DrawingML/VML identity binding, supported metadata
checks and negative native controls before any coverage expansion.

The subsequent [frame and preview mutation controls](../experiments/ole-frame-controls/README.md)
completed independent bounded comparisons. A coherent three-row frame movement
changed 6,473 RGBA pixels. Replacing only the cached EMF changed 5,242 pixels.
Both pairs retained identical capture geometry and their rendered images were
inspected. These controls establish that the native reference detects the two
specific mutations. Production bounded-OLE qualification remains outstanding.

## DrawingML and VML identity observations

The reference receipt now also retains the two linked representation inventories.
Each representation has a part hash. DrawingML records retain `cNvPr` identity,
native name and compatibility IDs. VML records retain the shape ID, style,
legacy anchor and preview relationship, including its target and hash. External
representation targets are recorded without fetching them. Representation XML
is bounded to one MiB, and missing VML preview links fail explicitly.

Six focused Rust tests passed, including local preview hashing, external targets,
missing links, missing identity fields and nested drawing records. Two further
native attempts retained the expected Unsupported qualification failure.

| Case | Seconds | Shape ID | Native name | Reference receipt SHA-256 |
| --- | ---: | --- | --- | --- |
| `word-object-representations` | 10.700414 | 4099 | Object 3 | `74eb905c4df2ceb962a09896499d3931aa4803d11a80baec86485de6b880eece` |
| `pdf-icon-representations` | 11.902800 | 2050 | Object 2 | `ac0fa2040b60e03252998f01392b348b940a13f3a8ab846d31a29663df8dd623` |

Explicit assertions against the retained JSON confirmed the worksheet shape ID,
DrawingML identity and `_x0000_s<id>` VML identity agree in both cases. The VML
preview target and hash also agree with the worksheet object's cached preview.
This checks these two observations only. Namespace resolution, supported-property
validation, DrawingML anchor agreement, native-frame agreement and all-object
accounting remain required for the bounded reference qualification rule.
