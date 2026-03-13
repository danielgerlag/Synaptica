use crate::graph::Label;
use crate::types::DataType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Schema definition for a node type within a graph type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeTypeSchema {
    pub labels: Vec<Label>,
    pub properties: BTreeMap<String, PropertySchema>,
}

/// Schema definition for an edge type within a graph type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeTypeSchema {
    pub label: Label,
    pub source_labels: Vec<Label>,
    pub target_labels: Vec<Label>,
    pub properties: BTreeMap<String, PropertySchema>,
}

/// Schema for a single property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertySchema {
    pub data_type: DataType,
    pub nullable: bool,
    pub default: Option<crate::types::Value>,
}

/// A graph type schema defining allowed node and edge types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphTypeSchema {
    pub name: String,
    pub node_types: Vec<NodeTypeSchema>,
    pub edge_types: Vec<EdgeTypeSchema>,
}
