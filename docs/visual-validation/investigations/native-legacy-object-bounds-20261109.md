# Native legacy-object bounds in large Destatis plans

## Retained cases

### Long-running whole-grid fallback observations

The 2026-11-09 live observations distinguish elapsed runtime from completed
full-capture timing. Neither attempt had finished at this checkpoint.

| Workbook | Sheet responsible for the large plan | All regions | Regions on that sheet | Elapsed hours | Captures completed per state |
| --- | --- | --- | --- | --- | --- |
| `elterngeld-leistungsbezuege-wohnsitz-kreisebene-j-5229211187005.xlsx` | `Erläuterung` | 104,836 | 104,833 | 5.29 | 60,864 |
| `abwasser-oeffentlich-2190212199005.xlsx` | `Inhalt` | 185,942 | 185,856 | 5.26 | 60,696 |

The respective attempt identities are
`e52cdda8adbafd3ea9bd/targeted-be0670db-f76a-40d1-b665-3615fc23179f`
and `f83cbc0ae64d8359ea27/targeted-02a4537a-a33d-4378-9c14-7aa26bc591ba`.
Their retained `capture-regions.jsonl` and SQLite-backed telemetry belong to
the Destatis run `90e78249-ac4b-493e-a3e4-0068b7060d0e`.
Both original and SheetShrink lanes were still returning successful batches.
Observed eight-region batches took approximately 3.4 and 2.7 seconds,
respectively. These samples do not establish the total full-capture duration.

The offline snapshot diagnostic reproduced visual bounds `A1:XFD1048576`
on both sheets, with no conditional-formatting blocks. Content bounds were
`A3:A6` and `A1:B47`, respectively. The unsupported-object fallback disables
the finite uniform-background-tail proof. This explains the large region plan
without attributing it to a stalled Excel process.

In `Inhalt`, `drawing2.xml` contains two coincident horizontal shapes, `Line 1`
and `Line 2`. Each runs from column 0/row 1, offset `(0, 95250)` EMU, to
column 3/row 1, offset `(800100, 95250)` EMU. The transform has origin
`(0, 323850)` and extent `(9925050, 0)` EMU. Both use preset geometry `line`
and explicit stroke width `10795` EMU, approximately 0.85 points. Their
`headEnd` and `tailEnd` elements have no attributes. The existing
`anchor_has_unmodeled_visuals` rule treats an explicit stroke width as
unmodeled. A zero-height geometric line still has visible stroke extent.

The focused test
`office_plan::visual_plan::tests::abwasser_explicit_line_stroke_retains_unmodeled_visual_fallback`
records this reduced reproducer and its current fallback behavior. It does not
qualify the line for bounded capture. Any replacement must account for stroke
and endpoint extent, hidden or collapsed neighboring cells, and unsupported
effects, and must include native mutation controls before deployment. Record
the replacement region count and measured runtime here when those checks pass.

#### Native line-removal control

`experiments/ole-frame-controls/line_controls.py` resolves `Inhalt` through
package relationships and validates the two named, explicit-width line shapes
before removing their complete anchors from an experiment copy. It verifies
that only the resolved drawing part changes, the ZIP CRCs pass, and the source
SHA-256 remains unchanged. No production workbook is edited.

The independently supplied native area `Inhalt!A1:H50` detected exactly 1,042
changed RGBA pixels after both copies were rebuilt. The changed rectangle is
`[1, 34, 1043, 35)` in image coordinates. Both images measure 1,369 × 1,636
pixels and have identical native range geometry. Visual inspection shows the
horizontal line below the heading disappearing, with the remaining image
unchanged. This demonstrates detection of the visible zero-height line in
this supplied area. It does not prove that a future automatic area is complete.

The reference command completed in 16.3446892 seconds, including opening,
rebuilding, capturing and cleanup. This one-area mutation control is not a
before/after timing of the production selector. Source SHA-256 is
`f21f1209cfdaea28ee0df629b43bc7f48f2b4381a5168ee1421e637dd5f468fb`.
The retained reference receipt SHA-256 is
`c50532773ace9ae036b99c73691abface76214347b02c7b0b598a798419d3ab8`.
Inputs, images, measurements and the successful owned-process launcher receipt
are retained under `G:\deltasieve-work\abwasser-line-controls-20261109`.
Object qualification was disabled because the reference object guard currently
accepts images and charts, with no qualified line envelope.

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

## Worksheet OLE namespace observations

The worksheet-byte scanner now records the actual lexical namespace URI for
each object and child element, and for each literal `r:id` attribute. A sparse
scope stack tracks declaration changes and restores parent bindings. Worksheet
cell data is still streamed and does not become a DOM. Unknown prefixes remain
explicit unknown observations. Duplicate declarations on one element fail.

Focused tests cover inherited foreign default namespaces, local relationship
prefix rebinding, restoration after empty and nested elements, unknown prefixes
and duplicate declarations. These checks prevent a later qualifier from using
only spelling or a global namespace-declaration inventory as evidence. They
leave the existing unsupported-object rule in place. Full DrawingML/VML
validation, anchor/native-frame agreement and all-object accounting remain
separate prerequisites.

Nine focused OLE tests passed. The explicitly enabled retained-file diagnostic
then inspected the real Word and PDF fixtures without opening Excel, taking
0.03 and 0.07 seconds respectively after compilation. Both resolved the object,
property and anchor-container elements to SpreadsheetML, the coordinate elements
to spreadsheet DrawingML, and `r:id` to the Office relationship namespace.
Both Choice and Fallback records retained their own observations.

The diagnostic receipts are retained beside the original experiment evidence.
Their SHA-256 identities are
`52b7bf700e326cf44ecb355e953cf834df41c7b6112c4b05de0ba3629727349f`
for Word and
`506bc92da1a48b3a5713251df122eaf8d3336b5173e9b1bea9f9f25daf31602f`
for PDF. No production capture rule or corpus verdict changed during this step.

## Representation namespace follow-up

The same lexical-scope tracker now records expanded names in recognized
DrawingML identity and VML shape subtrees. VML image relationship attributes
retain the actual `o:relid` namespace. A shared flattened observation structure
preserves the worksheet JSON field names. Ten focused OLE tests passed, including
local DrawingML compatibility-prefix rebinding, VML relationship-prefix
rebinding and restoration for the following shape.

Explicit XML-only inspection of the retained Word and PDF fixtures resolved
their recognized DrawingML, VML and Office relationship names as expected.
The representation-aware diagnostic receipt SHA-256 identities are
`c7f6212281fe2470c695f23b2a1065e93b46791591ed7f41230e7a25732b3199`
for Word and
`27e2500627a3305391817302adf08865c652678fab1a3aae0f9564487d1560c4`
for PDF. The diagnostics took 0.02 and 0.01 seconds after compilation.
This establishes namespace observations for the recognized subtrees. It does
not establish support for every representation property, rendering effect or
object frame. Production capture rules and native reference qualification
remain unchanged.

## Explicit VML point-frame observations

The representation scanner now parses the observed absolute VML CSS frame into
left, top, width and height in points. It records absent, parsed or unsupported
states explicitly. Unknown properties, duplicate declarations, rotation,
unsupported length units, missing coordinates, invalid values and overflowing
rectangles do not produce a parsed frame. Stacking order and the observed tight
wrap setting are validated separately from the point lengths.

Twelve focused OLE tests passed. XML-only inspection of the retained inputs
then matched their DrawingML compatibility IDs to VML IDs and native object
names. The original Word and PDF input hashes exactly match those retained in
the earlier native reference receipts. Their four parsed frame coordinates
agree with the native rectangles with zero point difference.

The existing moved-PDF fixture has top 159.75 points, compared with the original
native top of 114.75. The independent comparison found the expected 45-point
mismatch. This prevents interpreting any parsed style as agreement with an
arbitrary native observation. The private `vml-native-frame-checks.json` receipt
SHA-256 is
`3d6a1b4a108abcaf6e31d69743ca83a8940321d6be26170b6689bc82a64a1c89`.
All three XML inspections reused existing inputs and native evidence, without
starting Excel. This step leaves OLE qualification and capture coverage unchanged.

## Cross-representation correspondence

The metadata scanner now groups the worksheet's anchored and fallback OLE
records by shape ID and checks their attribute and namespace agreement. It
requires a unique matching DrawingML identity, compatibility ID and VML shape.
Worksheet and VML preview targets and hashes must agree. Eight worksheet anchor
coordinates must exactly match the legacy anchor, using 9,525 EMUs per legacy
pixel. Foreign relationship namespaces, external targets, conflicting previews,
duplicate identities, malformed coordinates and inverted anchors remain
unresolved. Missing or unsupported VML style frames remain unresolved too.

The focused comparison module run passed 151 tests with 14 explicit integration
experiments ignored. The retained-file diagnostic was then enabled separately
for the real Word, PDF and coherently moved PDF cases. All three produced one
matched correspondence. The moved case correctly retained its updated top of
159.75 points, demonstrating that correspondence is not an unchanged-geometry
verdict. The corresponding private receipt SHA-256 identities are
`926e4d239f3b54b1523df4ae45d2b5b0f6015f4b548b028ff2d221d44cefa575`,
`7a0dd2d3a939b2ac92ceb8ece725583e5131fad2cb53993f36f68bdb53c0ae33`
and `9c1d7c5f1ada439b7271fa815d8b17dc6a1263ca5f8a69c943277dc0b77293ff`.
This step leaves full property validation, native-frame qualification and
all-object accounting open. The production runtime was not replaced.

## Guarded VML picture-template observations

The VML scanner now recognizes a closed Office-generated type-75 picture family.
The check covers its complete part structure, namespaces, template attributes,
standard formula sequence and unique required picture properties. Explicit
point frames must have positive dimensions. Unknown elements or attributes,
duplicate/missing properties, cropping and rotation remain unsupported.

Checking only the shape's immediate children would miss inherited effects.
Microsoft documents that shadows can occur in either a shape or its shape type,
and that image data can include cropping and color transformations. The new
guard rejects these additions at both levels. OLE automatic loading is a
separate behavior and is not established by a recognized VML template.
[Microsoft VML shadow documentation](https://learn.microsoft.com/en-us/windows/win32/vml/web-workshop---how-to-use-vml-on-web-pages-----shadow--element),
[Microsoft VML image-data documentation](https://learn.microsoft.com/en-us/windows/win32/vml/msdn-online-vml-imagedata-element),
[Microsoft OLE AutoLoad documentation](https://learn.microsoft.com/en-us/office/vba/api/excel.oleobject.autoload).

Sixteen focused OLE tests passed. XML-only checks recognized the real Word and
PDF VML parts while correctly leaving their DrawingML parts not applicable to
this VML-specific guard. The retained diagnostic receipt SHA-256 identities are
`cfa6a0ab15c47fb00673d864094755a37ccb5f2485b0e2bf2134183b8601fcfd`
for Word and
`8084cd89562f4465ccc2765630765410db963e781038c53bf13f17ef9a60dfca`
for PDF. This step preserves the existing native reference qualification and
production capture rules.

## Zero-area text-box visibility controls

The same retained workbook has three zero-size `Text 1` shapes on Deutschland,
West and Ost VZ sheets, linked through drawings 2, 3 and 4. Relationship-resolved
removal controls found zero changed pixels on all three independently supplied
areas. Deliberately expanded controls found 1,531, 1,737 and 1,487 changed pixels,
respectively. The expanded images were inspected and contain a visible framed
text box. Their exact areas, identities, timings and an initial sheet/part mapping
error are retained in the [mutation control experiment](../experiments/ole-frame-controls/README.md#zero-area-text-box-controls).

These observations apply to the retained clipped, rectangular zero-size shapes.
They do not justify ignoring every zero-area shape, nor do they close the native
object coverage gate. The experiment leaves production files and selection rules
unchanged.
