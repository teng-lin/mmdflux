//! Shared graph-family derived grid geometry contracts.
//!
//! These types sit between engine-owned float-space solves and downstream
//! grid-space replay. They are graph-owned so callers can derive and hydrate
//! grid geometry without depending on engine-private enums or render-owned
//! namespaces.

mod attachments;
mod backward;
mod bounds;
mod derive;
mod intersect;
pub(crate) mod label_placement;
mod layout;
mod routing;

pub use derive::geometry_to_grid_layout_with_routed;
#[cfg(test)]
pub(crate) use intersect::{NodeFace, classify_face, face_extent, face_fixed_coord};
#[cfg(test)]
pub(crate) use layout::SelfEdgeDrawData;
pub use layout::{GridLayout, GridPos, NodeBounds, SubgraphBounds};
#[cfg(test)]
pub(crate) use routing::route_edge;
pub use routing::{
    AttachDirection, Point, RoutedEdge, Segment, repair_and_validate_routes, route_all_edges,
};
#[cfg(test)]
pub(crate) use routing::{
    AttachmentOverride, TextPathFamily, compute_attachment_plan, route_edge_with_probe,
};

pub use crate::graph::projection::OverrideSubgraphProjection;

/// Grid-layout rank assignment strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridRanker {
    #[default]
    NetworkSimplex,
    LongestPath,
}

/// Layout-unit spacing carried into terminal-cell projection.
///
/// Five layout units request one cell (rounded up). Existing label/port
/// clearances remain lower bounds. Edge spacing still controls dummy-edge
/// lanes in the solver; it is not a promise to separate shared edge trunks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridLayoutSpacing {
    pub node_sep: f64,
    pub rank_sep: f64,
    pub edge_sep: f64,
}

impl GridLayoutSpacing {
    pub fn apply(self, direction: crate::graph::Direction, config: &mut GridLayoutConfig) {
        config.layout_spacing = Some(self);
        // Keep the reference float-to-grid scale fixed. Using the requested
        // separation as its denominator would normalize away the solver's
        // spacing changes. Apply the request to actual cell-gap lower bounds.
        let node_gap = (self.node_sep / 5.0).ceil() as usize;
        let rank_gap = (self.rank_sep / 5.0).ceil() as usize;
        let (horizontal, vertical) = match direction {
            crate::graph::Direction::TopDown | crate::graph::Direction::BottomTop => {
                (node_gap, rank_gap)
            }
            crate::graph::Direction::LeftRight | crate::graph::Direction::RightLeft => {
                (rank_gap, node_gap)
            }
        };
        config.h_spacing = config.h_spacing.max(horizontal);
        config.v_spacing = config.v_spacing.max(vertical);
    }
}

/// Configuration for derived grid layout computation.
///
/// Controls integer-grid spacing, padding, and the underlying layout-derived
/// parameters used by the grid replay pipeline.
#[derive(Debug, Clone)]
pub struct GridLayoutConfig {
    /// Direction-independent terminal spacing, also applied to local sublayouts.
    pub layout_spacing: Option<GridLayoutSpacing>,
    /// Horizontal spacing between nodes.
    pub h_spacing: usize,
    /// Vertical spacing between nodes.
    pub v_spacing: usize,
    /// Padding around the entire diagram.
    pub padding: usize,
    /// Extra left margin for edge labels on left branches.
    pub left_label_margin: usize,
    /// Extra right margin for edge labels on right branches.
    pub right_label_margin: usize,
    /// Ranking algorithm override.
    pub ranker: Option<GridRanker>,
    /// Node spacing (nodesep).
    pub node_sep: f64,
    /// Edge segment spacing (edgesep).
    pub edge_sep: f64,
    /// Rank spacing (ranksep).
    pub rank_sep: f64,
    /// Layout margin (applied in translateGraph).
    pub margin: f64,
    /// Additional ranksep applied when subgraphs are present (Mermaid clusters).
    pub cluster_rank_sep: f64,
    /// Honor hydrated projection ranks when replaying a marked materialized view.
    pub use_pinned_ranks: bool,
    /// Reserve horizontal edge-label clearance at its endpoints instead of
    /// applying the widest label as a global rank gap. Terminal opt-in only.
    pub local_label_spacing: bool,
}

impl Default for GridLayoutConfig {
    fn default() -> Self {
        Self {
            layout_spacing: None,
            h_spacing: 4,
            v_spacing: 3,
            padding: 1,
            left_label_margin: 0,
            right_label_margin: 0,
            ranker: None,
            node_sep: 50.0,
            edge_sep: 20.0,
            rank_sep: 50.0,
            margin: 8.0,
            cluster_rank_sep: 25.0,
            use_pinned_ranks: false,
            local_label_spacing: false,
        }
    }
}
