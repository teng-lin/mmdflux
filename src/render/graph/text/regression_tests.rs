use std::collections::{HashMap, HashSet};

use crate::graph::geometry::{FPoint, FRect, GraphGeometry, LayoutEdge, PositionedNode};
use crate::graph::grid::{AttachDirection, GridLayout, Point, RoutedEdge, Segment};
use crate::graph::{Direction, Edge, Graph, Node, Shape};
use crate::render::graph::{TextRenderOptions, render_text_from_geometry};
use crate::render::text::TextRenderLimits;

#[test]
fn bounded_cluster_junction_preserves_arrow_markers() {
    use crate::render::text::canvas::Connections;
    use crate::render::text::{Canvas, CharSet};
    let charset = CharSet::unicode();
    let mut canvas = Canvas::new_limited(
        5,
        5,
        TextRenderLimits {
            max_width: 5,
            max_height: 5,
            max_cells: 25,
        },
    )
    .unwrap();
    canvas.set(2, 2, '►');
    canvas.expect_text(2, 2, "►");
    let connections = Connections {
        up: true,
        down: true,
        left: true,
        right: true,
    };
    super::set_junction_cell(&mut canvas, 2, 2, connections, &charset);
    assert_eq!(canvas.get(2, 2).unwrap().ch, '►');
    assert!(canvas.emission_diagnostics().is_empty());
    canvas.set(3, 2, '─');
    super::set_junction_cell(&mut canvas, 3, 2, connections, &charset);
    assert_eq!(canvas.get(3, 2).unwrap().ch, '┼');
}

#[test]
fn compacted_capture_keeps_route_cells_and_node_rectangles_in_sync() {
    use crate::graph::grid::{GridLayoutSpacing, NodeBounds};
    let mut graph = Graph::new(Direction::TopDown);
    graph.add_node(Node::new("A").with_label("Start"));
    graph.add_node(Node::new("B").with_label("End"));
    graph.add_edge(Edge::new("A", "B"));
    let bounds = |y| NodeBounds {
        x: 4,
        y,
        width: 9,
        height: 3,
        layout_center_x: None,
        layout_center_y: None,
    };
    let layout = GridLayout {
        width: 18,
        height: 110,
        h_spacing: 4,
        v_spacing: 3,
        node_bounds: HashMap::from([("A".into(), bounds(4)), ("B".into(), bounds(100))]),
        draw_positions: HashMap::from([("A".into(), (4, 4)), ("B".into(), (4, 100))]),
        ..Default::default()
    };
    let baseline = super::render_text_capture_from_grid_layout(
        &graph,
        &layout,
        None,
        &TextRenderOptions::default(),
        &TextRenderLimits {
            max_width: 100,
            max_height: 120,
            max_cells: 12000,
        },
    )
    .unwrap();
    let result = super::render_text_capture_from_grid_layout(
        &graph,
        &layout,
        None,
        &TextRenderOptions {
            layout_spacing: Some(GridLayoutSpacing {
                node_sep: 8.0,
                rank_sep: 15.0,
                edge_sep: 8.0,
            }),
            ..Default::default()
        },
        &TextRenderLimits {
            max_width: 100,
            max_height: 120,
            max_cells: 12000,
        },
    )
    .unwrap();
    assert!(result.height < 20, "{}", result.height);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let rows: Vec<_> = result.text.lines().collect();
    for (id, bounds) in &result.node_bounds {
        let original = &baseline.node_bounds[id];
        assert_eq!(
            (bounds.width, bounds.height),
            (original.width, original.height)
        );
        assert_eq!(rows[bounds.y].chars().nth(bounds.x), Some('┌'));
    }
    for route in &result.routes {
        for segment in &route.segments {
            for offset in 0..=segment.length() {
                let point = segment.point_at_offset(offset);
                assert!(point.y < result.height && point.x < result.width);
                assert_ne!(rows[point.y].chars().nth(point.x), Some(' '), "{point:?}");
            }
        }
    }
}

#[test]
fn structured_capture_cell_rectangles_match_serialized_boxes() {
    let (diagram, geometry) = smoke_graph_geometry();
    let result = crate::render::graph::render_text_from_geometry_capture(
        &diagram,
        &geometry,
        None,
        &TextRenderOptions::default(),
        &TextRenderLimits {
            max_width: 500,
            max_height: 500,
            max_cells: 250_000,
        },
    )
    .unwrap();
    let lines: Vec<_> = result.text.lines().collect();
    assert_eq!(result.height, lines.len());
    assert_eq!(
        result.width,
        lines
            .iter()
            .map(|s| unicode_width::UnicodeWidthStr::width(*s))
            .max()
            .unwrap()
    );
    assert_eq!(result.routed_edge_indices, vec![0]);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    for route in &result.routes {
        for segment in &route.segments {
            for offset in 0..=segment.length() {
                let point = segment.point_at_offset(offset);
                assert_ne!(
                    lines[point.y].chars().nth(point.x),
                    Some(' '),
                    "missing painted route cell {point:?}"
                );
            }
        }
    }
    for (id, bounds) in &result.node_bounds {
        assert_eq!(lines[bounds.y].chars().nth(bounds.x), Some('┌'), "{id}");
        assert_eq!(
            lines[bounds.y].chars().nth(bounds.x + bounds.width - 1),
            Some('┐'),
            "{id}"
        );
        assert!(lines[bounds.y + 1].contains(&diagram.nodes[id].label));
        assert!(bounds.x + bounds.width <= result.width);
        assert!(bounds.y + bounds.height <= result.height);
    }
}

#[test]
fn structured_capture_preserves_graphemes_inside_measured_node() {
    use crate::graph::grid::NodeBounds;
    let label = "e\u{301}中👩\u{200d}💻";
    let mut graph = Graph::new(Direction::TopDown);
    graph.add_node(Node::new("unicode").with_label(label));
    let bounds = NodeBounds {
        x: 5,
        y: 4,
        width: 9,
        height: 3,
        layout_center_x: None,
        layout_center_y: None,
    };
    let layout = GridLayout {
        width: 20,
        height: 10,
        draw_positions: HashMap::from([("unicode".into(), (5, 4))]),
        node_bounds: HashMap::from([("unicode".into(), bounds)]),
        ..Default::default()
    };
    let result = super::render_text_capture_from_grid_layout(
        &graph,
        &layout,
        None,
        &TextRenderOptions::default(),
        &TextRenderLimits {
            max_width: 100,
            max_height: 100,
            max_cells: 10000,
        },
    )
    .unwrap();
    assert_eq!(result.origin, (5, 4));
    assert_eq!((result.width, result.height), (9, 3));
    assert_eq!(
        (
            result.node_bounds["unicode"].x,
            result.node_bounds["unicode"].y
        ),
        (0, 0)
    );
    assert_eq!(result.text.lines().nth(1).unwrap(), format!("│ {label} │"));
    for line in result.text.lines() {
        assert_eq!(unicode_width::UnicodeWidthStr::width(line), 9);
    }
}

#[test]
fn structured_capture_rejects_canvas_limits_before_paint() {
    let (diagram, geometry) = smoke_graph_geometry();
    assert!(
        crate::render::graph::render_text_from_geometry_capture(
            &diagram,
            &geometry,
            None,
            &TextRenderOptions::default(),
            &TextRenderLimits {
                max_width: 1,
                max_height: 1,
                max_cells: 1
            },
        )
        .unwrap_err()
        .contains("limit")
    );
}

#[test]
fn text_owner_local_smoke_renders_text_output() {
    let (diagram, geometry) = smoke_graph_geometry();
    let text = render_text_from_geometry(&diagram, &geometry, None, &TextRenderOptions::default());

    assert!(text.contains("Start"));
    assert!(text.contains("End"));
}

fn smoke_graph_geometry() -> (Graph, GraphGeometry) {
    let mut diagram = Graph::new(Direction::TopDown);
    diagram.add_node(Node::new("A").with_label("Start"));
    diagram.add_node(Node::new("B").with_label("End"));
    diagram.add_edge(Edge::new("A", "B"));

    let nodes = HashMap::from([
        (
            "A".to_string(),
            PositionedNode {
                id: "A".to_string(),
                rect: FRect::new(50.0, 25.0, 40.0, 20.0),
                shape: Shape::Rectangle,
                label: "Start".to_string(),
                parent: None,
            },
        ),
        (
            "B".to_string(),
            PositionedNode {
                id: "B".to_string(),
                rect: FRect::new(50.0, 75.0, 40.0, 20.0),
                shape: Shape::Rectangle,
                label: "End".to_string(),
                parent: None,
            },
        ),
    ]);

    let geometry = GraphGeometry {
        nodes,
        edges: vec![LayoutEdge {
            index: 0,
            from: "A".to_string(),
            to: "B".to_string(),
            waypoints: vec![],
            label_position: None,
            label_side: None,
            from_subgraph: None,
            to_subgraph: None,
            layout_path_hint: Some(vec![FPoint::new(50.0, 45.0), FPoint::new(50.0, 75.0)]),
            preserve_orthogonal_topology: false,
            label_geometry: None,
            effective_wrapped_lines: None,
        }],
        subgraphs: HashMap::new(),
        self_edges: vec![],
        direction: Direction::TopDown,
        node_directions: HashMap::from([
            ("A".to_string(), Direction::TopDown),
            ("B".to_string(), Direction::TopDown),
        ]),
        bounds: FRect::new(0.0, 0.0, 100.0, 100.0),
        reversed_edges: vec![],
        engine_hints: None,
        grid_projection: None,
        rerouted_edges: HashSet::new(),
        enhanced_backward_routing: false,
    };

    (diagram, geometry)
}

#[test]
fn required_canvas_size_includes_post_route_overhang() {
    let layout = GridLayout {
        width: 21,
        height: 25,
        ..GridLayout::default()
    };
    let routed = RoutedEdge {
        edge: Edge::new("B", "C"),
        start: Point::new(17, 7),
        end: Point::new(12, 27),
        segments: vec![
            Segment::Horizontal {
                y: 7,
                x_start: 17,
                x_end: 22,
            },
            Segment::Vertical {
                x: 22,
                y_start: 7,
                y_end: 27,
            },
            Segment::Horizontal {
                y: 27,
                x_start: 22,
                x_end: 12,
            },
        ],
        source_connection: Some(AttachDirection::Left),
        entry_direction: AttachDirection::Right,
        is_backward: false,
        is_self_edge: false,
    };

    let (width, height) = super::required_canvas_size(&layout, &[routed]);
    assert_eq!((width, height), (23, 28));
}
