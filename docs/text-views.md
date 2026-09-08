# Prepared terminal views

`prepare_text_diagram` parses and compiles Mermaid once, without laying it out.
`render_prepared_text` projects an immutable copy and returns a plain terminal
canvas, cell rectangles, original identities, provenance and diagnostics.

```rust
use mmdflux::{prepare_text_diagram, render_prepared_text, TextLimits, TextRequest};

let prepared = prepare_text_diagram("flowchart LR\nA-->B", &TextLimits::default())?;
let mut request = TextRequest::default();
request.config.text_layout_spacing = true;
request.config.layout.node_sep = 8.0;
request.config.layout.rank_sep = 15.0;
request.config.layout.edge_sep = 8.0;
request.preferred_width = Some(120);
request.preferred_height = Some(40);
let result = render_prepared_text(&prepared, &request)?;
// Render result.text without wrapping. Pan and search do not need another solve.
# Ok::<(), mmdflux::TextDiagramError>(())
```

The preferred dimensions are soft targets, not clipping instructions. A bounded
deterministic candidate search tries label wrapping and, only with explicit
permission, an alternate root direction. It considers both width and height;
it does not claim a thousand-row graph is compact because its width fits.
`effective` reports the selected hints, candidate count and remaining overflow.
Explicit `Override` changes the root direction; `Preserve` never does. Subgraph
direction declarations are retained. Sequence order is never changed.

For a preview and viewer that must keep identical geometry across viewport
sizes, set `max_candidates = 1` and use the same `label_wrap`, direction,
spacing and semantic view. Preferred dimensions then only report overflow;
they do not authorize an automatic label-width change.

## Terminal spacing

Interactive consumers should enable `RenderConfig.text_layout_spacing`. This
carries node/rank spacing into the renderer's cell-gap policy instead of applying
it only to the float-space solve. Legacy defaults stay opt-out for compatibility.
The minimum requested cell gap is `ceil(layout units / 5)`, with floors of four
horizontal columns and three vertical rows, increased when edge labels require
more. Node gaps apply across the flow; rank gaps apply along it. Authored
direction overrides inside subgraphs use their own axes for these minima, not
the parent diagram's axes. Explicit spacing also bypasses legacy fixed-gap
compaction around override groups, preserving their solved separation.
The historical float-to-grid calibration stays fixed so it does not divide away
the requested solver spacing. Graphs without subgraphs use the solver's explicit rank metadata
instead of merging nearby ranks with a coordinate-distance heuristic.
Empty dummy/label ranks have no node extent and do not add phantom gaps.

For opted-in ungrouped horizontal layouts, label clearance is local to the
labelled connection, not a global minimum derived from the widest label.
Bounded ungrouped captures also shorten straight connector-only corridors while
retaining node/rank minima and an edge-spacing retention floor. Node boxes,
text spaces, complete graphemes, arrows, junctions and corners are protected.
Route coordinates and node rectangles are remapped with the emitted cells.
Compound/subgraph layouts retain their existing border and routing treatment.

Edge spacing remains the solver's dummy-lane separation, not a universal
edge-to-edge clearance guarantee. A linear chain can legitimately be unaffected
by node/edge spacing. Sequence rendering does not consume graph spacing.
Collapsed overviews do consume the same terminal gap policy as full graphs.

## Semantic navigation

The prepared catalog includes every original node and visible container, even
when absent from the current view. IDs distinguish node and subgraph namespaces;
ancestor lists are nearest-first. Use `capabilities()` before showing controls.

- `view.overview` collapses visible top-level containers, excluding internal note
  groups. `expanded_subgraphs` provides exceptions and reveals their ancestors.
- `collapsed_subgraphs` permits more detailed nested collapse. A collapsed node's
  `members` contains the original node IDs it represents.
- `TextFocus::one_hop(id)` selects both incoming and outgoing neighbors from the
  canonical graph. It reveals the anchor's ancestors and retains attached note
  companions. Other traversal directions/hop counts are explicit.
- Class `show_class_members = false` hides only member compartments. Headers,
  annotations, the original catalog and original source remain intact.

Every original graph edge ordinal belongs exactly once to visible edge membership,
collapsed internal edges, omitted boundary edges, wholly outside-focus edges, or
intentionally invisible layout edges. Aggregation requires equal endpoints,
labels, endpoint labels, markers and styles; aggregates display their multiplicity.
`TextEdge.routed` independently reports actual route capture: semantic accounting
alone is not proof that an edge was drawn. Geometry or text-emission limitations
are reported in `diagnostics`, never disguised as successful compact output.

Cell rectangles correspond to the final serialized canvas after empty-row and
left-padding removal. They are not MMDS/SVG pixel rectangles. Existing MMDS
`views::project` retains its separate shared-coordinate contract.

## Admission and concurrency

`TextLimits` bounds source/label bytes, graph/event counts, nesting, conservative
rank expansion, candidate count, and canvas dimensions/cell product. Canvas
growth for late labels is checked too. Prepared data is revalidated against each
request's limits, so tightening a limit cannot bypass admission through a cache.
Errors distinguish parse, unsupported, invalid request, limit and renderer errors.
An optional shared atomic `TextRequest.cancellation` flag produces a distinct
`Cancelled` error between projection/solve/emission stages and candidates. It
never mutates the prepared data, which remains reusable after cancellation.

Call the renderer off the UI thread. The facade is synchronous and does **not**
promise a wall-clock deadline or hard cancellation of an active solve. Consumers
should bound queued work, invalidate stale results, and bound prepared/result
caches separately. A source remains accessible after any view or render failure.

The legacy `render_diagram` API remains available and its default behavior is not
changed by these request policies. Prepared views accept Mermaid source; MMDS
replay continues through `render_document`.

For a reproducible probe:

```sh
cargo +1.95.0 run --release --example text_view -- diagram.mmd 120 overview compact terminal
cargo +1.95.0 run --release --example text_view -- diagram.mmd 120 overview 8,50,8 terminal
```
