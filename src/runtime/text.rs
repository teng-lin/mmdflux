//! Bounded prepared-text facade for interactive terminal consumers.

use super::config::RenderConfig;
use crate::errors::TextDiagramError;
use crate::graph::Direction;
use crate::graph::text_view::{
    TextAccounting, TextCatalogEntry, TextCellRect, TextDirectionPolicy, TextEdge, TextObjectId,
    TextView,
};
use crate::payload::Diagram;

#[derive(Debug, Clone)]
pub struct TextLimits {
    pub max_source_bytes: usize,
    pub max_label_bytes: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_subgraph_depth: usize,
    pub max_rank_expansion: usize,
    pub max_canvas_dimension: usize,
    pub max_canvas_cells: usize,
    pub max_candidates: usize,
}

impl Default for TextLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 1_048_576,
            max_label_bytes: 262_144,
            max_nodes: 1_000,
            max_edges: 4_000,
            max_subgraph_depth: 32,
            max_rank_expansion: 1_000_000,
            max_canvas_dimension: 32_768,
            max_canvas_cells: 4_000_000,
            max_candidates: 6,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextCapabilities {
    pub collapse: bool,
    pub focus: bool,
    pub direction: bool,
    pub class_members: bool,
    pub label_wrap: bool,
}

/// Immutable parsed/compiled data. Layout and view changes clone this payload.
#[derive(Debug, Clone)]
pub struct PreparedTextDiagram {
    source: String,
    diagram_type: &'static str,
    payload: Diagram,
    catalog: Vec<TextCatalogEntry>,
    capabilities: TextCapabilities,
}

impl PreparedTextDiagram {
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn diagram_type(&self) -> &'static str {
        self.diagram_type
    }
    pub fn catalog(&self) -> &[TextCatalogEntry] {
        &self.catalog
    }
    pub fn capabilities(&self) -> TextCapabilities {
        self.capabilities
    }
}

#[derive(Debug, Clone)]
pub struct TextRequest {
    pub config: RenderConfig,
    pub view: TextView,
    pub direction: TextDirectionPolicy,
    pub label_wrap: Option<usize>,
    pub preferred_width: Option<usize>,
    pub preferred_height: Option<usize>,
    pub max_candidates: usize,
    pub limits: TextLimits,
    /// Cooperative cancellation between expensive stages. This does not
    /// interrupt an already executing parser or layout solve.
    pub cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Default for TextRequest {
    fn default() -> Self {
        Self {
            config: RenderConfig::default(),
            view: TextView::default(),
            direction: TextDirectionPolicy::Preserve,
            label_wrap: None,
            preferred_width: None,
            preferred_height: None,
            max_candidates: 3,
            limits: TextLimits::default(),
            cancellation: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextObject {
    pub id: TextObjectId,
    pub label: String,
    pub rect: TextCellRect,
    /// Original node IDs represented by this node or collapsed group.
    pub members: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TextEffectiveHints {
    pub direction: Option<Direction>,
    pub label_wrap: Option<usize>,
    pub node_spacing: f64,
    pub rank_spacing: f64,
    pub edge_spacing: f64,
    pub candidates: usize,
    pub width_overflow: usize,
    pub height_overflow: usize,
}

#[derive(Debug, Clone)]
pub struct TextDiagram {
    pub text: String,
    pub width: usize,
    pub height: usize,
    pub objects: Vec<TextObject>,
    pub edges: Vec<TextEdge>,
    pub accounting: TextAccounting,
    pub effective: TextEffectiveHints,
    pub diagnostics: Vec<String>,
}

/// Parse/compile once, without running a layout or creating any canvas.
pub fn prepare_text_diagram(
    source: &str,
    limits: &TextLimits,
) -> Result<PreparedTextDiagram, TextDiagramError> {
    check_limit("source bytes", source.len(), limits.max_source_bytes)?;
    if matches!(
        crate::frontends::detect_input_frontend(source),
        Some(crate::frontends::InputFrontend::Mmds)
    ) {
        return Err(TextDiagramError::Unsupported(
            "prepared text accepts Mermaid source, not solved MMDS geometry".into(),
        ));
    }
    let registry = crate::builtins::default_registry();
    let diagram_type = registry
        .detect(source)
        .ok_or_else(|| TextDiagramError::Unsupported("unknown diagram type".into()))?;
    let instance = registry
        .create(diagram_type)
        .ok_or_else(|| TextDiagramError::Unsupported(diagram_type.into()))?;
    let payload = instance
        .parse(source)
        .map_err(|error| TextDiagramError::Parse(error.to_string()))?
        .into_payload()
        .map_err(|error| TextDiagramError::Parse(error.to_string()))?;
    validate_payload(&payload, limits)?;
    let (catalog, capabilities) = match &payload {
        Diagram::Flowchart(graph) | Diagram::State(graph) | Diagram::Class(graph) => (
            crate::graph::text_view::catalog(graph),
            TextCapabilities {
                collapse: graph.subgraphs.values().any(|group| !group.invisible),
                focus: true,
                direction: true,
                class_members: matches!(payload, Diagram::Class(_)),
                label_wrap: true,
            },
        ),
        Diagram::Sequence(sequence) => (
            sequence
                .participants
                .iter()
                .map(|participant| TextCatalogEntry {
                    id: TextObjectId::Node(participant.id.clone()),
                    label: participant.label.clone(),
                    ancestors: Vec::new(),
                })
                .collect(),
            TextCapabilities::default(),
        ),
    };
    Ok(PreparedTextDiagram {
        source: source.to_string(),
        diagram_type,
        payload,
        catalog,
        capabilities,
    })
}

/// Apply an explicitly requested semantic view, solve it in text-cell units,
/// and return the same coordinates used by final canvas emission.
///
/// Candidate limits bound work between solves, not an in-progress wall-clock
/// deadline. Call this off the UI thread. Pan/search use the returned canvas.
pub fn render_prepared_text(
    prepared: &PreparedTextDiagram,
    request: &TextRequest,
) -> Result<TextDiagram, TextDiagramError> {
    check_cancelled(request)?;
    validate_request(prepared, request)?;
    let limits = crate::render::text::TextRenderLimits {
        max_width: request.limits.max_canvas_dimension,
        max_height: request.limits.max_canvas_dimension,
        max_cells: request.limits.max_canvas_cells,
    };
    if let Diagram::Sequence(sequence) = &prepared.payload {
        let layout = crate::timeline::sequence::layout::layout(sequence);
        check_cancelled(request)?;
        let capture = crate::render::timeline::render_capture(
            &layout,
            &crate::render::text::CharSet::unicode(),
            &limits,
        )
        .map_err(TextDiagramError::Limit)?;
        let objects = sequence
            .participants
            .iter()
            .enumerate()
            .filter_map(|(index, participant)| {
                capture
                    .node_bounds
                    .get(&index.to_string())
                    .map(|bounds| TextObject {
                        id: TextObjectId::Node(participant.id.clone()),
                        label: participant.label.clone(),
                        rect: rect(bounds.x, bounds.y, bounds.width, bounds.height),
                        members: vec![participant.id.clone()],
                    })
            })
            .collect();
        let mut diagnostics = capture.diagnostics;
        append_overflow(&mut diagnostics, capture.width, capture.height, request);
        return Ok(TextDiagram {
            text: capture.text,
            width: capture.width,
            height: capture.height,
            objects,
            edges: Vec::new(),
            accounting: TextAccounting {
                original_nodes: sequence.participants.len(),
                visible_nodes: sequence.participants.len(),
                ..TextAccounting::default()
            },
            effective: effective(None, None, 1, capture.width, capture.height, request),
            diagnostics,
        });
    }
    let original = match &prepared.payload {
        Diagram::Flowchart(graph) | Diagram::Class(graph) | Diagram::State(graph) => graph,
        _ => unreachable!(),
    };
    let projection = crate::graph::text_view::project(
        original,
        &request.view,
        prepared.capabilities.class_members,
    )
    .map_err(TextDiagramError::InvalidRequest)?;
    check_cancelled(request)?;
    let candidates = candidates(original.direction, request);
    let attempts = candidates.len();
    let mut best: Option<(f64, TextDiagram)> = None;
    let mut failures = Vec::<TextDiagramError>::new();
    for (direction, wrap) in candidates {
        check_cancelled(request)?;
        let mut graph = projection.graph.clone();
        graph.direction = direction;
        if let Some(width) = wrap {
            crate::graph::text_view::wrap_graph_labels(&mut graph, width);
        }
        let mut config = request.config.clone();
        // This facade deliberately returns a plain cell canvas. Adapters own
        // styling, and ANSI bytes must never contaminate cell coordinates.
        config.text_color_mode = crate::format::TextColorMode::Plain;
        if config.show_ids {
            for node in graph.nodes.values_mut() {
                if node.label != node.id {
                    node.label = format!("{}: {}", node.id, node.label);
                }
            }
        }
        let solve = match super::graph_family::solve_graph_family_for_render(
            prepared.diagram_type,
            &mut graph,
            crate::OutputFormat::Text,
            &config,
        ) {
            Ok(result) => result,
            Err(error) => {
                failures.push(TextDiagramError::Render(error.to_string()));
                continue;
            }
        };
        check_cancelled(request)?;
        let capture = match crate::render::graph::render_text_from_geometry_capture(
            &graph,
            &solve.solve.geometry,
            solve.solve.routed.as_ref(),
            &config.text_render_options(crate::OutputFormat::Text),
            &limits,
        ) {
            Ok(capture) => capture,
            Err(error) => {
                failures.push(TextDiagramError::Limit(error));
                continue;
            }
        };
        check_cancelled(request)?;
        let mut objects = Vec::new();
        for (id, bounds) in &capture.node_bounds {
            let Some(identity) = projection.identities.get(id) else {
                continue;
            };
            let label = prepared
                .catalog
                .iter()
                .find(|entry| entry.id == *identity)
                .map(|entry| entry.label.clone())
                .unwrap_or_else(|| graph.nodes[id].label.clone());
            objects.push(TextObject {
                id: identity.clone(),
                label,
                rect: rect(bounds.x, bounds.y, bounds.width, bounds.height),
                members: projection.memberships.get(id).cloned().unwrap_or_default(),
            });
        }
        for (id, bounds) in &capture.subgraph_bounds {
            if bounds.invisible {
                continue;
            }
            objects.push(TextObject {
                id: TextObjectId::Subgraph(id.clone()),
                label: bounds.title.clone(),
                rect: rect(bounds.x, bounds.y, bounds.width, bounds.height),
                members: Vec::new(),
            });
        }
        objects.sort_by(|a, b| a.id.cmp(&b.id));
        let mut edges = projection.edges.clone();
        for (index, edge) in edges.iter_mut().enumerate() {
            edge.routed = capture.routed_edge_indices.contains(&index);
        }
        edges.retain(|edge| !edge.original_edges.is_empty());
        let mut diagnostics = capture.diagnostics;
        for edge in edges.iter().filter(|edge| !edge.routed) {
            diagnostics.push(format!(
                "unrouted original edges {:?}: {} -> {}",
                edge.original_edges,
                edge.from.as_str(),
                edge.to.as_str()
            ));
        }
        let score = score(capture.width, capture.height, diagnostics.len(), request);
        append_overflow(&mut diagnostics, capture.width, capture.height, request);
        let result = TextDiagram {
            text: capture.text,
            width: capture.width,
            height: capture.height,
            objects,
            edges,
            accounting: projection.accounting.clone(),
            effective: effective(
                Some(direction),
                wrap,
                attempts,
                capture.width,
                capture.height,
                request,
            ),
            diagnostics,
        };
        if best.as_ref().is_none_or(|(prior, _)| score < *prior) {
            best = Some((score, result));
        }
    }
    let Some((_, mut result)) = best else {
        return Err(failures
            .pop()
            .unwrap_or_else(|| TextDiagramError::Render("no candidate produced output".into())));
    };
    for failure in failures {
        result
            .diagnostics
            .push(format!("candidate rejected: {failure}"));
    }
    Ok(result)
}

fn check_cancelled(request: &TextRequest) -> Result<(), TextDiagramError> {
    if request
        .cancellation
        .as_ref()
        .is_some_and(|token| token.load(std::sync::atomic::Ordering::Relaxed))
    {
        Err(TextDiagramError::Cancelled)
    } else {
        Ok(())
    }
}

fn rect(x: usize, y: usize, width: usize, height: usize) -> TextCellRect {
    TextCellRect {
        x,
        y,
        width,
        height,
    }
}

fn check_limit(name: &str, actual: usize, maximum: usize) -> Result<(), TextDiagramError> {
    if actual > maximum {
        Err(TextDiagramError::Limit(format!(
            "{name}: {actual} exceeds {maximum}"
        )))
    } else {
        Ok(())
    }
}

fn validate_payload(payload: &Diagram, limits: &TextLimits) -> Result<(), TextDiagramError> {
    let graph = match payload {
        Diagram::Flowchart(graph) | Diagram::State(graph) | Diagram::Class(graph) => graph,
        Diagram::Sequence(sequence) => {
            check_limit(
                "participants",
                sequence.participants.len(),
                limits.max_nodes,
            )?;
            check_limit("sequence events", sequence.events.len(), limits.max_edges)?;
            use crate::timeline::sequence::model::SequenceEvent;
            let labels = sequence
                .participants
                .iter()
                .map(|participant| participant.label.len())
                .chain(sequence.title.iter().map(String::len))
                .chain(
                    sequence
                        .participant_boxes
                        .iter()
                        .filter_map(|group| group.label.as_ref().map(String::len)),
                )
                .chain(sequence.events.iter().map(|event| match event {
                    SequenceEvent::Message { text, .. } | SequenceEvent::Note { text, .. } => {
                        text.len()
                    }
                    SequenceEvent::BlockStart { label, .. }
                    | SequenceEvent::BlockDivider { label, .. } => label.len(),
                    _ => 0,
                }))
                .fold(0usize, usize::saturating_add);
            check_limit("label bytes", labels, limits.max_label_bytes)?;
            let mut depth = 0usize;
            for event in &sequence.events {
                match event {
                    SequenceEvent::BlockStart { .. } => depth += 1,
                    SequenceEvent::BlockEnd => depth = depth.saturating_sub(1),
                    _ => {}
                }
                check_limit("sequence block depth", depth, limits.max_subgraph_depth)?;
            }
            return Ok(());
        }
    };
    check_limit(
        "nodes and containers",
        graph.nodes.len().saturating_add(graph.subgraphs.len()),
        limits.max_nodes,
    )?;
    check_limit("edges", graph.edges.len(), limits.max_edges)?;
    let labels = graph
        .nodes
        .values()
        .map(|node| node.label.len())
        .chain(graph.subgraphs.values().map(|group| group.title.len()))
        .chain(graph.notes.iter().map(|note| note.text.len()))
        .chain(graph.edges.iter().flat_map(|edge| {
            [
                edge.label.as_ref(),
                edge.head_label.as_ref(),
                edge.tail_label.as_ref(),
            ]
            .into_iter()
            .flatten()
            .map(String::len)
        }))
        .fold(0usize, usize::saturating_add);
    check_limit("label bytes", labels, limits.max_label_bytes)?;
    for entry in crate::graph::text_view::catalog(graph) {
        check_limit(
            "subgraph depth",
            entry.ancestors.len(),
            limits.max_subgraph_depth,
        )?;
    }
    let minlen = graph
        .edges
        .iter()
        .map(|edge| usize::try_from(edge.minlen).unwrap_or(usize::MAX))
        .try_fold(0usize, |sum, value| sum.checked_add(value))
        .unwrap_or(usize::MAX);
    // Conservative bound on rank-normalization dummy expansion. It may reject
    // large shallow graphs: callers may explicitly raise the admission budget.
    check_limit(
        "rank expansion estimate",
        minlen.saturating_mul(graph.nodes.len().max(1)),
        limits.max_rank_expansion,
    )
}

fn validate_request(
    prepared: &PreparedTextDiagram,
    request: &TextRequest,
) -> Result<(), TextDiagramError> {
    check_limit(
        "source bytes",
        prepared.source.len(),
        request.limits.max_source_bytes,
    )?;
    validate_payload(&prepared.payload, &request.limits)?;
    for (name, value) in [
        ("node spacing", request.config.layout.node_sep),
        ("rank spacing", request.config.layout.rank_sep),
        ("edge spacing", request.config.layout.edge_sep),
    ] {
        if !value.is_finite() || !(0.0..=500.0).contains(&value) {
            return Err(TextDiagramError::InvalidRequest(format!(
                "{name} must be finite and in 0..500"
            )));
        }
    }
    let layout = &request.config.layout;
    for (name, value) in [
        ("layout margin", Some(layout.margin)),
        ("edge label spacing", Some(layout.edge_label_spacing)),
        ("cluster spacing", request.config.cluster_ranksep),
    ] {
        if value.is_some_and(|value| !value.is_finite() || !(0.0..=500.0).contains(&value)) {
            return Err(TextDiagramError::InvalidRequest(format!(
                "{name} must be finite and in 0..500"
            )));
        }
    }
    if layout
        .rank_sep_overrides
        .values()
        .any(|value| !value.is_finite() || !(0.0..=500.0).contains(value))
    {
        return Err(TextDiagramError::InvalidRequest(
            "rank spacing overrides must be finite and in 0..500".into(),
        ));
    }
    if layout
        .edge_label_max_width
        .is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 1_000_000.0)
    {
        return Err(TextDiagramError::InvalidRequest(
            "edge label maximum width must be finite and in 0..1,000,000".into(),
        ));
    }
    if layout.subgraph_title_margin.as_ref().is_some_and(|margin| {
        [margin.top, margin.bottom]
            .into_iter()
            .any(|value| !value.is_finite() || !(0.0..=500.0).contains(&value))
    }) {
        return Err(TextDiagramError::InvalidRequest(
            "subgraph title margins must be finite and in 0..500".into(),
        ));
    }
    if let Some(padding) = request.config.padding {
        check_limit("padding", padding, request.limits.max_canvas_dimension)?;
    }
    if request.max_candidates == 0 || request.max_candidates > request.limits.max_candidates {
        return Err(TextDiagramError::InvalidRequest(
            "candidate count must be positive and within the candidate limit".into(),
        ));
    }
    if request.label_wrap == Some(0)
        || request.preferred_width == Some(0)
        || request.preferred_height == Some(0)
    {
        return Err(TextDiagramError::InvalidRequest(
            "wrap and preferred dimensions must be positive".into(),
        ));
    }
    if request
        .view
        .focus
        .as_ref()
        .is_some_and(|focus| focus.hops > request.limits.max_nodes)
    {
        return Err(TextDiagramError::InvalidRequest(
            "focus hops exceeds node limit".into(),
        ));
    }
    if !prepared.capabilities.focus && request.view.focus.is_some() {
        return Err(TextDiagramError::Unsupported(
            "focus is graph-family only".into(),
        ));
    }
    if !prepared.capabilities.collapse
        && (request.view.overview
            || !request.view.collapsed_subgraphs.is_empty()
            || !request.view.expanded_subgraphs.is_empty())
    {
        return Err(TextDiagramError::Unsupported(
            "this diagram has no collapsible containers".into(),
        ));
    }
    if !prepared.capabilities.class_members && !request.view.show_class_members {
        return Err(TextDiagramError::Unsupported(
            "member compartments are class-diagram only".into(),
        ));
    }
    if !prepared.capabilities.direction && request.direction != TextDirectionPolicy::Preserve {
        return Err(TextDiagramError::Unsupported(
            "sequence direction preserves participant order".into(),
        ));
    }
    if !prepared.capabilities.label_wrap && request.label_wrap.is_some() {
        return Err(TextDiagramError::Unsupported(
            "sequence label wrapping is not supported".into(),
        ));
    }
    Ok(())
}

fn candidates(authored: Direction, request: &TextRequest) -> Vec<(Direction, Option<usize>)> {
    let direction = match request.direction {
        TextDirectionPolicy::Override(direction) => direction,
        _ => authored,
    };
    let mut directions = vec![direction];
    if request.direction == TextDirectionPolicy::AllowAlternate {
        directions.push(match direction {
            Direction::TopDown => Direction::LeftRight,
            Direction::BottomTop => Direction::RightLeft,
            Direction::LeftRight => Direction::TopDown,
            Direction::RightLeft => Direction::BottomTop,
        });
    }
    let mut wraps = vec![request.label_wrap];
    if let Some(width) = request.preferred_width {
        let wrap = Some((width / 3).clamp(8, 40));
        if !wraps.contains(&wrap) {
            wraps.push(wrap);
        }
        let wrap = Some((width / 4).clamp(8, 24));
        if !wraps.contains(&wrap) {
            wraps.push(wrap);
        }
    }
    let mut result = Vec::new();
    for wrap in wraps {
        for direction in &directions {
            result.push((*direction, wrap));
        }
    }
    result.truncate(request.max_candidates);
    result
}

fn score(width: usize, height: usize, diagnostics: usize, request: &TextRequest) -> f64 {
    let w = request.preferred_width.unwrap_or(width.max(1)) as f64;
    let h = request.preferred_height.unwrap_or(60) as f64;
    diagnostics as f64 * 1_000_000.0
        + 2.0 * ((width as f64 / w) - 1.0).max(0.0)
        + ((height as f64 / h) - 1.0).max(0.0)
        + (width as f64 / w) * (height as f64 / h) * 0.05
}

fn append_overflow(
    diagnostics: &mut Vec<String>,
    width: usize,
    height: usize,
    request: &TextRequest,
) {
    if let Some(target) = request.preferred_width.filter(|target| width > *target) {
        diagnostics.push(format!(
            "canvas is {width} columns; preferred width {target} is a soft target"
        ));
    }
    if let Some(target) = request.preferred_height.filter(|target| height > *target) {
        diagnostics.push(format!(
            "canvas is {height} rows; preferred height {target} is a soft target"
        ));
    }
}

fn effective(
    direction: Option<Direction>,
    wrap: Option<usize>,
    candidates: usize,
    width: usize,
    height: usize,
    request: &TextRequest,
) -> TextEffectiveHints {
    TextEffectiveHints {
        direction,
        label_wrap: wrap,
        node_spacing: request.config.layout.node_sep,
        rank_spacing: request.config.layout.rank_sep,
        edge_spacing: request.config.layout.edge_sep,
        candidates,
        width_overflow: request
            .preferred_width
            .map_or(0, |target| width.saturating_sub(target)),
        height_overflow: request
            .preferred_height
            .map_or(0, |target| height.saturating_sub(target)),
    }
}
