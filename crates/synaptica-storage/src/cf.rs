/// Column family names used by the storage engine.
pub struct ColumnFamilies;

impl ColumnFamilies {
    /// Default CF — used for system metadata.
    pub const DEFAULT: &'static str = "default";

    /// Nodes CF — stores node data keyed by (graph_id, node_id).
    pub const NODES: &'static str = "nodes";

    /// Edges CF — stores edge data keyed by (graph_id, edge_id).
    pub const EDGES: &'static str = "edges";

    /// Outgoing adjacency CF — keyed by (graph_id, source_node_id, label, edge_id).
    pub const ADJ_OUT: &'static str = "adj_out";

    /// Incoming adjacency CF — keyed by (graph_id, target_node_id, label, edge_id).
    pub const ADJ_IN: &'static str = "adj_in";

    /// Label-to-node index CF — keyed by (graph_id, label, node_id).
    pub const NODE_LABELS: &'static str = "node_labels";

    /// Label-to-edge index CF — keyed by (graph_id, label, edge_id).
    pub const EDGE_LABELS: &'static str = "edge_labels";

    /// Property indexes CF — keyed by (graph_id, property_name, value, entity_id).
    pub const PROP_INDEX: &'static str = "prop_index";

    /// Graph metadata CF — keyed by graph_id.
    pub const GRAPH_META: &'static str = "graph_meta";

    /// Schema CF — stores graph type schemas.
    pub const SCHEMA: &'static str = "schema";

    /// Raft log CF — stores Raft log entries keyed by log index.
    pub const RAFT_LOG: &'static str = "raft_log";

    /// Raft metadata CF — stores vote, last_applied, membership, snapshot meta.
    pub const RAFT_META: &'static str = "raft_meta";

    /// Returns all column family names.
    pub fn all() -> &'static [&'static str] {
        &[
            Self::DEFAULT,
            Self::NODES,
            Self::EDGES,
            Self::ADJ_OUT,
            Self::ADJ_IN,
            Self::NODE_LABELS,
            Self::EDGE_LABELS,
            Self::PROP_INDEX,
            Self::GRAPH_META,
            Self::SCHEMA,
            Self::RAFT_LOG,
            Self::RAFT_META,
        ]
    }
}
