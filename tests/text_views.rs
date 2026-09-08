//! Public prepared-text integration and semantic conservation contracts.
use mmdflux::{
    OutputFormat, RenderConfig, TextDiagram, TextDiagramError, TextDirectionPolicy, TextFocus,
    TextLimits, TextObjectId, TextRequest, prepare_text_diagram, render_prepared_text,
};
use unicode_width::UnicodeWidthStr;

fn render(source: &str, request: &TextRequest) -> TextDiagram {
    let prepared = prepare_text_diagram(source, &request.limits).unwrap();
    render_prepared_text(&prepared, request).unwrap()
}

fn conserved(result: &TextDiagram) {
    let a = &result.accounting;
    assert_eq!(
        a.visible_nodes + a.collapsed_nodes + a.omitted_nodes,
        a.original_nodes
    );
    let mut edges: Vec<_> = result
        .edges
        .iter()
        .flat_map(|edge| edge.original_edges.iter().copied())
        .chain(a.collapsed_internal_edges.iter().copied())
        .chain(a.boundary_edges.iter().copied())
        .chain(a.outside_edges.iter().copied())
        .chain(a.invisible_edges.iter().copied())
        .collect();
    edges.sort();
    assert_eq!(edges, (0..a.original_edges).collect::<Vec<_>>());
}

fn extents(result: &TextDiagram) {
    assert_eq!(
        result.width,
        result
            .text
            .lines()
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0)
    );
    assert_eq!(result.height, result.text.lines().count());
    for object in &result.objects {
        assert!(
            object.rect.x + object.rect.width <= result.width,
            "{object:?} exceeds {}x{}\n{}",
            result.width,
            result.height,
            result.text
        );
        assert!(
            object.rect.y + object.rect.height <= result.height,
            "{object:?} exceeds {}x{}",
            result.width,
            result.height
        );
    }
}

const NESTED: &str = "flowchart LR\nsubgraph outer[Outer]\nsubgraph inner[Inner]\nA[Alpha]-->B[Beta]\nend\nB-->C[Gamma]\nend\nC-->D[Delta]\nD-->E[Epsilon]\nE-->F[Phi]\nA-->A\nA-.->B\nA~~~C\n";

fn terminal_request(node: f64, rank: f64, edge: f64) -> TextRequest {
    TextRequest {
        config: RenderConfig {
            text_layout_spacing: true,
            layout: mmdflux::LayoutConfig {
                node_sep: node,
                rank_sep: rank,
                edge_sep: edge,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    }
}

fn object_gap(result: &TextDiagram, a: &str, b: &str, horizontal: bool) -> usize {
    let rect = |id: &str| {
        result
            .objects
            .iter()
            .find(|object| object.id == TextObjectId::Node(id.into()))
            .unwrap()
            .rect
    };
    let a = rect(a);
    let b = rect(b);
    let (mut a, mut b) = if horizontal {
        ((a.x, a.width), (b.x, b.width))
    } else {
        ((a.y, a.height), (b.y, b.height))
    };
    if a.0 > b.0 {
        std::mem::swap(&mut a, &mut b);
    }
    b.0.saturating_sub(a.0 + a.1)
}

#[test]
fn terminal_class_layout_stays_compact_without_rewrapping_or_losing_content() {
    let source = include_str!("fixtures/terminal/rookie-classes.mmd");
    let mut request = terminal_request(4.0, 8.0, 4.0);
    request.max_candidates = 1;
    let result = render(source, &request);
    conserved(&result);
    extents(&result);
    assert_eq!(
        (
            result.accounting.visible_nodes,
            result.accounting.original_edges
        ),
        (31, 27)
    );
    assert!(
        result.width <= 400 && result.height <= 210,
        "{}x{}",
        result.width,
        result.height
    );
    assert!(result.edges.iter().all(|edge| edge.routed));
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.effective.label_wrap, None);
    for text in [
        "produces via RecordSink",
        "report_build projects",
        "legacy projection",
        "detailed projection",
        "unseal()",
        "classify",
        "CancellationHandle?",
    ] {
        assert!(result.text.contains(text), "lost {text}");
    }
    eprintln!(
        "class full, unwrapped 4/8/4: {}x{}",
        result.width, result.height
    );
}

#[test]
fn terminal_spacing_respects_local_subgraph_direction() {
    let source = include_str!("fixtures/terminal/backedges-subgraphs.mmd");
    let mut small = terminal_request(8.0, 20.0, 8.0);
    small.max_candidates = 1;
    let mut large = terminal_request(20.0, 50.0, 20.0);
    large.max_candidates = 1;
    let a = render(source, &small);
    let b = render(source, &large);
    for (from, to) in [("I1", "I2"), ("I2", "I3")] {
        let small_gap = object_gap(&a, from, to, true);
        let large_gap = object_gap(&b, from, to, true);
        assert!(
            large_gap >= 10 && large_gap > small_gap,
            "{from}->{to}: {small_gap} vs {large_gap}\n{}",
            b.text
        );
    }
    for result in [&a, &b] {
        conserved(result);
        extents(result);
        assert_eq!(result.accounting.visible_nodes, 12);
        assert_eq!(result.accounting.original_edges, 18);
        assert!(result.edges.iter().all(|edge| edge.routed));
        for label in ["retry", "replay", "incident"] {
            assert!(result.text.contains(label));
        }
    }
    for request in [&small, &large] {
        let baseline = render(source, request);
        for _ in 0..8 {
            assert_eq!(baseline.text, render(source, request).text);
        }
    }
}

#[test]
fn terminal_spacing_each_node_and_rank_control_changes_actual_cell_gaps() {
    for direction in ["TD", "BT", "LR", "RL"] {
        let source = format!("flowchart {direction}\nA[Alpha]-->B[Bravo]\nA-->C[Gamma]\n");
        let compact = render(&source, &terminal_request(8.0, 15.0, 8.0));
        let nodes = render(&source, &terminal_request(50.0, 15.0, 8.0));
        let ranks = render(&source, &terminal_request(8.0, 50.0, 8.0));
        let vertical = matches!(direction, "TD" | "BT");
        assert!(
            object_gap(&nodes, "B", "C", vertical) > object_gap(&compact, "B", "C", vertical),
            "node spacing {direction}"
        );
        assert!(
            object_gap(&ranks, "A", "B", !vertical) > object_gap(&compact, "A", "B", !vertical),
            "rank spacing {direction}"
        );
        assert!(object_gap(&nodes, "B", "C", vertical) >= 10);
        assert!(object_gap(&ranks, "A", "B", !vertical) >= 10);
        for result in [&compact, &nodes, &ranks] {
            conserved(result);
            extents(result);
            assert!(
                result.diagnostics.is_empty(),
                "{direction}: {:?}\n{}",
                result.diagnostics,
                result.text
            );
        }
    }
}

#[test]
fn terminal_spacing_rank_control_changes_collapsed_overview_geometry() {
    let source = "flowchart LR\nsubgraph one[Group one]\nA-->B\nend\nsubgraph two[Group two]\nC-->D\nend\nsubgraph three[Group three]\nE-->F\nend\nB-->C\nD-->E\n";
    for overview in [false, true] {
        let mut request = terminal_request(8.0, 15.0, 8.0);
        request.view.overview = overview;
        let compact = render(source, &request);
        request.config.layout.rank_sep = 50.0;
        let spacious = render(source, &request);
        assert!(
            spacious.width > compact.width,
            "overview={overview}: {} vs {}",
            spacious.width,
            compact.width
        );
        assert_ne!(spacious.text, compact.text);
        for result in [&compact, &spacious] {
            conserved(result);
            extents(result);
            assert!(
                result.diagnostics.is_empty(),
                "{overview}: {:?}",
                result.diagnostics
            );
        }
    }
}

#[test]
fn terminal_spacing_edge_control_changes_a_graph_with_long_edge_lanes() {
    let source = "flowchart TD\nA-->B-->C-->D\nA-->D\nB-->D\n";
    let compact = render(source, &terminal_request(8.0, 15.0, 8.0));
    let spacious = render(source, &terminal_request(8.0, 15.0, 50.0));
    assert_ne!(
        compact.text, spacious.text,
        "edge spacing must affect actual output, not only reported hints"
    );
    for result in [&compact, &spacious] {
        conserved(result);
        extents(result);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }
}

#[test]
fn terminal_spacing_real_corpus_preserves_routes_labels_and_originals() {
    for (name, source) in [
        (
            "dense-200",
            include_str!("fixtures/terminal/dense-cyclic-200.mmd"),
        ),
        (
            "architecture",
            include_str!("fixtures/terminal/rookie-architecture.mmd"),
        ),
        (
            "classes",
            include_str!("fixtures/terminal/rookie-classes.mmd"),
        ),
        (
            "grouped-200",
            include_str!("fixtures/terminal/flowchart-large.mmd"),
        ),
    ] {
        for (node, rank, edge) in [
            (4.0, 8.0, 4.0),
            (8.0, 15.0, 8.0),
            (50.0, 15.0, 8.0),
            (8.0, 50.0, 8.0),
            (8.0, 15.0, 20.0),
            (50.0, 50.0, 20.0),
        ] {
            for width in [80, 120, 200] {
                let mut request = terminal_request(node, rank, edge);
                request.preferred_width = Some(width);
                request.preferred_height = Some(40);
                let result = render(source, &request);
                conserved(&result);
                extents(&result);
                assert!(result.edges.iter().all(|edge| edge.routed));
                assert!(
                    result
                        .diagnostics
                        .iter()
                        .all(|message| message.contains("soft target")),
                    "{name} {node}/{rank}/{edge} width={width}: {:?}",
                    result.diagnostics
                );
            }
        }
    }
}

#[test]
fn prepared_defaults_preserve_ordinary_output_and_original_catalog() {
    let source = "flowchart LR\nA[Alpha]-->B[Beta]";
    let result = render(source, &TextRequest::default());
    assert_eq!(
        result.text,
        mmdflux::render_diagram(source, OutputFormat::Text, &RenderConfig::default()).unwrap()
    );
    assert!(result.edges.iter().all(|edge| edge.routed));
    conserved(&result);
    extents(&result);
    let prepared = prepare_text_diagram(NESTED, &TextLimits::default()).unwrap();
    assert_eq!(prepared.source(), NESTED);
    assert_eq!(
        prepared
            .catalog()
            .iter()
            .find(|entry| entry.id == TextObjectId::Node("A".into()))
            .unwrap()
            .ancestors,
        vec!["inner", "outer"]
    );
}

#[test]
fn nested_overview_expand_focus_conserve_every_original() {
    let prepared = prepare_text_diagram(NESTED, &TextLimits::default()).unwrap();
    let mut request = TextRequest::default();
    request.view.overview = true;
    let overview = render_prepared_text(&prepared, &request).unwrap();
    conserved(&overview);
    extents(&overview);
    assert_eq!(overview.accounting.collapsed_nodes, 3);
    assert!(
        overview
            .objects
            .iter()
            .any(|object| object.id == TextObjectId::Subgraph("outer".into())
                && object.members.len() == 3)
    );
    request.view.expanded_subgraphs.insert("outer".into());
    request.view.collapsed_subgraphs.insert("inner".into());
    let expanded = render_prepared_text(&prepared, &request).unwrap();
    conserved(&expanded);
    assert_eq!(expanded.accounting.collapsed_nodes, 2);
    request.view.focus = Some(TextFocus::one_hop("A"));
    let focused = render_prepared_text(&prepared, &request).unwrap();
    conserved(&focused);
    extents(&focused);
    assert!(
        focused
            .objects
            .iter()
            .any(|object| object.id == TextObjectId::Node("A".into()))
    );
    assert!(focused.accounting.omitted_nodes > 0);
    assert!(!focused.accounting.boundary_edges.is_empty());
    assert!(!focused.accounting.outside_edges.is_empty());
    let full = render_prepared_text(&prepared, &TextRequest::default()).unwrap();
    conserved(&full);
    assert_eq!(full.accounting.visible_nodes, 6);
}

#[test]
fn identity_collisions_and_parallel_semantics_are_preserved() {
    let source = "flowchart LR\nsubgraph group[Group]\nA-->B\nend\n__mmdflux_text_group_0[User node]\nA-->|same|Z\nB-->|same|Z\nA-.->|different|Z\nZ-->Z\nZ-->|again|Z\n";
    let mut request = TextRequest::default();
    request.view.overview = true;
    let result = render(source, &request);
    conserved(&result);
    assert!(
        result
            .objects
            .iter()
            .any(|object| object.id == TextObjectId::Node("__mmdflux_text_group_0".into()))
    );
    assert!(
        result
            .edges
            .iter()
            .any(|edge| edge.original_edges.len() == 2)
    );
    assert_eq!(
        result
            .edges
            .iter()
            .filter(|edge| edge.from == edge.to)
            .count(),
        2
    );
    assert!(
        result
            .edges
            .iter()
            .filter(|edge| edge.from == edge.to)
            .all(|edge| edge.routed)
    );
}

#[test]
fn class_member_projection_preserves_headers_originals_and_cardinalities() {
    let source = "classDiagram\nclass User {\n<<interface>>\n+String name\n+read() String\n}\nUser \"1\" --> \"*\" Account : owns\n";
    let prepared = prepare_text_diagram(source, &TextLimits::default()).unwrap();
    let full = render_prepared_text(&prepared, &TextRequest::default()).unwrap();
    let mut request = TextRequest::default();
    request.view.show_class_members = false;
    let summary = render_prepared_text(&prepared, &request).unwrap();
    conserved(&summary);
    extents(&summary);
    assert!(summary.text.contains("interface"));
    assert!(!summary.text.contains("read()"));
    assert!(summary.text.contains("owns"));
    assert!(summary.text.contains('*'));
    assert_eq!(
        full.text,
        render_prepared_text(&prepared, &TextRequest::default())
            .unwrap()
            .text
    );
    assert!(
        prepared
            .catalog()
            .iter()
            .any(|entry| entry.label.contains("read()"))
    );
}

#[test]
fn width_hints_preserve_authored_direction_unless_explicitly_allowed() {
    let mut request = TextRequest {
        preferred_width: Some(20),
        preferred_height: Some(40),
        ..TextRequest::default()
    };
    let source = "flowchart LR\nA[Long first label]-->B[Long second label]-->C[Long third label]";
    let preserved = render(source, &request);
    assert_eq!(
        preserved.effective.direction,
        Some(mmdflux::graph::Direction::LeftRight)
    );
    assert!(preserved.effective.width_overflow > 0);
    request.direction = TextDirectionPolicy::Override(mmdflux::graph::Direction::TopDown);
    let vertical = render(source, &request);
    assert_eq!(
        vertical.effective.direction,
        Some(mmdflux::graph::Direction::TopDown)
    );
    conserved(&vertical);
    extents(&vertical);
}

#[test]
fn cell_geometry_survives_padding_cjk_and_combining_emoji() {
    for source in [
        "flowchart LR\nA[中文标题]-->B[再见]",
        "flowchart LR\nA[éé]-->B[👩‍💻界]",
    ] {
        for padding in [0, 1, 8] {
            let mut request = TextRequest::default();
            request.config.padding = Some(padding);
            let result = render(source, &request);
            extents(&result);
            // Rectangle corner checks independently decode emitted rows.
            for object in &result.objects {
                let row = result.text.lines().nth(object.rect.y).unwrap();
                let mut x = 0;
                let corner = row
                    .chars()
                    .find(|ch| {
                        let current = x;
                        x += unicode_width::UnicodeWidthChar::width(*ch).unwrap_or(0);
                        current == object.rect.x
                    })
                    .unwrap();
                assert_eq!(corner, '┌', "{}", result.text);
            }
        }
    }
}

#[test]
fn sequence_has_truthful_capabilities_and_full_unwrapped_geometry() {
    let source = "sequenceDiagram\nparticipant A as 中文\nparticipant B as Service\nA->>B: request\nB-->>A: result";
    let prepared = prepare_text_diagram(source, &TextLimits::default()).unwrap();
    assert!(!prepared.capabilities().focus);
    let result = render_prepared_text(&prepared, &TextRequest::default()).unwrap();
    extents(&result);
    assert_eq!(result.objects.len(), 2);
    let mut request = TextRequest::default();
    request.view.focus = Some(TextFocus::one_hop("A"));
    assert!(matches!(
        render_prepared_text(&prepared, &request),
        Err(TextDiagramError::Unsupported(_))
    ));
}

#[test]
fn resource_limits_reject_before_render_and_request_revalidates_prepared() {
    let limits = TextLimits {
        max_source_bytes: 4,
        ..TextLimits::default()
    };
    assert!(matches!(
        prepare_text_diagram("flowchart LR\nA-->B", &limits),
        Err(TextDiagramError::Limit(_))
    ));
    let prepared = prepare_text_diagram("flowchart LR\nA-->B", &TextLimits::default()).unwrap();
    let mut request = TextRequest::default();
    request.limits.max_nodes = 1;
    assert!(matches!(
        render_prepared_text(&prepared, &request),
        Err(TextDiagramError::Limit(_))
    ));
    request = TextRequest::default();
    request.config.layout.node_sep = f64::NAN;
    assert!(matches!(
        render_prepared_text(&prepared, &request),
        Err(TextDiagramError::InvalidRequest(_))
    ));
    request = TextRequest::default();
    request.max_candidates = 0;
    assert!(matches!(
        render_prepared_text(&prepared, &request),
        Err(TextDiagramError::InvalidRequest(_))
    ));
    request = TextRequest::default();
    request.limits.max_canvas_cells = 1;
    assert!(
        render_prepared_text(&prepared, &request)
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
}

#[test]
fn state_notes_and_concurrent_regions_remain_inspectable() {
    for source in [
        include_str!("fixtures/state/notes.mmd"),
        include_str!("fixtures/state/concurrent_nested.mmd"),
    ] {
        let result = render(source, &TextRequest::default());
        conserved(&result);
        extents(&result);
        assert!(!result.text.trim().is_empty());
    }
}

#[test]
fn compact_200_node_chain_is_complete_and_budgeted() {
    let mut source = "flowchart LR\n".to_string();
    for index in 0..199 {
        source.push_str(&format!("N{index}-->N{}\n", index + 1));
    }
    let mut request = TextRequest::default();
    request.config.layout.node_sep = 8.0;
    request.config.layout.rank_sep = 15.0;
    request.config.layout.edge_sep = 8.0;
    request.preferred_width = Some(120);
    request.max_candidates = 1;
    let result = render(&source, &request);
    conserved(&result);
    extents(&result);
    assert_eq!(result.accounting.visible_nodes, 200);
    assert_eq!(result.edges.iter().filter(|edge| edge.routed).count(), 199);
    assert!(result.effective.width_overflow > 0);
}

#[test]
fn compact_dense_cyclic_200_node_graph_keeps_all_edge_identities() {
    let mut source = "flowchart LR\n".to_string();
    for index in 0..200 {
        source.push_str(&format!("N{index}-->N{}\n", (index + 1) % 200));
        if index % 5 == 0 {
            source.push_str(&format!("N{index}-.->N{}\n", (index + 163) % 200));
        }
        if index % 7 == 0 {
            source.push_str(&format!("N{index}-->N{}\n", (index + 37) % 200));
        }
    }
    let mut request = TextRequest::default();
    request.config.layout.node_sep = 8.0;
    request.config.layout.rank_sep = 15.0;
    request.config.layout.edge_sep = 8.0;
    request.preferred_width = Some(120);
    request.max_candidates = 1;
    let result = render(&source, &request);
    conserved(&result);
    extents(&result);
    eprintln!(
        "dense cyclic: {}x{}, {} edges, diagnostics {:?}",
        result.width,
        result.height,
        result.edges.len(),
        result.diagnostics
    );
    assert_eq!(result.accounting.visible_nodes, 200);
    assert_eq!(result.edges.len(), 269);
    assert!(result.edges.iter().all(|edge| edge.routed));
    assert!(
        result
            .diagnostics
            .iter()
            .all(|message| message.contains("soft target")),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn representative_real_corpus_has_complete_geometry_and_semantic_views() {
    for source in [
        include_str!("fixtures/terminal/rookie-architecture.mmd"),
        include_str!("fixtures/terminal/rookie-classes.mmd"),
        include_str!("fixtures/terminal/flowchart-large.mmd"),
    ] {
        for width in [80, 120, 200] {
            let mut request = TextRequest::default();
            request.config.layout.node_sep = 8.0;
            request.config.layout.rank_sep = 15.0;
            request.config.layout.edge_sep = 8.0;
            request.preferred_width = Some(width);
            request.preferred_height = Some(40);
            let result = render(source, &request);
            conserved(&result);
            extents(&result);
            assert!(result.edges.iter().all(|edge| edge.routed));
            assert!(
                result
                    .diagnostics
                    .iter()
                    .all(|message| message.contains("soft target")),
                "{} at {width}: {:?}",
                source.lines().next().unwrap(),
                result.diagnostics
            );
        }
    }
}

#[test]
fn cancelled_request_stops_before_render_and_does_not_poison_prepared() {
    let prepared = prepare_text_diagram("flowchart LR\nA-->B", &TextLimits::default()).unwrap();
    let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let request = TextRequest {
        cancellation: Some(token.clone()),
        ..TextRequest::default()
    };
    assert!(matches!(
        render_prepared_text(&prepared, &request),
        Err(TextDiagramError::Cancelled)
    ));
    token.store(false, std::sync::atomic::Ordering::Relaxed);
    assert!(render_prepared_text(&prepared, &request).is_ok());
}

#[test]
fn prepared_full_architecture_is_deterministic_across_parse_and_render_instances() {
    let source = include_str!("fixtures/terminal/rookie-architecture.mmd");
    // Exact resolved settings from the CLI-vs-environment regression. These
    // equal today's defaults, but stay pinned if default density evolves.
    let request = TextRequest {
        config: RenderConfig {
            layout: mmdflux::LayoutConfig {
                node_sep: 50.0,
                rank_sep: 50.0,
                edge_sep: 20.0,
                ..Default::default()
            },
            ..Default::default()
        },
        ..TextRequest::default()
    };
    let baseline = render(source, &request);
    for iteration in 0..48 {
        let result = render(source, &request);
        assert_eq!(
            result.objects, baseline.objects,
            "object geometry varies on iteration {iteration}"
        );
        assert_eq!(
            result.text, baseline.text,
            "text geometry varies on iteration {iteration}"
        );
    }
}
