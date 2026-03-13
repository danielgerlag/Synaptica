//! Catalog interface for graph metadata.
//!
//! Provides a trait for resolving graph names, labels and properties, plus an
//! in-memory implementation for testing.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A unique identifier for a graph within the catalog.
pub type GraphId = u64;

/// Metadata about a graph stored in the catalog.
#[derive(Debug, Clone)]
pub struct GraphMeta {
    pub id: GraphId,
    pub name: String,
}

/// Catalog provides access to graph metadata.
pub trait Catalog {
    /// Look up a graph by name, returning its metadata if it exists.
    fn get_graph(&self, name: &str) -> Option<&GraphMeta>;

    /// Return the set of node/edge labels defined for `graph_id`.
    fn get_labels(&self, graph_id: GraphId) -> Vec<String>;

    /// Return the property names defined for a given `label` within `graph_id`.
    fn get_properties(&self, graph_id: GraphId, label: &str) -> Vec<String>;

    /// Convenience check: does a graph with the given `name` exist?
    fn graph_exists(&self, name: &str) -> bool {
        self.get_graph(name).is_some()
    }
}

// ---------------------------------------------------------------------------
// In-memory implementation (for tests)
// ---------------------------------------------------------------------------

/// Simple in-memory catalog suitable for unit tests and prototyping.
#[derive(Debug, Clone, Default)]
pub struct InMemoryCatalog {
    graphs: HashMap<String, GraphMeta>,
    /// graph_id -> list of labels
    labels: HashMap<GraphId, Vec<String>>,
    /// (graph_id, label) -> list of property names
    properties: HashMap<(GraphId, String), Vec<String>>,
}

impl InMemoryCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a graph with the given name and id.
    pub fn add_graph(&mut self, name: impl Into<String>, id: GraphId) {
        let name = name.into();
        self.graphs.insert(name.clone(), GraphMeta { id, name });
    }

    /// Register labels for a graph.
    pub fn add_labels(&mut self, graph_id: GraphId, labels: Vec<String>) {
        self.labels.entry(graph_id).or_default().extend(labels);
    }

    /// Register properties for a (graph, label) pair.
    pub fn add_properties(
        &mut self,
        graph_id: GraphId,
        label: impl Into<String>,
        properties: Vec<String>,
    ) {
        self.properties
            .entry((graph_id, label.into()))
            .or_default()
            .extend(properties);
    }
}

impl Catalog for InMemoryCatalog {
    fn get_graph(&self, name: &str) -> Option<&GraphMeta> {
        self.graphs.get(name)
    }

    fn get_labels(&self, graph_id: GraphId) -> Vec<String> {
        self.labels.get(&graph_id).cloned().unwrap_or_default()
    }

    fn get_properties(&self, graph_id: GraphId, label: &str) -> Vec<String> {
        self.properties
            .get(&(graph_id, label.to_owned()))
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_catalog_roundtrip() {
        let mut cat = InMemoryCatalog::new();
        cat.add_graph("social", 1);
        cat.add_labels(1, vec!["Person".into(), "Post".into()]);
        cat.add_properties(1, "Person", vec!["name".into(), "age".into()]);

        assert!(cat.graph_exists("social"));
        assert!(!cat.graph_exists("missing"));
        assert_eq!(cat.get_labels(1), vec!["Person", "Post"]);
        assert_eq!(cat.get_properties(1, "Person"), vec!["name", "age"]);
        assert!(cat.get_properties(1, "Post").is_empty());
    }
}
