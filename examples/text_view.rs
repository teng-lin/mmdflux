//! Probe: file.mmd [width] [full|overview|focus:ID|members] [compact|N,R,E] [terminal].
use mmdflux::{TextFocus, TextLimits, TextRequest, prepare_text_diagram, render_prepared_text};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "off".into()),
        )
        .with_writer(std::io::stderr)
        .try_init();
    let args: Vec<_> = std::env::args().collect();
    let source = std::fs::read_to_string(args.get(1).ok_or("expected Mermaid file")?)?;
    let prepared = prepare_text_diagram(&source, &TextLimits::default())?;
    let mut request = TextRequest {
        preferred_width: Some(args.get(2).map_or(Ok(120), |value| value.parse())?),
        preferred_height: Some(40),
        ..TextRequest::default()
    };
    if let Some(mode) = args.get(3) {
        if mode == "overview" {
            request.view.overview = true;
        } else if mode == "members" {
            request.view.show_class_members = false;
        } else if let Some(id) = mode.strip_prefix("focus:") {
            request.view.focus = Some(TextFocus::one_hop(id));
        }
    }
    if args.get(4).is_some_and(|value| value == "compact") {
        request.config.layout.node_sep = 8.0;
        request.config.layout.rank_sep = 15.0;
        request.config.layout.edge_sep = 8.0;
    } else if let Some(values) = args.get(4) {
        let values: Vec<f64> = values
            .split(',')
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        if values.len() != 3 {
            return Err("spacing requires node,rank,edge".into());
        }
        request.config.layout.node_sep = values[0];
        request.config.layout.rank_sep = values[1];
        request.config.layout.edge_sep = values[2];
    }
    request.config.text_layout_spacing = args.get(5).is_some_and(|value| value == "terminal");
    let started = std::time::Instant::now();
    let result = render_prepared_text(&prepared, &request)?;
    eprintln!(
        "{}",
        serde_json::json!({"width":result.width,"height":result.height,
        "original_nodes":result.accounting.original_nodes,"visible_nodes":result.accounting.visible_nodes,
        "collapsed_nodes":result.accounting.collapsed_nodes,"omitted_nodes":result.accounting.omitted_nodes,
        "original_edges":result.accounting.original_edges,"rendered_edges":result.edges.len(),
        "unrouted_edges":result.edges.iter().filter(|edge|!edge.routed).count(),
        "boundary_edges":result.accounting.boundary_edges.len(),"diagnostics":result.diagnostics,
        "spacing":[result.effective.node_spacing,result.effective.rank_spacing,result.effective.edge_spacing],
        "elapsed_ms":started.elapsed().as_secs_f64()*1000.0})
    );
    println!("{}", result.text);
    Ok(())
}
