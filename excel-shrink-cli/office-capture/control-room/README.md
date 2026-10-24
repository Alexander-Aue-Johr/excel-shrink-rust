# Live control room

The resident host serves this directory from a loopback-only control-room instance.
It needs no npm installation and makes no requests to a CDN. Three.js 0.180.0 and
OrbitControls are vendored with their MIT license and download provenance.

The UI reads `/api/state` and the selected `/api/groups/{id}`. Controls are partial
JSON updates to `/api/control`; acknowledged values are shown separately from an
unsubmitted edit. Images are accepted only from same-origin `/data/` paths.

Every worksheet has an equally sized 16:9 card in a curved grid. Each card holds
the original and three profile mosaics. Planned outlines and arriving images use
the actual per-profile pixel coordinates, including horizontal offsets and changed
row heights. Capture jobs currently remain full-width guarded row bands; the
visual inspection grid does not change the scientific capture boundaries.

The overview fits whole worksheets. Selecting a capture focuses the same pixel
window in all four profiles; panning and zooming do not create capture jobs.
The minimap previews sheets on hover and focuses them on click. Manual navigation
stops auto-follow; the follow toggle resumes at the current capture. Separate
buttons navigate differing inspection regions, worksheets, and workbook groups.

Hover details retain a separate original reference and combine variants only
where pixel equality is proven. Host-provided inspection regions supply exact
regional equivalence and difference counts. Older results get bounded display
crops without invented regional classifications. Full images and masks are
cropped before GPU upload/downscaling so fine changes survive even in captures
wider than the GPU's maximum texture size. The linked PNG remains the complete
saved capture.

Only host-provided pixel classes determine equivalence. Missing classes remain
unclassified, and the original always has a separate detail panel. Red pulse masks
are supplied by the host. No browser hashing or illustrative progress is used.

The group list initially renders 100 rows, with a filename/corpus filter and a
load-more button. The selected group remains reachable outside that window. Only
visible capture rectangles are textured, with two image decodes in flight; old
unused textures are disposed and queued loads outside the new viewport cancelled.
Pending tile outlines share a line mesh per profile, and image previews are
bounded per card; large plans do not allocate one image texture per pending job.

Capture target megapixels, batch size, and hidden Excel launching apply to newly
admitted groups. Their effective settings and measured physical capture throughput
are shown separately. Each Excel card displays its verified process status, PID,
active command and errors. The green heart pulses only when the real heartbeat
counter advances; its glow decays from the last heartbeat timestamp. Ordinary
activity is not treated as a heartbeat, and silence is not classified as death.

Restart actions POST `/api/groups/{id}/restart`; an optional `client_id` identifies
the problematic instance. Both actions restart the complete comparison group so
that all peer captures belong to one attempt. The UI reports the host's request
acknowledgment and then follows the scheduler's cleanup/requeue status.

Pure view-model checks can run using `npm test`, or by importing
`/tests/model.test.js` in the browser. `window.__observatory.snapshot()` and
`.geometry()` expose the same selected state and world-space connector endpoints
for diagnostics and browser checks. These helpers do not alter runner controls.
