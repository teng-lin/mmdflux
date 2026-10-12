use super::*;
use crate::graph::{Edge, Node, Stroke};

fn ancestors(graph: &Graph, parent: Option<&str>) -> Vec<String> {
    let mut result = Vec::new();
    let mut next = parent;
    while let Some(id) = next {
        if result.iter().any(|seen| seen == id) {
            break;
        }
        result.push(id.to_string());
        next = graph
            .subgraphs
            .get(id)
            .and_then(|group| group.parent.as_deref());
    }
    result
}

pub(crate) fn catalog(graph: &Graph) -> Vec<TextCatalogEntry> {
    let mut result: Vec<_> = graph
        .nodes
        .values()
        .map(|node| TextCatalogEntry {
            id: TextObjectId::Node(node.id.clone()),
            label: node.label.clone(),
            ancestors: ancestors(graph, node.parent.as_deref()),
        })
        .chain(
            graph
                .subgraphs
                .values()
                .filter(|group| !group.invisible)
                .map(|group| TextCatalogEntry {
                    id: TextObjectId::Subgraph(group.id.clone()),
                    label: group.title.clone(),
                    ancestors: ancestors(graph, group.parent.as_deref()),
                }),
        )
        .collect();
    result.sort_by(|a, b| a.id.cmp(&b.id));
    result
}

fn focused_nodes(graph: &Graph, focus: Option<&TextFocus>) -> Result<BTreeSet<String>, String> {
    let Some(focus) = focus else {
        return Ok(graph.nodes.keys().cloned().collect());
    };
    if !graph.nodes.contains_key(&focus.node_id) {
        return Err(format!("unknown focus node '{}'", focus.node_id));
    }
    let mut selected = BTreeSet::from([focus.node_id.clone()]);
    let mut frontier = selected.clone();
    for _ in 0..focus.hops.min(graph.nodes.len()) {
        let mut next = BTreeSet::new();
        for edge in &graph.edges {
            if focus.direction != TextTraversalDirection::Upstream
                && frontier.contains(&edge.from)
                && graph.nodes.contains_key(&edge.to)
            {
                next.insert(edge.to.clone());
            }
            if focus.direction != TextTraversalDirection::Downstream
                && frontier.contains(&edge.to)
                && graph.nodes.contains_key(&edge.from)
            {
                next.insert(edge.from.clone());
            }
        }
        next.retain(|id| !selected.contains(id));
        if next.is_empty() {
            break;
        }
        selected.extend(next.iter().cloned());
        frontier = next;
    }
    // Compiler-created note companions are semantic annotations, not extra
    // graph hops. Keep them when their target is in view.
    loop {
        let mut added = Vec::new();
        for edge in &graph.edges {
            if edge.stroke != Stroke::Invisible {
                continue;
            }
            for (target, note) in [(&edge.from, &edge.to), (&edge.to, &edge.from)] {
                if selected.contains(target)
                    && !selected.contains(note)
                    && graph
                        .nodes
                        .get(note)
                        .is_some_and(|node| node.shape == crate::graph::Shape::NoteRect)
                {
                    added.push(note.clone());
                }
            }
        }
        if added.is_empty() {
            break;
        }
        selected.extend(added);
    }
    Ok(selected)
}

pub(crate) fn project(
    graph: &Graph,
    view: &TextView,
    is_class: bool,
) -> Result<ProjectedTextGraph, String> {
    let selected = focused_nodes(graph, view.focus.as_ref())?;
    let mut collapsed = view.collapsed_subgraphs.clone();
    for id in &collapsed {
        let Some(group) = graph.subgraphs.get(id) else {
            return Err(format!("unknown subgraph '{id}'"));
        };
        if group.invisible {
            return Err(format!("subgraph '{id}' is an internal annotation group"));
        }
    }
    if view.overview {
        for group in graph.subgraphs.values().filter(|group| !group.invisible) {
            if !ancestors(graph, group.parent.as_deref()).iter().any(|id| {
                graph
                    .subgraphs
                    .get(id)
                    .is_some_and(|parent| !parent.invisible)
            }) {
                collapsed.insert(group.id.clone());
            }
        }
    }
    for id in &view.expanded_subgraphs {
        if !graph.subgraphs.contains_key(id) {
            return Err(format!("unknown expanded subgraph '{id}'"));
        }
        collapsed.remove(id);
        for ancestor in ancestors(graph, graph.subgraphs[id].parent.as_deref()) {
            collapsed.remove(&ancestor);
        }
    }
    if let Some(focus) = &view.focus {
        for ancestor in ancestors(graph, graph.nodes[&focus.node_id].parent.as_deref()) {
            collapsed.remove(&ancestor);
        }
    }
    let collapsed_snapshot = collapsed.clone();
    collapsed.retain(|id| {
        !ancestors(graph, graph.subgraphs[id].parent.as_deref())
            .iter()
            .any(|ancestor| collapsed_snapshot.contains(ancestor))
    });

    let mut memberships = BTreeMap::<String, Vec<String>>::new();
    let mut proxy_for_group = BTreeMap::new();
    let mut proxy_for_node = BTreeMap::new();
    let mut identities = BTreeMap::new();
    let mut used: BTreeSet<_> = graph
        .nodes
        .keys()
        .chain(graph.subgraphs.keys())
        .cloned()
        .collect();
    let mut serial = 0usize;
    for group_id in &collapsed {
        let members: Vec<_> = selected
            .iter()
            .filter(|id| ancestors(graph, graph.nodes[*id].parent.as_deref()).contains(group_id))
            .cloned()
            .collect();
        if members.is_empty() {
            continue;
        }
        let proxy = loop {
            let id = format!("__mmdflux_text_group_{serial}");
            serial += 1;
            if used.insert(id.clone()) {
                break id;
            }
        };
        for member in &members {
            proxy_for_node.insert(member.clone(), proxy.clone());
        }
        identities.insert(proxy.clone(), TextObjectId::Subgraph(group_id.clone()));
        memberships.insert(proxy.clone(), members);
        proxy_for_group.insert(group_id.clone(), proxy);
    }
    let mut projected = Graph::new(graph.direction);
    for id in &selected {
        if proxy_for_node.contains_key(id) {
            continue;
        }
        let mut node = graph.nodes[id].clone();
        if is_class && !view.show_class_members {
            let lines: Vec<_> = node.label.lines().collect();
            let header_len = lines
                .iter()
                .take_while(|line| line.starts_with("<<") && line.ends_with(">>"))
                .count()
                + 1;
            let separator = lines
                .iter()
                .enumerate()
                .skip(header_len)
                .find(|(_, line)| **line == Node::SEPARATOR)
                .map(|(index, _)| index)
                .unwrap_or(lines.len());
            node.label = lines[..separator].join("\n");
        }
        identities.insert(id.clone(), TextObjectId::Node(id.clone()));
        memberships.insert(id.clone(), vec![id.clone()]);
        projected.add_node(node);
    }
    for (group_id, proxy_id) in &proxy_for_group {
        let group = &graph.subgraphs[group_id];
        let mut node = Node::new(proxy_id).with_label(format!(
            "{}\n[{} nodes]",
            group.title,
            memberships[proxy_id].len()
        ));
        node.parent = group.parent.clone();
        node.style = group.style.clone();
        projected.add_node(node);
    }
    let kept_groups: BTreeSet<_> = projected
        .nodes
        .values()
        .flat_map(|node| ancestors(graph, node.parent.as_deref()))
        .collect();
    for id in &kept_groups {
        let mut group = graph.subgraphs[id].clone();
        let children: BTreeSet<_> = projected
            .nodes
            .values()
            .filter(|node| node.parent.as_ref() == Some(id))
            .map(|node| node.id.clone())
            .chain(
                kept_groups
                    .iter()
                    .filter(|child| graph.subgraphs[*child].parent.as_ref() == Some(id))
                    .cloned(),
            )
            .collect();
        group.nodes.retain(|child| children.contains(child));
        let missing: Vec<_> = children
            .into_iter()
            .filter(|child| !group.nodes.contains(child))
            .collect();
        group.nodes.extend(missing);
        group
            .concurrent_regions
            .retain(|child| kept_groups.contains(child));
        projected.subgraphs.insert(id.clone(), group);
    }
    projected.subgraph_order = graph
        .subgraph_order
        .iter()
        .filter(|id| kept_groups.contains(*id))
        .cloned()
        .collect();
    // Graph IR adapters need not provide parse order. Preserve deterministic
    // inner-first order for any missing ancestors as well.
    let mut missing: Vec<_> = kept_groups
        .iter()
        .filter(|id| !projected.subgraph_order.contains(id))
        .cloned()
        .collect();
    missing.sort_by_key(|id| {
        (
            std::cmp::Reverse(ancestors(graph, Some(id)).len()),
            id.clone(),
        )
    });
    projected.subgraph_order.extend(missing);
    projected.notes = graph
        .notes
        .iter()
        .filter(|note| selected.contains(&note.target))
        .cloned()
        .map(|mut note| {
            if let Some(proxy) = proxy_for_node.get(&note.target) {
                note.target = proxy.clone();
            }
            note
        })
        .collect();

    let mut accounting = TextAccounting {
        original_nodes: graph.nodes.len(),
        original_edges: graph.edges.len(),
        visible_nodes: selected.len() - proxy_for_node.len(),
        collapsed_nodes: proxy_for_node.len(),
        omitted_nodes: graph.nodes.len() - selected.len(),
        ..TextAccounting::default()
    };
    let mut edges = Vec::<TextEdge>::new();
    for (ordinal, original) in graph.edges.iter().enumerate() {
        let from_inside = selected.contains(&original.from);
        let to_inside = selected.contains(&original.to);
        if original.stroke == Stroke::Invisible {
            accounting.invisible_edges.push(ordinal);
            if from_inside && to_inside {
                let mut edge = remap_edge(original, &proxy_for_node, &kept_groups);
                if edge.from != edge.to {
                    edge.index = projected.edges.len();
                    edges.push(TextEdge {
                        from: identities[&edge.from].clone(),
                        to: identities[&edge.to].clone(),
                        original_edges: Vec::new(),
                        routed: false,
                    });
                    projected.add_edge(edge);
                }
            }
            continue;
        }
        if !from_inside || !to_inside {
            if from_inside || to_inside {
                accounting.boundary_edges.push(ordinal);
            } else {
                accounting.outside_edges.push(ordinal);
            }
            continue;
        }
        let mut edge = remap_edge(original, &proxy_for_node, &kept_groups);
        if edge.from == edge.to
            && (proxy_for_node.contains_key(&original.from)
                || proxy_for_node.contains_key(&original.to))
        {
            accounting.collapsed_internal_edges.push(ordinal);
            continue;
        }
        // Exact semantic equality is conservative: differing labels/markers/
        // endpoint labels/styles stay distinct rather than becoming false facts.
        let aggregate = (proxy_for_node.contains_key(&original.from)
            || proxy_for_node.contains_key(&original.to))
        .then(|| {
            projected
                .edges
                .iter()
                .position(|existing| equivalent_edges(existing, &edge))
        })
        .flatten();
        if let Some(index) = aggregate {
            edges[index].original_edges.push(ordinal);
        } else {
            edge.index = projected.edges.len();
            edges.push(TextEdge {
                from: identities[&edge.from].clone(),
                to: identities[&edge.to].clone(),
                original_edges: vec![ordinal],
                routed: false,
            });
            projected.add_edge(edge);
        }
    }
    for (edge, membership) in projected.edges.iter_mut().zip(&edges) {
        if membership.original_edges.len() > 1 {
            edge.label = Some(match edge.label.as_deref() {
                Some(label) => format!("{label} [x{}]", membership.original_edges.len()),
                None => format!("{} edges", membership.original_edges.len()),
            });
        }
    }
    Ok(ProjectedTextGraph {
        graph: projected,
        identities,
        memberships,
        edges,
        accounting,
    })
}

fn remap_edge(
    edge: &Edge,
    proxies: &BTreeMap<String, String>,
    kept_groups: &BTreeSet<String>,
) -> Edge {
    let mut edge = edge.clone();
    if let Some(proxy) = proxies.get(&edge.from) {
        edge.from = proxy.clone();
    }
    if let Some(proxy) = proxies.get(&edge.to) {
        edge.to = proxy.clone();
    }
    if edge
        .from_subgraph
        .as_ref()
        .is_some_and(|id| !kept_groups.contains(id))
    {
        edge.from_subgraph = None;
    }
    if edge
        .to_subgraph
        .as_ref()
        .is_some_and(|id| !kept_groups.contains(id))
    {
        edge.to_subgraph = None;
    }
    edge.wrapped_label_lines = None;
    edge
}

fn equivalent_edges(a: &Edge, b: &Edge) -> bool {
    a.from == b.from
        && a.to == b.to
        && a.from_subgraph == b.from_subgraph
        && a.to_subgraph == b.to_subgraph
        && a.label == b.label
        && a.stroke == b.stroke
        && a.style == b.style
        && a.arrow_start == b.arrow_start
        && a.arrow_end == b.arrow_end
        && a.head_label == b.head_label
        && a.tail_label == b.tail_label
        && a.minlen == b.minlen
}
