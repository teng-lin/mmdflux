use unicode_segmentation::UnicodeSegmentation;

use super::Graph;
use crate::format::display_width;
use crate::graph::Node;

/// Wrap on grapheme boundaries before measurement. Existing newlines and
/// compartment separators are respected; literal backslash-n stays literal.
fn wrap(label: &str, width: usize) -> String {
    label
        .split('\n')
        .map(|line| {
            if line == Node::SEPARATOR {
                return line.to_string();
            }
            let mut rows = Vec::new();
            let mut row = String::new();
            let mut cells = 0usize;
            for grapheme in line.graphemes(true) {
                let next = display_width(grapheme);
                if cells.saturating_add(next) > width && !row.is_empty() {
                    // Prefer a word boundary; an unbroken token (including CJK)
                    // still wraps safely on a grapheme boundary.
                    if let Some((byte, ch)) = row
                        .char_indices()
                        .rev()
                        .find(|(byte, ch)| *byte > 0 && ch.is_whitespace())
                    {
                        let tail = row[byte + ch.len_utf8()..].to_string();
                        rows.push(row[..byte].to_string());
                        row = tail;
                        cells = display_width(&row);
                        if cells.saturating_add(next) > width {
                            rows.push(std::mem::take(&mut row));
                            cells = 0;
                        }
                    } else {
                        rows.push(std::mem::take(&mut row));
                        cells = 0;
                    }
                }
                row.push_str(grapheme);
                cells += next;
            }
            rows.push(row);
            rows.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn wrap_graph_labels(graph: &mut Graph, width: usize) {
    let width = width.max(1);
    for node in graph.nodes.values_mut() {
        node.label = wrap(&node.label, width);
    }
    for edge in &mut graph.edges {
        if let Some(label) = &edge.label {
            edge.wrapped_label_lines =
                Some(wrap(label, width).split('\n').map(str::to_owned).collect());
        }
        // Endpoint labels have no independent wrap artifact; this is a view
        // clone and the original endpoint label remains in the prepared IR.
        for label in [&mut edge.head_label, &mut edge.tail_label]
            .into_iter()
            .flatten()
        {
            *label = wrap(label, width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrap_preserves_graphemes_and_literal_escapes() {
        assert_eq!(
            wrap("e\u{301}e\u{301}e\u{301}", 2),
            "e\u{301}e\u{301}\ne\u{301}"
        );
        assert_eq!(wrap("中文标题", 4), "中文\n标题");
        assert_eq!(wrap("👩‍💻👩‍💻", 2), "👩‍💻\n👩‍💻");
        assert_eq!(wrap(r"x\ny", 20), r"x\ny");
        assert_eq!(
            wrap("Class\n---\nmethod()", 3),
            "Cla\nss\n---\nmet\nhod\n()"
        );
    }
}
