# Visual capture bounds

Regression checks stored values first. The Office add-in then compares row heights
between the repaired baseline and the direct shrink output. If any sheet has changed
heights, it captures a representative affected row for the report and skips the
full image comparison for that case.

When values and row heights match across the case, the add-in captures paired worksheet
images with `Range.getImage()`. Rust compares the images pixel by pixel and saves
focused original, shrunk, and difference excerpts for the scrollable case report.

## Bounds and overflow

Rust derives content, merge, and drawing bounds from the workbook package. The
add-in reads Excel's rendered row and column geometry to plan image tiles. Where
text may extend beyond the package boundary, it captures extra visible columns
and checks the right edge before choosing the final image width. Both workbook
versions use the same comparison range.

An unresolved drawing boundary, overflow edge, image dimension, or missing tile
fails the sheet. It is never counted as a visual match.
