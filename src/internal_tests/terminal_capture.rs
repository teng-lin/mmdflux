//! Real terminal-workload regression gates; source is never regenerated.

use crate::{TextLimits, TextRequest, prepare_text_diagram, render_prepared_text};

#[test]
fn real_compact_architecture_has_complete_routes_and_labels() {
    check_compact(include_str!(
        "../../tests/fixtures/terminal/rookie-architecture.mmd"
    ));
}

#[test]
fn real_compact_classes_have_complete_routes_and_labels() {
    check_compact(include_str!(
        "../../tests/fixtures/terminal/rookie-classes.mmd"
    ));
}

#[test]
fn large_compact_flowchart_has_complete_routes_and_labels() {
    check_compact(include_str!(
        "../../tests/fixtures/terminal/flowchart-large.mmd"
    ));
}

#[test]
fn dense_compact_cyclic_capture_has_complete_routes_and_labels() {
    check_compact(include_str!(
        "../../tests/fixtures/terminal/dense-cyclic-200.mmd"
    ));
}

#[test]
fn compact_state_and_sequence_capture_preserve_existing_families() {
    for source in [
        include_str!("../../tests/fixtures/state/notes.mmd"),
        include_str!("../../tests/fixtures/state/concurrent.mmd"),
        include_str!("../../tests/fixtures/state/self_transition.mmd"),
        include_str!("../../tests/fixtures/sequence/title.mmd"),
        include_str!("../../tests/fixtures/sequence/create_participant.mmd"),
        include_str!("../../tests/fixtures/sequence/destroy_participant.mmd"),
    ] {
        check_compact(source);
    }
}

#[test]
fn real_compact_views_have_intact_geometry_at_terminal_widths() {
    for source in [
        include_str!("../../tests/fixtures/terminal/rookie-architecture.mmd"),
        include_str!("../../tests/fixtures/terminal/rookie-classes.mmd"),
        include_str!("../../tests/fixtures/terminal/flowchart-large.mmd"),
    ] {
        let prepared = prepare_text_diagram(source, &TextLimits::default()).unwrap();
        for width in [80, 120, 200] {
            let mut request = compact_request();
            request.preferred_width = Some(width);
            let result = render_prepared_text(&prepared, &request).unwrap();
            let errors: Vec<_> = result
                .diagnostics
                .iter()
                .filter(|message| !message.contains("soft target"))
                .collect();
            assert!(errors.is_empty(), "{width} columns: {errors:?}");
        }
    }
}

fn compact_request() -> TextRequest {
    let mut request = TextRequest::default();
    request.config.layout.node_sep = 8.0;
    request.config.layout.rank_sep = 15.0;
    request.config.layout.edge_sep = 8.0;
    request
}

fn check_compact(source: &str) {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            "mmdflux::graph::grid::routing::checked=debug,mmdflux::render::text::canvas=debug",
        )
        .with_test_writer()
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let prepared = prepare_text_diagram(source, &TextLimits::default()).unwrap();
    let request = compact_request();
    let result = render_prepared_text(&prepared, &request).unwrap();
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
}
