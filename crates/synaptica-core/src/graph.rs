use crate::types::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Unique identifier for a graph within the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GraphId(pub Uuid);

impl GraphId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }
}

impl Default for GraphId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for a node within a graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for an edge within a graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EdgeId(pub Uuid);

impl EdgeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }
}

impl Default for EdgeId {
    fn default() -> Self {
        Self::new()
    }
}

/// A label applied to a node or edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Label(pub String);

impl Label {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Label {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for Label {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Properties stored on a node or edge.
pub type Properties = BTreeMap<String, Value>;

/// A node (vertex) in the property graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub graph_id: GraphId,
    pub labels: BTreeSet<Label>,
    pub properties: Properties,
}

impl Node {
    pub fn new(graph_id: GraphId) -> Self {
        Self {
            id: NodeId::new(),
            graph_id,
            labels: BTreeSet::new(),
            properties: BTreeMap::new(),
        }
    }

    pub fn with_id(graph_id: GraphId, id: NodeId) -> Self {
        Self {
            id,
            graph_id,
            labels: BTreeSet::new(),
            properties: BTreeMap::new(),
        }
    }

    pub fn add_label(&mut self, label: impl Into<Label>) -> &mut Self {
        self.labels.insert(label.into());
        self
    }

    pub fn set_property(&mut self, key: impl Into<String>, value: Value) -> &mut Self {
        self.properties.insert(key.into(), value);
        self
    }

    pub fn get_property(&self, key: &str) -> Option<&Value> {
        self.properties.get(key)
    }

    pub fn has_label(&self, label: &str) -> bool {
        self.labels.contains(&Label(label.to_string()))
    }
}

/// Edge direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Outgoing,
    Incoming,
    Both,
}

/// An edge (relationship) in the property graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub graph_id: GraphId,
    pub source: NodeId,
    pub target: NodeId,
    pub label: Label,
    pub properties: Properties,
}

impl Edge {
    pub fn new(
        graph_id: GraphId,
        source: NodeId,
        target: NodeId,
        label: impl Into<Label>,
    ) -> Self {
        Self {
            id: EdgeId::new(),
            graph_id,
            source,
            target,
            label: label.into(),
            properties: BTreeMap::new(),
        }
    }

    pub fn with_id(
        graph_id: GraphId,
        id: EdgeId,
        source: NodeId,
        target: NodeId,
        label: impl Into<Label>,
    ) -> Self {
        Self {
            id,
            graph_id,
            source,
            target,
            label: label.into(),
            properties: BTreeMap::new(),
        }
    }

    pub fn set_property(&mut self, key: impl Into<String>, value: Value) -> &mut Self {
        self.properties.insert(key.into(), value);
        self
    }

    pub fn get_property(&self, key: &str) -> Option<&Value> {
        self.properties.get(key)
    }

    /// Returns the other endpoint of this edge relative to the given node.
    pub fn other_node(&self, node: NodeId) -> Option<NodeId> {
        if self.source == node {
            Some(self.target)
        } else if self.target == node {
            Some(self.source)
        } else {
            None
        }
    }
}

/// Metadata for a named graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphMeta {
    pub id: GraphId,
    pub name: String,
    pub graph_type: Option<String>,
}