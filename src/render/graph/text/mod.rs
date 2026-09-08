//! Low-level text drawing for graph-family rendering.
//!
//! This module owns the render-side step that consumes graph-owned grid
//! geometry and paints it onto a character canvas.

mod edge;
pub(crate) mod label_placement;
mod label_util;
mod shape;
mod subgraph;

#[cfg(test)]
pub(crate) use edge::{compute_edge_containment, render_all_edges, render_edge};

#[cfg(test)]
pub(crate) fn required_canvas_size_for_test(
    layout: &GridLayout,
    routed_edges: &[RoutedEdge],
) -> (usize, usize) {
    required_canvas_size(layout, routed_edges)
}

#[cfg(test)]
mod regression_tests;

use super::TextRenderOptions;
use crate::format::OutputFormat;
use crate::graph::Graph;
use crate::graph::geometry::RoutedGraphGeometry;
use crate::graph::grid::{GridLayout, RoutedEdge, Segment, SubgraphBounds, route_all_edges};
use crate::render::text::canvas::{Cell, Connections};
use crate::render::text::{Canvas, CharSet, TextRenderCapture, TextRenderLimits};

/// Render text output from a derived grid layout.
pub fn render_text_from_grid_layout(
    diagram: &Graph,
    layout: &GridLayout,
    routed: Option<&RoutedGraphGeometry>,
    options: &TextRenderOptions,
) -> String {
    let charset = match options.output_format {
        OutputFormat::Ascii => CharSet::ascii(),
        _ => CharSet::unicode(),
    };

    let routed_edges = route_all_edges(&diagram.edges, layout, diagram.direction);
    let (canvas_width, canvas_height) = required_canvas_size(layout, &routed_edges);
    let mut canvas = Canvas::new(canvas_width, canvas_height);

    paint_canvas(
        diagram,
        layout,
        routed,
        &routed_edges,
        &mut canvas,
        &charset,
    );

    if options.text_color_mode.uses_ansi() {
        canvas.to_ansi_string()
    } else {
        canvas.to_string()
    }
}

pub fn render_text_capture_from_grid_layout(
    diagram: &Graph,
    layout: &GridLayout,
    routed: Option<&RoutedGraphGeometry>,
    options: &TextRenderOptions,
    limits: &TextRenderLimits,
) -> Result<TextRenderCapture, String> {
    limits.check(layout.width, layout.height)?;
    let charset = if options.output_format == OutputFormat::Ascii {
        CharSet::ascii()
    } else {
        CharSet::unicode()
    };
    let mut routed_edges = route_all_edges(&diagram.edges, layout, diagram.direction);
    let (width, height) = required_canvas_size(layout, &routed_edges);
    limits.check(width, height)?;
    let mut diagnostics = crate::graph::grid::repair_and_validate_routes(
        &mut routed_edges,
        &diagram.edges,
        layout,
        width,
        height,
    );
    let (width, height) = required_canvas_size(layout, &routed_edges);
    let mut canvas = Canvas::new_limited(width, height, *limits)?;
    let mut node_bounds = paint_canvas(
        diagram,
        layout,
        routed,
        &routed_edges,
        &mut canvas,
        &charset,
    );
    canvas.check_limits()?;
    if let Some(spacing) = options.layout_spacing
        && layout.subgraph_bounds.is_empty()
    {
        let lane_gap = (spacing.edge_sep / 5.0).ceil() as usize;
        if let Some((x_map, y_map)) = canvas.compact_corridors(
            layout.h_spacing.max(lane_gap),
            layout.v_spacing.max(lane_gap),
        ) {
            for bounds in node_bounds.values_mut() {
                bounds.width = x_map[bounds.x + bounds.width] - x_map[bounds.x];
                bounds.height = y_map[bounds.y + bounds.height] - y_map[bounds.y];
                bounds.x = x_map[bounds.x];
                bounds.y = y_map[bounds.y];
                bounds.layout_center_x = bounds.layout_center_x.and_then(|x| x_map.get(x).copied());
                bounds.layout_center_y = bounds.layout_center_y.and_then(|y| y_map.get(y).copied());
            }
            map_route_coordinates(&mut routed_edges, |x| x_map[x], |y| y_map[y]);
        }
    }
    diagnostics.extend(canvas.emission_diagnostics());
    let (text, origin, width, height) = canvas.serialize(false);
    for bounds in node_bounds.values_mut() {
        bounds.x = bounds.x.saturating_sub(origin.0);
        bounds.y = bounds.y.saturating_sub(origin.1);
        bounds.layout_center_x = bounds.layout_center_x.map(|x| x.saturating_sub(origin.0));
        bounds.layout_center_y = bounds.layout_center_y.map(|y| y.saturating_sub(origin.1));
    }
    let mut subgraph_bounds = layout.subgraph_bounds.clone();
    subgraph_bounds.retain(|_, bounds| !bounds.invisible);
    for bounds in subgraph_bounds.values_mut() {
        bounds.x = bounds.x.saturating_sub(origin.0);
        bounds.y = bounds.y.saturating_sub(origin.1);
    }
    map_route_coordinates(
        &mut routed_edges,
        |x| x.saturating_sub(origin.0),
        |y| y.saturating_sub(origin.1),
    );
    Ok(TextRenderCapture {
        text,
        origin,
        width,
        height,
        node_bounds,
        subgraph_bounds,
        routed_edge_indices: routed_edges.iter().map(|route| route.edge.index).collect(),
        routes: routed_edges,
        diagnostics,
    })
}

fn map_route_coordinates(
    routes: &mut [RoutedEdge],
    map_x: impl Fn(usize) -> usize,
    map_y: impl Fn(usize) -> usize,
) {
    for route in routes {
        route.start.x = map_x(route.start.x);
        route.start.y = map_y(route.start.y);
        route.end.x = map_x(route.end.x);
        route.end.y = map_y(route.end.y);
        for segment in &mut route.segments {
            match segment {
                Segment::Horizontal { y, x_start, x_end } => {
                    *y = map_y(*y);
                    *x_start = map_x(*x_start);
                    *x_end = map_x(*x_end);
                }
                Segment::Vertical { x, y_start, y_end } => {
                    *x = map_x(*x);
                    *y_start = map_y(*y_start);
                    *y_end = map_y(*y_end);
                }
            }
        }
    }
}

fn paint_canvas(
    diagram: &Graph,
    layout: &GridLayout,
    routed: Option<&RoutedGraphGeometry>,
    routed_edges: &[RoutedEdge],
    canvas: &mut Canvas,
    charset: &CharSet,
) -> std::collections::HashMap<String, crate::graph::grid::NodeBounds> {
    if !layout.subgraph_bounds.is_empty() {
        subgraph::render_subgraph_borders(canvas, &layout.subgraph_bounds, charset);
    }

    let mut node_keys: Vec<&String> = diagram.nodes.keys().collect();
    let mut node_bounds = std::collections::HashMap::new();
    node_keys.sort();
    for node_id in node_keys {
        let node = &diagram.nodes[node_id];
        if let Some(&(x, y)) = layout.draw_positions.get(node_id) {
            let bounds = shape::render_node(canvas, node, x, y, charset, diagram.direction);
            node_bounds.insert(node_id.clone(), bounds);
        }
    }

    let edge_containment =
        edge::compute_edge_containment(&diagram.edges, &diagram.subgraphs, &layout.subgraph_bounds);

    edge::render_all_edges_with_labels(
        canvas,
        routed_edges,
        charset,
        diagram.direction,
        &edge_containment,
        layout,
        routed,
    );

    apply_subgraph_border_junctions(canvas, &layout.subgraph_bounds, routed_edges, charset);

    node_bounds
}

fn required_canvas_size(layout: &GridLayout, routed_edges: &[RoutedEdge]) -> (usize, usize) {
    let mut width = layout.width.max(1);
    let mut height = layout.height.max(1);

    for bounds in layout.node_bounds.values() {
        width = width.max(bounds.x + bounds.width);
        height = height.max(bounds.y + bounds.height);
    }

    for bounds in layout.subgraph_bounds.values() {
        width = width.max(bounds.x + bounds.width);
        height = height.max(bounds.y + bounds.height);
    }

    for routed in routed_edges {
        width = width.max(routed.start.x.saturating_add(1));
        height = height.max(routed.start.y.saturating_add(1));
        width = width.max(routed.end.x.saturating_add(1));
        height = height.max(routed.end.y.saturating_add(1));

        for segment in &routed.segments {
            match *segment {
                Segment::Vertical { x, y_start, y_end } => {
                    width = width.max(x.saturating_add(1));
                    height = height.max(y_start.max(y_end).saturating_add(1));
                }
                Segment::Horizontal { y, x_start, x_end } => {
                    width = width.max(x_start.max(x_end).saturating_add(1));
                    height = height.max(y.saturating_add(1));
                }
            }
        }
    }

    (width, height)
}

fn apply_subgraph_border_junctions(
    canvas: &mut Canvas,
    subgraph_bounds: &std::collections::HashMap<String, SubgraphBounds>,
    routed_edges: &[RoutedEdge],
    charset: &CharSet,
) {
    if subgraph_bounds.is_empty() || routed_edges.is_empty() {
        return;
    }

    let should_skip_title_cell =
        |cell: &Cell| cell.is_subgraph_title && cell.ch != charset.horizontal && cell.ch != ' ';
    let conns_all = Connections {
        up: true,
        down: true,
        left: true,
        right: true,
    };

    for bounds in subgraph_bounds.values() {
        if bounds.width < 2 || bounds.height < 2 {
            continue;
        }

        let left = bounds.x;
        let right = bounds.x.saturating_add(bounds.width.saturating_sub(1));
        let top = bounds.y;
        let bottom = bounds.y.saturating_add(bounds.height.saturating_sub(1));

        for routed in routed_edges {
            for segment in &routed.segments {
                match *segment {
                    Segment::Vertical { x, y_start, y_end } => {
                        let (y_min, y_max) = if y_start <= y_end {
                            (y_start, y_end)
                        } else {
                            (y_end, y_start)
                        };
                        if x > left && x < right {
                            if y_min < top
                                && top <= y_max
                                && let Some(cell) = canvas.get(x, top)
                                && !should_skip_title_cell(cell)
                            {
                                set_junction_cell(canvas, x, top, conns_all, charset);
                            }
                            if y_min <= bottom
                                && bottom < y_max
                                && let Some(cell) = canvas.get(x, bottom)
                                && !should_skip_title_cell(cell)
                            {
                                set_junction_cell(canvas, x, bottom, conns_all, charset);
                            }
                        }
                    }
                    Segment::Horizontal { y, x_start, x_end } => {
                        let (x_min, x_max) = if x_start <= x_end {
                            (x_start, x_end)
                        } else {
                            (x_end, x_start)
                        };
                        if y > top && y < bottom {
                            if x_min < left
                                && left <= x_max
                                && let Some(cell) = canvas.get(left, y)
                                && !should_skip_title_cell(cell)
                            {
                                set_junction_cell(canvas, left, y, conns_all, charset);
                            }
                            if x_min <= right
                                && right < x_max
                                && let Some(cell) = canvas.get(right, y)
                                && !should_skip_title_cell(cell)
                            {
                                set_junction_cell(canvas, right, y, conns_all, charset);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn set_junction_cell(
    canvas: &mut Canvas,
    x: usize,
    y: usize,
    conns: Connections,
    charset: &CharSet,
) {
    let preserve_content = canvas.is_bounded();
    if let Some(cell) = canvas.get_mut(x, y) {
        // An edge crossing a cluster border may place its arrow on that very
        // cell. The final border-junction pass must not erase the marker (or
        // node/title text) that the bounded renderer has just audited.
        if preserve_content && (cell.is_node || cell.is_subgraph_title || charset.is_arrow(cell.ch))
        {
            return;
        }
        cell.ch = charset.junction(conns);
        cell.connections = conns;
        cell.is_edge = true;
    }
}
