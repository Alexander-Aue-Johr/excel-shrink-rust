# Real-input reference sample reconciliation

## Historical Enron development sample

The existing seeded twelve-occurrence Enron sample was reconciled on 2026-11-09
against its frozen selection, prepared member identities, reference images and
native launcher receipts. The selection SHA-256 is
`921c9fb98808c7c6583fec284ee0d01eaef6861a35ad6309973c0d001839b309`.
No source occurrence was replaced after its outcome became known.

| Outcome | Selected workbooks | Native rectangles retained |
| --- | --- | --- |
| Completed native reference | 7 | 33 |
| Rejected at image-size preflight | 2 | 0 |
| Unsupported graphical or embedded envelope | 3 | 0 |
| Total selected | 12 | 33 |

All retained source, candidate and image hashes were checked against the receipts.
Launcher success was required alongside the native reference status. The 33
completed rectangles contain zero differing overlapping RGBA pixels. Two
rectangles have unequal image extents.

- `0966d3e126908018b94a`, `Capital Structure!A1:D15`, has images 817 by 264
  and 817 by 263. The additional 817 pixels are opaque white.
- `62927fc6b2b57ebb6a87`, `Sheet1!A1:C268`, has images 765 by 1093 and
  765 by 837. The additional 195,840 pixels are opaque white.

The original raw dimension/pixel inequality observations remain retained. The
later interpretation records exterior canvas under the agreed overlap contract.
Neither image was resized or resampled to obtain equality. This is a historical
development sample with declared stored-dimension rectangles. It is not held out,
does not certify ink outside those rectangles and does not estimate selector
recall. The two failed and three unsupported inputs remain in the denominator.

The working evidence is under `G:\deltasieve-work\reference-samples-20261108`.
`reconciliation-20261109.json` retains all twelve identities, receipt hashes,
native failures, reference counts and the exact overlapping/exterior pixel totals.
This reconciliation reread earlier observations. It did not restart the old
Enron corpus or execute new native trials.

## Destatis selection frozen on 2026-11-09

The same versioned outcome-independent sampling tool selected three occurrences
per size quartile from the full 1,182-member Destatis archive. Its archive SHA-256
is `f0b39b1a30778a80227b01ae64109db964c6f71f83e6ccb3097e2f9e26f4ac45`.
All twelve selected preparation bundles and member identities were verified.

All twelve were classified unsupported by the current stored-dimension envelope
preparer because their packages contain graphical or embedded parts. No native
reference claim is made for this sample. The selected identities remain fixed
under `G:\deltasieve-work\reference-samples-20261109\destatis`, with their twelve
unsupported outcomes in the sibling `destatis-inputs/preparation.json`.

Inspection of three selected packages found normal worksheet pictures, including
two-cell anchors, and one package also contains an embedded Word document and
alternate drawing markup. A blanket dimension-only reference is inadequate for
that population. The next envelope implementation must account for actual object
extent or use an independently supplied bounded area with explicit coverage
limits. Choosing replacement inputs without graphics would change the experiment
and is not a remedy for these unsupported outcomes.
