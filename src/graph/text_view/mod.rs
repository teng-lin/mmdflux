//! Semantic, freshly laid-out text views over immutable graph-family IR.
//!
//! Unlike MMDS `views`, these projections intentionally discard old geometry.
//! They preserve original identity and account for every original edge.

mod packing;
mod project;
mod wrap;

use std::collections::{BTreeMap, BTreeSet};

pub(crate) use packing::pack_disconnected_components;
pub(crate) use project::{catalog, project};
pub(crate) use wrap::wrap_graph_labels;

use super::{Direction, Graph};

/// A stable original identity. Node and container namespaces are distinct.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextObjectId {
    Node(String),
    Subgraph(String),
}

impl TextObjectId {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Node(id) | Self::Subgraph(id) => id,
        }
    }
}

/// Searchable original object, including objects hidden by the current view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCatalogEntry {
    pub id: TextObjectId,
    pub label: String,
    /// Closest parent first, followed by its ancestors.
    pub ancestors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextTraversalDirection {
    #[default]
    Both,
    Upstream,
    Downstream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFocus {
    pub node_id: String,
    pub hops: usize,
    pub direction: TextTraversalDirection,
}

impl TextFocus {
    pub fn one_hop(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            hops: 1,
            direction: TextTraversalDirection::Both,
        }
    }
}

/// Policies apply to clones: the prepared graph and its source never change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextView {
    pub overview: bool,
    pub collapsed_subgraphs: BTreeSet<String>,
    /// Exceptions to automatic overview collapse (also reveals nested objects).
    pub expanded_subgraphs: BTreeSet<String>,
    pub focus: Option<TextFocus>,
    pub show_class_members: bool,
}

impl Default for TextView {
    fn default() -> Self {
        Self {
            overview: false,
            collapsed_subgraphs: BTreeSet::new(),
            expanded_subgraphs: BTreeSet::new(),
            focus: None,
            show_class_members: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDirectionPolicy {
    #[default]
    Preserve,
    Override(Direction),
    /// Permit one candidate with the alternate root axis; preserve overrides.
    AllowAlternate,
}

/// Rectangle in the emitted terminal canvas, not proportional coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextCellRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// Semantic partition of original nodes and edges. Original edge IDs are
/// zero-based source-payload ordinals, never projected layout indices.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextAccounting {
    pub original_nodes: usize,
    pub original_edges: usize,
    pub visible_nodes: usize,
    pub collapsed_nodes: usize,
    pub omitted_nodes: usize,
    pub collapsed_internal_edges: Vec<usize>,
    pub boundary_edges: Vec<usize>,
    pub outside_edges: Vec<usize>,
    pub invisible_edges: Vec<usize>,
}

impl TextAccounting {
    pub fn hidden_nodes(&self) -> usize {
        self.collapsed_nodes + self.omitted_nodes
    }
    pub fn hidden_edges(&self) -> usize {
        self.collapsed_internal_edges.len() + self.boundary_edges.len() + self.outside_edges.len()
    }
}

/// One drawable projected edge and all original edges represented by it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdge {
    pub from: TextObjectId,
    pub to: TextObjectId,
    pub original_edges: Vec<usize>,
    /// False indicates an explicit renderer failure, not semantic omission.
    pub routed: bool,
}

pub(crate) struct ProjectedTextGraph {
    pub graph: Graph,
    pub identities: BTreeMap<String, TextObjectId>,
    pub memberships: BTreeMap<String, Vec<String>>,
    pub edges: Vec<TextEdge>,
    pub accounting: TextAccounting,
}
