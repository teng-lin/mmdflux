//! Timeline-family rendering (sequence diagrams).

pub mod svg;
pub mod svg_layout;
pub mod text;

use crate::graph::measure::ProportionalTextMetrics;
use crate::render::svg::theme::ResolvedSvgTheme;
use crate::render::text::{Canvas, CharSet, TextRenderCapture, TextRenderLimits};
use crate::timeline::sequence::layout::SequenceLayout;
use crate::timeline::sequence::model::Sequence;

pub fn render(layout: &SequenceLayout, charset: &CharSet) -> String {
    text::render(layout, charset)
}

pub fn render_capture(
    layout: &SequenceLayout,
    charset: &CharSet,
    limits: &TextRenderLimits,
) -> Result<TextRenderCapture, String> {
    if layout.participants.is_empty() {
        return Ok(TextRenderCapture::default());
    }
    let mut canvas = Canvas::new_limited(layout.width, layout.height, *limits)?;
    text::paint(layout, charset, &mut canvas);
    canvas.check_limits()?;
    let (text, origin, width, height) = canvas.serialize(false);
    let node_bounds = layout
        .participants
        .iter()
        .enumerate()
        .map(|(index, p)| {
            (
                index.to_string(),
                crate::graph::grid::NodeBounds {
                    x: p.box_x.saturating_sub(origin.0),
                    y: p.box_y.saturating_sub(origin.1),
                    width: p.box_width,
                    height: 3,
                    layout_center_x: None,
                    layout_center_y: None,
                },
            )
        })
        .collect();
    Ok(TextRenderCapture {
        text,
        width,
        height,
        origin,
        node_bounds,
        diagnostics: canvas.emission_diagnostics(),
        ..Default::default()
    })
}

pub fn render_svg(
    model: &Sequence,
    metrics: &ProportionalTextMetrics,
    font_family: &str,
    theme: Option<&ResolvedSvgTheme>,
) -> String {
    let layout = svg_layout::layout(model, metrics, font_family);
    svg::render(&layout, theme)
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::timeline::sequence::layout;
    use crate::timeline::sequence::model::{Participant, ParticipantKind, Sequence};

    #[test]
    fn sequence_capture_uses_cells_for_unicode_participants_and_title() {
        let labels = ["e\u{301}", "中文", "👩\u{200d}💻"];
        let model = Sequence {
            title: Some("中文 title".into()),
            participants: labels
                .iter()
                .enumerate()
                .map(|(i, label)| Participant {
                    id: i.to_string(),
                    label: (*label).into(),
                    kind: ParticipantKind::Participant,
                })
                .collect(),
            participant_boxes: vec![],
            events: vec![],
            autonumber: Default::default(),
        };
        let layout = layout::layout(&model);
        let capture = render_capture(
            &layout,
            &CharSet::unicode(),
            &TextRenderLimits {
                max_width: 300,
                max_height: 100,
                max_cells: 30000,
            },
        )
        .unwrap();
        assert_eq!(capture.text, render(&layout, &CharSet::unicode()));
        for (index, label) in labels.iter().enumerate() {
            let bounds = &capture.node_bounds[&index.to_string()];
            assert_eq!(
                bounds.width,
                unicode_width::UnicodeWidthStr::width(*label) + 4
            );
            assert!(
                capture
                    .text
                    .lines()
                    .nth(bounds.y + 1)
                    .unwrap()
                    .contains(label)
            );
            assert!(bounds.x + bounds.width <= capture.width);
        }
        assert_eq!(
            capture.width,
            capture
                .text
                .lines()
                .map(unicode_width::UnicodeWidthStr::width)
                .max()
                .unwrap()
        );
    }

    #[test]
    fn sequence_capture_checks_limits_before_canvas_allocation() {
        let layout = SequenceLayout {
            title: None,
            participants: vec![],
            participant_boxes: vec![],
            rows: vec![],
            blocks: vec![],
            activations: vec![],
            width: usize::MAX,
            height: usize::MAX,
        };
        // Empty diagrams allocate no canvas and have a genuinely empty extent.
        assert_eq!(
            render_capture(
                &layout,
                &CharSet::unicode(),
                &TextRenderLimits {
                    max_width: 1,
                    max_height: 1,
                    max_cells: 1
                }
            )
            .unwrap()
            .width,
            0
        );
    }
}
