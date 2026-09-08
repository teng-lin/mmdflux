//! Keep disconnected compounds from overlapping after grid quantization.
//!
//! Layout engines can separate float rectangles while expanded terminal labels
//! and borders overlap. Pack only independent components, preserving all local
//! geometry, edge connectivity and compound membership.
use std::collections::{BTreeMap, HashMap};

use crate::graph::Graph;
use crate::graph::grid::GridLayout;

fn root(parents: &mut [usize], index: usize) -> usize {
    let mut cursor = index;
    while parents[cursor] != cursor {
        cursor = parents[cursor];
    }
    let result = cursor;
    let mut cursor = index;
    while parents[cursor] != cursor {
        let next = parents[cursor];
        parents[cursor] = result;
        cursor = next;
    }
    result
}

fn join(parents: &mut [usize], a: usize, b: usize) {
    let a = root(parents, a);
    let b = root(parents, b);
    parents[a.max(b)] = a.min(b);
}

#[derive(Clone, Copy)]
struct Bounds {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}
impl Bounds {
    fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            left: x,
            top: y,
            right: x.saturating_add(width),
            bottom: y.saturating_add(height),
        }
    }
    fn include(&mut self, other: Self) {
        self.left = self.left.min(other.left);
        self.top = self.top.min(other.top);
        self.right = self.right.max(other.right);
        self.bottom = self.bottom.max(other.bottom);
    }
}

pub(crate) fn pack_disconnected_components(graph: &Graph, layout: &mut GridLayout) -> bool {
    let mut nodes: Vec<_> = graph.nodes.keys().cloned().collect();
    nodes.sort();
    let mut groups: Vec<_> = graph.subgraphs.keys().cloned().collect();
    groups.sort();
    let node_index: HashMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i))
        .collect();
    let group_index: HashMap<_, _> = groups
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i + nodes.len()))
        .collect();
    let mut parents: Vec<_> = (0..nodes.len() + groups.len()).collect();
    for node in graph.nodes.values() {
        if let Some(parent) = node.parent.as_ref().and_then(|id| group_index.get(id)) {
            join(&mut parents, node_index[&node.id], *parent);
        }
    }
    for group in graph.subgraphs.values() {
        if let Some(parent) = group.parent.as_ref().and_then(|id| group_index.get(id)) {
            join(&mut parents, group_index[&group.id], *parent);
        }
    }
    for edge in &graph.edges {
        if let (Some(from), Some(to)) = (node_index.get(&edge.from), node_index.get(&edge.to)) {
            join(&mut parents, *from, *to);
        }
    }
    for index in 0..parents.len() {
        root(&mut parents, index);
    }
    let mut bounds = BTreeMap::<usize, Bounds>::new();
    let mut include = |component: usize, rect: Bounds| {
        bounds
            .entry(component)
            .and_modify(|bounds| bounds.include(rect))
            .or_insert(rect);
    };
    for (id, node) in &layout.node_bounds {
        if let Some(index) = node_index.get(id) {
            include(
                parents[*index],
                Bounds::new(node.x, node.y, node.width, node.height),
            );
        }
    }
    for (id, group) in &layout.subgraph_bounds {
        if let Some(index) = group_index.get(id) {
            include(
                parents[*index],
                Bounds::new(group.x, group.y, group.width, group.height),
            );
        }
    }
    for paths in [&layout.edge_waypoints, &layout.routed_edge_paths] {
        for (index, path) in paths {
            if let Some(component) = graph
                .edges
                .get(*index)
                .and_then(|edge| node_index.get(&edge.from))
                .map(|index| parents[*index])
            {
                for &(x, y) in path {
                    include(component, Bounds::new(x, y, 1, 1));
                }
            }
        }
    }
    for edge in &layout.self_edges {
        if let Some(index) = node_index.get(&edge.node_id) {
            for &(x, y) in &edge.points {
                include(parents[*index], Bounds::new(x, y, 1, 1));
            }
        }
    }
    if bounds.len() < 2 {
        return false;
    }
    // Preserve requested cell gaps when packing as well. Four cells remain a
    // floor for late marker/title extents, not an override of larger spacing.
    let gutter_x = layout.h_spacing.max(4);
    let gutter_y = layout.v_spacing.max(4);
    let components: Vec<_> = bounds.iter().map(|(&id, &rect)| (id, rect)).collect();
    let overlaps = components.iter().enumerate().any(|(i, (_, a))| {
        components.iter().skip(i + 1).any(|(_, b)| {
            a.left < b.right.saturating_add(gutter_x)
                && b.left < a.right.saturating_add(gutter_x)
                && a.top < b.bottom.saturating_add(gutter_y)
                && b.top < a.bottom.saturating_add(gutter_y)
        })
    });
    if !overlaps {
        return false;
    }
    let mut components = components;
    components.sort_by_key(|(id, bounds)| (bounds.top, bounds.left, *id));
    let mut offsets = HashMap::new();
    let mut y = 0usize;
    let mut width = 0usize;
    for (id, bounds) in components {
        offsets.insert(id, (-(bounds.left as i128), y as i128 - bounds.top as i128));
        width = width.max(bounds.right.saturating_sub(bounds.left));
        y = y
            .saturating_add(bounds.bottom.saturating_sub(bounds.top))
            .saturating_add(gutter_y);
    }
    let shift = |value: usize, delta: i128| -> usize {
        (value as i128 + delta).clamp(0, usize::MAX as i128) as usize
    };
    let offset = |index: usize| offsets.get(&parents[index]).copied().unwrap_or_default();
    for (id, bounds) in &mut layout.node_bounds {
        if let Some(index) = node_index.get(id) {
            let (dx, dy) = offset(*index);
            bounds.x = shift(bounds.x, dx);
            bounds.y = shift(bounds.y, dy);
            bounds.layout_center_x = bounds.layout_center_x.map(|x| shift(x, dx));
            bounds.layout_center_y = bounds.layout_center_y.map(|y| shift(y, dy));
        }
    }
    for (id, (x, y)) in &mut layout.draw_positions {
        if let Some(index) = node_index.get(id) {
            let (dx, dy) = offset(*index);
            *x = shift(*x, dx);
            *y = shift(*y, dy);
        }
    }
    for (id, bounds) in &mut layout.subgraph_bounds {
        if let Some(index) = group_index.get(id) {
            let (dx, dy) = offset(*index);
            bounds.x = shift(bounds.x, dx);
            bounds.y = shift(bounds.y, dy);
        }
    }
    for paths in [&mut layout.edge_waypoints, &mut layout.routed_edge_paths] {
        for (index, path) in paths {
            if let Some(index) = graph
                .edges
                .get(*index)
                .and_then(|edge| node_index.get(&edge.from))
            {
                let (dx, dy) = offset(*index);
                for (x, y) in path {
                    *x = shift(*x, dx);
                    *y = shift(*y, dy);
                }
            }
        }
    }
    for edge in &mut layout.self_edges {
        if let Some(index) = node_index.get(&edge.node_id) {
            let (dx, dy) = offset(*index);
            for (x, y) in &mut edge.points {
                *x = shift(*x, dx);
                *y = shift(*y, dy);
            }
        }
    }
    layout.width = width.saturating_add(gutter_x);
    layout.height = y;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::grid::NodeBounds;
    use crate::graph::{Direction, Edge, Node};
    fn bounds(x: usize) -> NodeBounds {
        NodeBounds {
            x,
            y: 0,
            width: 10,
            height: 4,
            layout_center_x: None,
            layout_center_y: None,
        }
    }
    #[test]
    fn packs_overlapping_components_without_changing_connected_geometry() {
        let mut graph = Graph::new(Direction::TopDown);
        graph.add_node(Node::new("A"));
        graph.add_node(Node::new("B"));
        let mut layout = GridLayout::default();
        layout.node_bounds.insert("A".into(), bounds(0));
        layout.node_bounds.insert("B".into(), bounds(5));
        layout.draw_positions.insert("A".into(), (0, 0));
        layout.draw_positions.insert("B".into(), (5, 0));
        assert!(pack_disconnected_components(&graph, &mut layout));
        let a = layout.node_bounds["A"];
        let b = layout.node_bounds["B"];
        assert!(a.y + a.height + 4 <= b.y || b.y + b.height + 4 <= a.y);
        assert_eq!(layout.draw_positions["A"], (a.x, a.y));
        assert_eq!(layout.draw_positions["B"], (b.x, b.y));
        assert!(!pack_disconnected_components(&graph, &mut layout));
        graph.add_edge(Edge::new("A", "B"));
        layout.node_bounds.insert("A".into(), bounds(0));
        layout.node_bounds.insert("B".into(), bounds(5));
        assert!(!pack_disconnected_components(&graph, &mut layout));
        assert_eq!(layout.node_bounds["B"].x, 5);
    }
}
