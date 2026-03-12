use std::sync::Arc;

use crate::backend::SynapticaBackend;

/// Read a resource by URI.
pub async fn read_resource(
    uri: &str,
    backend: &Arc<dyn SynapticaBackend>,
) -> anyhow::Result<String> {
    match uri {
        "synaptica://schema" => {
            let schema = backend.get_schema("").await?;
            let mut out = String::new();
            out.push_str("=== Synaptica Graph Schema ===\n\n");
            out.push_str("Node Labels:\n");
            for ls in &schema.node_labels {
                out.push_str(&format!(
                    "  :{} ({} nodes) — properties: [{}]\n",
                    ls.label,
                    ls.count,
                    ls.property_keys.join(", ")
                ));
            }
            out.push_str("\nEdge Labels:\n");
            for ls in &schema.edge_labels {
                out.push_str(&format!(
                    "  :{} ({} edges) — properties: [{}]\n",
                    ls.label,
                    ls.count,
                    ls.property_keys.join(", ")
                ));
            }
            Ok(out)
        }
        "synaptica://labels" => {
            let labels = backend.list_labels("").await?;
            let mut out = String::from("=== Labels ===\n");
            for l in &labels {
                out.push_str(&format!("  :{} ({}, count: {})\n", l.name, l.kind, l.count));
            }
            Ok(out)
        }
        _ => anyhow::bail!("Unknown resource URI: {}", uri),
    }
}
