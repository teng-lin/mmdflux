//! Bounded validation/repair used by the structured terminal renderer.
//!
//! Legacy text snapshots keep their routing policy; interactive captures must
//! disclose missing geometry and may reroute a blocked path before painting it.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

use super::{AttachDirection, Point, RoutedEdge, Segment, segment_collides_with_nodes};
use crate::graph::grid::GridLayout;
use crate::graph::{Edge, Stroke};

/// Repair paths that cross unrelated nodes, preserving route and edge identity.
/// Work is bounded across the entire capture, not separately per edge.
pub fn repair_and_validate_routes(
    routes: &mut [RoutedEdge],
    edges: &[Edge],
    layout: &GridLayout,
    width: usize,
    height: usize,
) -> Vec<String> {
    let mut diagnostics = Vec::new();
    let mut seen = HashSet::new();
    let mut work_left = 2_000_000usize;
    for route in routes.iter_mut() {
        if !seen.insert(route.edge.index) {
            diagnostics.push(format!(
                "edge {} has duplicate drawable routes",
                route.edge.index
            ));
        }
        let original_ports = (route.start, route.end);
        let changed_ports = normalize_ports(route, layout);
        let needs_repair = changed_ports
            || route
                .segments
                .iter()
                .any(|s| collides_reserved(s, layout, &route.edge));
        if needs_repair
            && !repair(route, layout, width, height, &mut work_left)
            && !repair_alternate_ports(route, layout, width, height, &mut work_left)
        {
            tracing::debug!(route = ?route, obstacles = ?layout.node_bounds.iter().filter(|(_, bounds)| {
                    bounds.contains(route.start.x, route.start.y) || bounds.contains(route.end.x, route.end.y)
                }).collect::<Vec<_>>(), "bounded text route repair failed");
            diagnostics.push(format!(
                "edge {} crosses reserved node geometry; bounded repair unavailable",
                route.edge.index
            ));
            (route.start, route.end) = original_ports;
        }
        if route.segments.is_empty()
            || route
                .segments
                .windows(2)
                .any(|s| s[0].end_point() != s[1].start_point())
        {
            tracing::debug!(route = ?route, "incomplete drawable route geometry");
            diagnostics.push(format!(
                "edge {} has incomplete drawable route geometry",
                route.edge.index
            ));
        }
    }
    for edge in edges.iter().filter(|edge| edge.stroke != Stroke::Invisible) {
        if !seen.contains(&edge.index) {
            diagnostics.push(format!("edge {} has no drawable route", edge.index));
        }
    }
    diagnostics
}

fn repair_alternate_ports(
    route: &mut RoutedEdge,
    layout: &GridLayout,
    width: usize,
    height: usize,
    work_left: &mut usize,
) -> bool {
    use AttachDirection::*;
    let faces = [None, Some(Left), Some(Right), Some(Top), Some(Bottom)];
    for source in faces {
        for target in faces {
            if source.is_none() && target.is_none() {
                continue;
            }
            let mut candidate = route.clone();
            if let Some(face) = source {
                let Some(bounds) = layout
                    .node_bounds
                    .get(&candidate.edge.from)
                    .filter(|_| candidate.edge.from_subgraph.is_none())
                else {
                    continue;
                };
                candidate.start = outside_port(bounds, face, candidate.start);
                candidate.source_connection = Some(match face {
                    Left => Right,
                    Right => Left,
                    Top => Bottom,
                    Bottom => Top,
                });
            }
            if let Some(face) = target {
                let Some(bounds) = layout
                    .node_bounds
                    .get(&candidate.edge.to)
                    .filter(|_| candidate.edge.to_subgraph.is_none())
                else {
                    continue;
                };
                candidate.end = outside_port(bounds, face, candidate.end);
                candidate.entry_direction = face;
            }
            if repair(&mut candidate, layout, width, height, work_left) {
                *route = candidate;
                return true;
            }
        }
    }
    false
}

fn collides_reserved(segment: &Segment, layout: &GridLayout, edge: &Edge) -> bool {
    segment_collides_with_nodes(segment, layout, edge)
        || layout.subgraph_bounds.values().any(|bounds| {
            let Some((left, end)) = bounds.title_span() else {
                return false;
            };
            match segment {
                Segment::Horizontal { y, x_start, x_end } => {
                    *y == bounds.y && (*x_start).min(*x_end) < end && (*x_start).max(*x_end) >= left
                }
                Segment::Vertical { x, y_start, y_end } => {
                    (left..end).contains(x)
                        && (*y_start).min(*y_end) <= bounds.y
                        && (*y_start).max(*y_end) >= bounds.y
                }
            }
        })
}

fn normalize_ports(route: &mut RoutedEdge, layout: &GridLayout) -> bool {
    use AttachDirection::*;
    let mut changed = false;
    if route.edge.from_subgraph.is_none()
        && let Some(bounds) = layout.node_bounds.get(&route.edge.from)
        && bounds.contains(route.start.x, route.start.y)
    {
        let face = match route.source_connection.unwrap_or(Top) {
            Left => Right,
            Right => Left,
            Top => Bottom,
            Bottom => Top,
        };
        route.start = outside_port(bounds, face, route.start);
        changed = true;
    }
    if route.edge.to_subgraph.is_none()
        && let Some(bounds) = layout.node_bounds.get(&route.edge.to)
        && bounds.contains(route.end.x, route.end.y)
    {
        route.end = outside_port(bounds, route.entry_direction, route.end);
        changed = true;
    }
    changed
}

fn outside_port(
    bounds: &crate::graph::grid::NodeBounds,
    face: AttachDirection,
    old: Point,
) -> Point {
    let x = old.x.clamp(
        bounds.x.saturating_add(1),
        bounds
            .x
            .saturating_add(bounds.width.saturating_sub(2))
            .max(bounds.x.saturating_add(1)),
    );
    let y = old.y.clamp(
        bounds.y.saturating_add(1),
        bounds
            .y
            .saturating_add(bounds.height.saturating_sub(2))
            .max(bounds.y.saturating_add(1)),
    );
    match face {
        AttachDirection::Left => Point::new(bounds.x.saturating_sub(1), y),
        AttachDirection::Right => Point::new(bounds.x.saturating_add(bounds.width), y),
        AttachDirection::Top => Point::new(x, bounds.y.saturating_sub(1)),
        AttachDirection::Bottom => Point::new(x, bounds.y.saturating_add(bounds.height)),
    }
}

fn step(point: Point, direction: AttachDirection) -> Option<Point> {
    Some(match direction {
        AttachDirection::Left => Point::new(point.x.checked_sub(1)?, point.y),
        AttachDirection::Right => Point::new(point.x.checked_add(1)?, point.y),
        AttachDirection::Top => Point::new(point.x, point.y.checked_sub(1)?),
        AttachDirection::Bottom => Point::new(point.x, point.y.checked_add(1)?),
    })
}

fn segment(a: Point, b: Point) -> Segment {
    if a.x == b.x {
        Segment::Vertical {
            x: a.x,
            y_start: a.y,
            y_end: b.y,
        }
    } else {
        Segment::Horizontal {
            y: a.y,
            x_start: a.x,
            x_end: b.x,
        }
    }
}

fn repair(
    route: &mut RoutedEdge,
    layout: &GridLayout,
    width: usize,
    height: usize,
    work_left: &mut usize,
) -> bool {
    let width = width.saturating_add(2);
    let height = height.saturating_add(2);
    let Some(cells) = width
        .checked_mul(height)
        .filter(|&n| n <= 4_000_000 && n > 0)
    else {
        return false;
    };
    if *work_left == 0 {
        return false;
    }
    let Some(first) = route.segments.iter().find(|s| s.length() > 0) else {
        return false;
    };
    let Some(last) = route.segments.iter().rfind(|s| s.length() > 0) else {
        return false;
    };
    // Preserve the launch and arrival directions, so markers still point at
    // their declared ports rather than at an arbitrary repaired approach.
    let outgoing = route.source_connection.map(|direction| match direction {
        AttachDirection::Left => AttachDirection::Right,
        AttachDirection::Right => AttachDirection::Left,
        AttachDirection::Top => AttachDirection::Bottom,
        AttachDirection::Bottom => AttachDirection::Top,
    });
    let Some(start) = outgoing
        .map(|direction| step(route.start, direction))
        .unwrap_or_else(|| Some(first.point_at_offset(1)))
    else {
        return false;
    };
    let Some(goal) = step(route.end, route.entry_direction)
        .or_else(|| Some(last.point_at_offset(last.length().saturating_sub(1))))
    else {
        return false;
    };
    if [start, goal, route.start, route.end]
        .iter()
        .any(|p| p.x >= width || p.y >= height)
    {
        return false;
    }
    let prefix = segment(route.start, start);
    let suffix = segment(goal, route.end);
    if collides_reserved(&prefix, layout, &route.edge)
        || collides_reserved(&suffix, layout, &route.edge)
    {
        return false;
    }

    let mut blocked = vec![false; cells];
    for bounds in layout.node_bounds.values() {
        for y in bounds.y..bounds.y.saturating_add(bounds.height).min(height) {
            let left = bounds.x.min(width);
            let right = bounds.x.saturating_add(bounds.width).min(width);
            blocked[y * width + left..y * width + right].fill(true);
        }
    }
    for bounds in layout.subgraph_bounds.values() {
        if let Some((left, end)) = bounds.title_span()
            && bounds.y < height
        {
            blocked[bounds.y * width + left.min(width)..bounds.y * width + end.min(width)]
                .fill(true);
        }
    }
    let start_id = start.y * width + start.x;
    let goal_id = goal.y * width + goal.x;
    if blocked[start_id] || blocked[goal_id] {
        tracing::debug!(
            ?start,
            ?goal,
            start_blocked = blocked[start_id],
            goal_blocked = blocked[goal_id],
            "terminal support cells blocked"
        );
        return false;
    }
    let mut distance = vec![u32::MAX; cells];
    let mut parent = vec![u32::MAX; cells];
    let mut heap = BinaryHeap::new();
    distance[start_id] = 0;
    heap.push(Reverse((
        start.x.abs_diff(goal.x) + start.y.abs_diff(goal.y),
        Reverse(0u32),
        start_id,
    )));
    let mut local_work = 200_000usize;
    // For equal A* estimates prefer progress toward the goal. Preferring the
    // smallest g-cost floods an entire open rectangle before reaching a far
    // endpoint, exhausting the budget when node spacing widens a dense graph.
    while let Some(Reverse((_, Reverse(cost), id))) = heap.pop() {
        if cost != distance[id] {
            continue;
        }
        if id == goal_id {
            let mut points = vec![goal];
            let mut cursor = goal_id;
            while cursor != start_id {
                cursor = parent[cursor] as usize;
                points.push(Point::new(cursor % width, cursor / width));
            }
            points.reverse();
            let mut segments = vec![prefix];
            let mut previous = start;
            let mut bend = start;
            let mut direction = None;
            for point in points.into_iter().skip(1) {
                let next_direction = (
                    point.x as isize - previous.x as isize,
                    point.y as isize - previous.y as isize,
                );
                if direction.is_some_and(|d| d != next_direction) {
                    segments.push(segment(bend, previous));
                    bend = previous;
                }
                direction = Some(next_direction);
                previous = point;
            }
            if bend != goal {
                segments.push(segment(bend, goal));
            }
            segments.push(suffix);
            segments.retain(|s| s.length() > 0);
            if segments
                .iter()
                .any(|s| collides_reserved(s, layout, &route.edge))
            {
                return false;
            }
            route.segments = segments;
            return true;
        }
        if *work_left == 0 || local_work == 0 {
            return false;
        }
        *work_left -= 1;
        local_work -= 1;
        let x = id % width;
        let y = id / width;
        for next in [
            x.checked_sub(1).map(|x| y * width + x),
            (x + 1 < width).then_some(id + 1),
            y.checked_sub(1).map(|y| y * width + x),
            (y + 1 < height).then_some(id + width),
        ]
        .into_iter()
        .flatten()
        {
            if blocked[next] || cost.saturating_add(1) >= distance[next] {
                continue;
            }
            let next_cost = cost + 1;
            distance[next] = next_cost;
            parent[next] = id as u32;
            let heuristic = (next % width).abs_diff(goal.x) + (next / width).abs_diff(goal.y);
            heap.push(Reverse((
                next_cost as usize + heuristic,
                Reverse(next_cost),
                next,
            )));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::grid::{AttachDirection, NodeBounds};

    #[test]
    fn long_open_route_does_not_flood_the_whole_canvas_before_reaching_goal() {
        let bounds = |x, y| NodeBounds {
            x,
            y,
            width: 5,
            height: 3,
            layout_center_x: None,
            layout_center_y: None,
        };
        let layout = GridLayout {
            width: 2200,
            height: 220,
            node_bounds: [
                ("a".into(), bounds(1, 1)),
                ("b".into(), bounds(2100, 200)),
                ("obstacle".into(), bounds(800, 1)),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let mut route = RoutedEdge {
            edge: Edge::new("a", "b"),
            start: Point::new(6, 2),
            end: Point::new(2099, 201),
            segments: vec![
                Segment::Horizontal {
                    y: 2,
                    x_start: 6,
                    x_end: 2099,
                },
                Segment::Vertical {
                    x: 2099,
                    y_start: 2,
                    y_end: 201,
                },
            ],
            source_connection: Some(AttachDirection::Left),
            entry_direction: AttachDirection::Left,
            is_backward: false,
            is_self_edge: false,
        };
        let mut budget = 5000;
        assert!(repair(&mut route, &layout, 2200, 220, &mut budget));
        assert!(budget > 0);
        assert!(route.segments.iter().all(|segment| !collides_reserved(
            segment,
            &layout,
            &route.edge
        )));
    }

    #[test]
    fn bounded_repair_avoids_node_and_preserves_identity_and_terminal_ports() {
        let bounds = |x, y| NodeBounds {
            x,
            y,
            width: 5,
            height: 3,
            layout_center_x: None,
            layout_center_y: None,
        };
        let layout = GridLayout {
            width: 32,
            height: 14,
            node_bounds: [
                ("a".into(), bounds(1, 5)),
                ("b".into(), bounds(26, 5)),
                ("obstacle".into(), bounds(13, 5)),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let mut edge = Edge::new("a", "b");
        edge.index = 7;
        let mut routes = vec![RoutedEdge {
            edge: edge.clone(),
            start: Point::new(6, 6),
            end: Point::new(25, 6),
            segments: vec![Segment::Horizontal {
                y: 6,
                x_start: 6,
                x_end: 25,
            }],
            source_connection: Some(AttachDirection::Left),
            entry_direction: AttachDirection::Left,
            is_backward: false,
            is_self_edge: false,
        }];
        let diagnostics = repair_and_validate_routes(&mut routes, &[edge], &layout, 32, 14);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(routes[0].edge.index, 7);
        assert_eq!(
            routes[0].segments.first().unwrap().start_point(),
            Point::new(6, 6)
        );
        assert_eq!(
            routes[0].segments.last().unwrap().end_point(),
            Point::new(25, 6)
        );
        assert!(
            routes[0]
                .segments
                .iter()
                .all(|s| !segment_collides_with_nodes(s, &layout, &routes[0].edge))
        );
        assert!(routes[0].segments.len() > 1);
    }

    #[test]
    fn missing_drawable_route_is_not_counted_as_success() {
        let diagnostic = repair_and_validate_routes(
            &mut [],
            &[Edge::new("a", "b")],
            &GridLayout::default(),
            0,
            0,
        );
        assert_eq!(diagnostic, ["edge 0 has no drawable route"]);
    }

    #[test]
    fn parallel_self_edges_keep_their_distinct_original_indices_and_labels() {
        use crate::graph::grid::SelfEdgeDrawData;
        let mut a = Edge::new("a", "a").with_label("first");
        a.index = 0;
        let mut b = Edge::new("a", "a").with_label("second");
        b.index = 1;
        let mut hidden = Edge::new("a", "a").with_stroke(Stroke::Invisible);
        hidden.index = 2;
        let layout = GridLayout {
            self_edges: (0..3)
                .map(|index| SelfEdgeDrawData {
                    node_id: "a".into(),
                    edge_index: index,
                    points: vec![(5, 5), (10 + index, 5), (10 + index, 8), (5, 8)],
                })
                .collect(),
            ..Default::default()
        };
        let routes = super::super::route_all_edges(
            &[a, b, hidden],
            &layout,
            crate::graph::Direction::TopDown,
        );
        assert_eq!(
            routes
                .iter()
                .map(|route| (route.edge.index, route.edge.label.as_deref()))
                .collect::<Vec<_>>(),
            vec![(0, Some("first")), (1, Some("second"))]
        );
    }
}
