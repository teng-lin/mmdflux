//! Shared text-output infrastructure reused across graph and diagram renderers.

pub(crate) mod canvas;
pub(crate) mod chars;
pub(crate) mod connections;

use std::collections::HashMap;

pub use canvas::Canvas;
pub use chars::CharSet;

use crate::graph::grid::{NodeBounds, RoutedEdge, SubgraphBounds};

/// Allocation limits applied before creating or growing a text canvas.
#[derive(Debug, Clone, Copy)]
pub struct TextRenderLimits {
    pub max_width: usize,
    pub max_height: usize,
    pub max_cells: usize,
}

impl TextRenderLimits {
    pub(crate) fn check(self, width: usize, height: usize) -> Result<(), String> {
        if width > self.max_width || height > self.max_height {
            return Err(format!(
                "text canvas {width}x{height} exceeds dimension limit {}x{}",
                self.max_width, self.max_height
            ));
        }
        if width
            .checked_mul(height)
            .is_none_or(|cells| cells > self.max_cells)
        {
            return Err(format!(
                "text canvas {width}x{height} exceeds {} cell limit",
                self.max_cells
            ));
        }
        Ok(())
    }
}

/// Text emission together with positions in the *serialized* terminal canvas.
#[derive(Debug, Clone, Default)]
pub struct TextRenderCapture {
    pub text: String,
    pub width: usize,
    pub height: usize,
    /// Top-left draw-coordinate cell removed by final whitespace cropping.
    #[allow(dead_code)] // Retained for independent geometry assertions and debugging.
    pub origin: (usize, usize),
    pub node_bounds: HashMap<String, NodeBounds>,
    pub subgraph_bounds: HashMap<String, SubgraphBounds>,
    pub routed_edge_indices: Vec<usize>,
    /// Actual painted route geometry translated by the same emitted origin.
    #[allow(dead_code)] // Used by geometry audits; adapters currently need node bounds.
    pub routes: Vec<RoutedEdge>,
    pub diagnostics: Vec<String>,
}
