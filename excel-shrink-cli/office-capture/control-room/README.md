# Live control room

The resident host serves this directory from a loopback-only control-room instance.
It needs no npm installation and makes no requests to a CDN. Three.js 0.180.0 and
OrbitControls are vendored with their MIT license and download provenance.

The UI reads `/api/state` and the selected `/api/groups/{id}`. Controls are partial
JSON updates to `/api/control`; acknowledged values are shown separately from an
unsubmitted edit. Images are accepted only from same-origin `/data/` paths.

The four profile surfaces form one concave arc. Tile geometry uses each profile's
actual capture pixel coordinates and dimensions, including changed row heights.
A common sheet viewport clips image UVs across all profiles;
horizontal and vertical panning do not create new capture jobs. Following incoming
captures stops while a detail is held, or after manual panning. Hover details use
one common pixel window within the corresponding job, preserving image aspect
ratios. Peers outside the sheet viewport remain connected through edge anchors.
The linked PNG opens the complete saved capture.

Only host-provided pixel classes determine equivalence. Missing classes remain
unclassified, and the original always has a separate detail panel. Red pulse masks
are supplied by the host. No browser hashing or illustrative progress is used.

The group list initially renders 100 rows, with a filename/corpus filter and a
load-more button. The selected group remains reachable outside that window. Only
visible capture rectangles are textured, with two image decodes in flight; old
unused textures are disposed and queued loads outside the new viewport cancelled.

Pure view-model checks can run using `npm test`, or by importing
`/tests/model.test.js` in the browser. `window.__observatory.snapshot()` and
`.geometry()` expose the same selected state and world-space connector endpoints
for diagnostics and browser checks. These helpers do not alter runner controls.
