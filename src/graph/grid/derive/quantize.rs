//! Grid quantization helpers for float-to-grid derivation.
//!
//! These functions assign abstract grid coordinates, convert float-space
//! spacing into grid scale factors, and repair spacing after integer rounding.

use std::collections::HashMap;

use crate::graph::grid::{GridPos, NodeBounds};
use crate::graph::{Direction, Graph};

/// Reserve label space only across the connection that needs it. Shifting a
/// suffix preserves existing node order and cannot reduce another pair's gap.
pub(super) fn local_label_gap_repair(
    graph: &Graph,
    positions: &mut HashMap<String, (usize, usize)>,
    dimensions: &HashMap<String, (usize, usize)>,
) {
    if !matches!(graph.direction, Direction::LeftRight | Direction::RightLeft) {
        return;
    }
    let mut neighbors = HashMap::<&str, Vec<&str>>::new();
    for edge in &graph.edges {
        neighbors.entry(&edge.from).or_default().push(&edge.to);
        neighbors.entry(&edge.to).or_default().push(&edge.from);
    }
    let mut components = HashMap::<&str, usize>::new();
    for id in graph.nodes.keys() {
        if components.contains_key(id.as_str()) {
            continue;
        }
        let component = components.len();
        let mut pending = vec![id.as_str()];
        while let Some(node) = pending.pop() {
            if components.contains_key(node) {
                continue;
            }
            components.insert(node, component);
            pending.extend(neighbors.get(node).into_iter().flatten().copied());
        }
    }
    let mut constraints = Vec::new();
    for edge in &graph.edges {
        let Some(label) = &edge.label else { continue };
        let Some(&(from_x, _)) = positions.get(&edge.from) else {
            continue;
        };
        let Some(&(to_x, _)) = positions.get(&edge.to) else {
            continue;
        };
        if from_x == to_x {
            continue;
        }
        let (left, right) = if from_x < to_x {
            (&edge.from, &edge.to)
        } else {
            (&edge.to, &edge.from)
        };
        let width = label
            .lines()
            .map(crate::format::display_width)
            .max()
            .unwrap_or(0);
        constraints.push((positions[right].0, edge.index, left, right, width + 4));
    }
    constraints.sort_by_key(|&(x, index, _, _, _)| (x, index));
    for (_, _, left, right, gap) in constraints {
        let required = positions[left].0 + dimensions[left].0 + gap;
        let start = positions[right].0;
        if required > start {
            let shift = required - start;
            for (id, (x, _)) in positions.iter_mut() {
                if components[id.as_str()] == components[right.as_str()] && *x >= start {
                    *x += shift;
                }
            }
        }
    }
}

pub(super) fn compute_grid_positions(layers: &[Vec<String>]) -> HashMap<String, GridPos> {
    let mut positions = HashMap::new();

    for (layer_idx, layer) in layers.iter().enumerate() {
        for (pos_idx, node_id) in layer.iter().enumerate() {
            positions.insert(
                node_id.clone(),
                GridPos {
                    layer: layer_idx,
                    pos: pos_idx,
                },
            );
        }
    }

    positions
}

/// Compute per-axis ASCII scale factors for translating layout float coordinates
/// to character grid positions.
///
/// Returns `(scale_x, scale_y)` where each factor maps layout coordinate deltas
/// to ASCII character deltas along that axis.
///
/// For vertical layouts (TD/BT):
///   - scale_y (primary) = (max_h + v_spacing) / (max_h + rank_sep)
///   - scale_x (cross)   = (avg_w + h_spacing) / (avg_w + node_sep)
///
/// For horizontal layouts (LR/RL):
///   - scale_x (primary) = (max_w + h_spacing) / (max_w + rank_sep)
///   - scale_y (cross)   = (avg_h + v_spacing) / (avg_h + node_sep)
pub(super) fn compute_grid_scale_factors(
    node_dims: &HashMap<String, (usize, usize)>,
    rank_sep: f64,
    node_sep: f64,
    v_spacing: usize,
    h_spacing: usize,
    is_vertical: bool,
    ranks_doubled: bool,
) -> (f64, f64) {
    let (total_w, total_h, max_w, max_h, count) = node_dims.values().fold(
        (0usize, 0usize, 0usize, 0usize, 0usize),
        |(tw, th, mw, mh, c), &(w, h)| (tw + w, th + h, mw.max(w), mh.max(h), c + 1),
    );
    let count_f = count.max(1) as f64;
    let avg_w = total_w as f64 / count_f;
    let avg_h = total_h as f64 / count_f;

    if is_vertical {
        // When ranks are doubled, the layout positions nodes 2× further apart.
        // To compensate exactly, we need: eff_rs = max_h + 2 * rank_sep
        // This gives scale_primary_new = scale_primary_old / 2, so that
        // (2 * rank_sep) * scale_new = rank_sep * scale_old.
        let effective_rank_sep = if ranks_doubled {
            max_h as f64 + 2.0 * rank_sep
        } else {
            rank_sep
        };
        let scale_primary = (max_h as f64 + v_spacing as f64) / (max_h as f64 + effective_rank_sep);
        let scale_cross = (avg_w + h_spacing as f64) / (avg_w + node_sep);
        (scale_cross, scale_primary)
    } else {
        let effective_rank_sep = if ranks_doubled {
            max_w as f64 + 2.0 * rank_sep
        } else {
            rank_sep
        };
        let scale_primary = (max_w as f64 + h_spacing as f64) / (max_w as f64 + effective_rank_sep);
        let scale_cross = (avg_h + v_spacing as f64) / (avg_h + node_sep);
        (scale_primary, scale_cross)
    }
}

/// Enforce minimum spacing between adjacent nodes within each layer after
/// scaling and rounding.
///
/// Nodes are sorted by their cross-axis position within each layer, then
/// scanned left-to-right (or top-to-bottom for horizontal layouts). If any
/// adjacent pair overlaps or is too close, the later node is pushed forward.
/// This cascades: pushing node B may cause it to overlap C, which also gets pushed.
///
/// For vertical layouts (`is_vertical = true`), the cross-axis is X.
/// For horizontal layouts (`is_vertical = false`), the cross-axis is Y.
pub(super) fn collision_repair(
    layers: &[Vec<String>],
    draw_positions: &mut HashMap<String, (usize, usize)>,
    node_dims: &HashMap<String, (usize, usize)>,
    is_vertical: bool,
    min_gap: usize,
) {
    for layer in layers {
        if layer.len() <= 1 {
            continue;
        }

        let mut sorted: Vec<String> = layer.clone();
        sorted.sort_by_key(|id| {
            let &(x, y) = &draw_positions[id];
            if is_vertical { x } else { y }
        });

        for i in 1..sorted.len() {
            let prev_id = &sorted[i - 1];
            let curr_id = &sorted[i];
            let &(pw, ph) = &node_dims[prev_id];
            let (prev_x, prev_y) = draw_positions[prev_id];
            let (curr_x, curr_y) = draw_positions[curr_id];

            if is_vertical {
                let min_x = prev_x + pw + min_gap;
                if curr_x < min_x {
                    draw_positions.insert(curr_id.clone(), (min_x, curr_y));
                }
            } else {
                let min_y = prev_y + ph + min_gap;
                if curr_y < min_y {
                    draw_positions.insert(curr_id.clone(), (curr_x, min_y));
                }
            }
        }
    }
}

/// Enforce minimum spacing between adjacent layers along the primary axis.
///
/// For vertical layouts, layers stack along Y; for horizontal, along X.
/// If the closest node in the next layer is too close to the farthest node
/// in the previous layer, shift the entire next layer (and all subsequent layers)
/// forward to maintain the minimum gap.
pub(super) fn rank_gap_repair(
    layers: &[Vec<String>],
    draw_positions: &mut HashMap<String, (usize, usize)>,
    node_dims: &HashMap<String, (usize, usize)>,
    is_vertical: bool,
    min_gap: usize,
) {
    if layers.len() <= 1 {
        return;
    }

    // Pinned ranks include empty dummy/label layers. An empty layer has no
    // extent: treating its minimum as zero repeatedly shifts every later rank
    // by the preceding node's absolute position, inflating sparse layouts.
    let populated: Vec<usize> = layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| layer.iter().any(|id| draw_positions.contains_key(id)))
        .map(|(index, _)| index)
        .collect();
    for pair in populated.windows(2) {
        let previous = pair[0];
        let i = pair[1];
        // Find the maximum primary-axis extent of the previous layer
        let prev_max = layers[previous]
            .iter()
            .filter_map(|id| {
                let &(x, y) = draw_positions.get(id)?;
                let &(w, h) = node_dims.get(id)?;
                Some(if is_vertical { y + h } else { x + w })
            })
            .max()
            .unwrap_or(0);

        // Find the minimum primary-axis position in the current layer
        let curr_min = layers[i]
            .iter()
            .filter_map(|id| {
                let &(x, y) = draw_positions.get(id)?;
                Some(if is_vertical { y } else { x })
            })
            .min()
            .unwrap_or(0);

        let required = prev_max + min_gap;
        if curr_min < required {
            let shift = required - curr_min;
            // Shift all nodes in this layer and all subsequent layers
            for layer in &layers[i..] {
                for id in layer {
                    if let Some(&(x, y)) = draw_positions.get(id) {
                        let new_pos = if is_vertical {
                            (x, y + shift)
                        } else {
                            (x + shift, y)
                        };
                        draw_positions.insert(id.clone(), new_pos);
                    }
                }
            }
        }
    }
}

/// Compute rank-to-draw-coordinate mapping for waypoint transformation.
///
/// For each layout rank that contains real nodes, computes the primary-axis
/// draw-coordinate extent from `node_bounds`. For ranks without real nodes
/// (dummy/label ranks from edge normalization), linearly interpolates between
/// the nearest neighboring real-node ranks.
///
/// Returns a `Vec<usize>` indexed by rank, where `layer_starts[rank]` gives
/// the primary-axis draw coordinate for that rank.
pub(super) fn compute_layer_starts(
    node_ranks: &HashMap<String, i32>,
    node_bounds: &HashMap<String, NodeBounds>,
    is_vertical: bool,
) -> Vec<usize> {
    // Map each rank to its draw-coordinate extent (start, end) from real nodes
    let mut rank_to_actual_bounds: HashMap<i32, (usize, usize)> = HashMap::new();
    for (node_id, &rank) in node_ranks {
        if let Some(bounds) = node_bounds.get(node_id.as_str()) {
            let (start, end) = if is_vertical {
                (bounds.y, bounds.y + bounds.height)
            } else {
                (bounds.x, bounds.x + bounds.width)
            };
            rank_to_actual_bounds
                .entry(rank)
                .and_modify(|(s, e)| {
                    *s = (*s).min(start);
                    *e = (*e).max(end);
                })
                .or_insert((start, end));
        }
    }

    let max_rank = node_ranks.values().copied().max().unwrap_or(0).max(0) as usize;

    (0..=max_rank)
        .map(|rank| {
            let rank_i32 = rank as i32;
            if let Some(&(start, _end)) = rank_to_actual_bounds.get(&rank_i32) {
                start
            } else {
                // Interpolate between nearest real-node ranks
                let lower = (0..rank_i32)
                    .rev()
                    .find_map(|r| rank_to_actual_bounds.get(&r).map(|&(_, end)| (r, end)));
                let upper = ((rank_i32 + 1)..=(max_rank as i32))
                    .find_map(|r| rank_to_actual_bounds.get(&r).map(|&(start, _)| (r, start)));

                match (lower, upper) {
                    (Some((lower_rank, lower_end)), Some((upper_rank, upper_start))) => {
                        let rank_span = upper_rank - lower_rank;
                        let rank_offset = rank_i32 - lower_rank;
                        let pos_span = upper_start as i32 - lower_end as i32;
                        (lower_end as i32 + (pos_span * rank_offset) / rank_span) as usize
                    }
                    (Some((_, lower_end)), None) => lower_end,
                    (None, Some((_, upper_start))) => upper_start,
                    (None, None) => 0,
                }
            }
        })
        .collect()
}
