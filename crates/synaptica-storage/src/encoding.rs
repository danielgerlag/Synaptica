use synaptica_core::graph::{EdgeId, GraphId, Label, NodeId};

/// Key encoding utilities for RocksDB.
///
/// All keys use big-endian byte encoding to ensure correct lexicographic
/// ordering in RocksDB, which is critical for range-based partitioning
/// and efficient prefix scans.

const UUID_LEN: usize = 16;

/// Encode a UUID as 16 big-endian bytes.
fn encode_uuid(uuid_bytes: &[u8; 16], buf: &mut Vec<u8>) {
    buf.extend_from_slice(uuid_bytes);
}

/// Encode a label string as length-prefixed bytes.
/// Labels exceeding u16::MAX bytes are truncated (such lengths are not meaningful).
fn encode_label(label: &str, buf: &mut Vec<u8>) {
    let bytes = label.as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    buf.extend_from_slice(&(len as u16).to_be_bytes());
    buf.extend_from_slice(&bytes[..len]);
}

/// Decode a label from length-prefixed bytes, returning (label, bytes_consumed).
pub fn decode_label(data: &[u8]) -> Option<(String, usize)> {
    if data.len() < 2 {
        return None;
    }
    let len = u16::from_be_bytes([data[0], data[1]]) as usize;
    if data.len() < 2 + len {
        return None;
    }
    let s = std::str::from_utf8(&data[2..2 + len]).ok()?;
    Some((s.to_string(), 2 + len))
}

// --- Node keys ---

/// Encode a node key: graph_id ++ node_id
pub fn encode_node_key(graph_id: &GraphId, node_id: &NodeId) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN * 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_uuid(node_id.as_bytes(), &mut buf);
    buf
}

/// Decode a node key back into (GraphId, NodeId).
pub fn decode_node_key(data: &[u8]) -> Option<(GraphId, NodeId)> {
    if data.len() < UUID_LEN * 2 {
        return None;
    }
    let graph_id = GraphId::from_bytes(data[..UUID_LEN].try_into().ok()?);
    let node_id = NodeId::from_bytes(data[UUID_LEN..UUID_LEN * 2].try_into().ok()?);
    Some((graph_id, node_id))
}

/// Encode prefix for scanning all nodes in a graph.
pub fn encode_node_prefix(graph_id: &GraphId) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    buf
}

// --- Edge keys ---

/// Encode an edge key: graph_id ++ edge_id
pub fn encode_edge_key(graph_id: &GraphId, edge_id: &EdgeId) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN * 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_uuid(edge_id.as_bytes(), &mut buf);
    buf
}

/// Decode an edge key back into (GraphId, EdgeId).
pub fn decode_edge_key(data: &[u8]) -> Option<(GraphId, EdgeId)> {
    if data.len() < UUID_LEN * 2 {
        return None;
    }
    let graph_id = GraphId::from_bytes(data[..UUID_LEN].try_into().ok()?);
    let edge_id = EdgeId::from_bytes(data[UUID_LEN..UUID_LEN * 2].try_into().ok()?);
    Some((graph_id, edge_id))
}

// --- Adjacency keys ---

/// Encode outgoing adjacency key: graph_id ++ source_node_id ++ label ++ edge_id
pub fn encode_adj_out_key(
    graph_id: &GraphId,
    source: &NodeId,
    label: &Label,
    edge_id: &EdgeId,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN * 3 + label.as_str().len() + 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_uuid(source.as_bytes(), &mut buf);
    encode_label(label.as_str(), &mut buf);
    encode_uuid(edge_id.as_bytes(), &mut buf);
    buf
}

/// Encode incoming adjacency key: graph_id ++ target_node_id ++ label ++ edge_id
pub fn encode_adj_in_key(
    graph_id: &GraphId,
    target: &NodeId,
    label: &Label,
    edge_id: &EdgeId,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN * 3 + label.as_str().len() + 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_uuid(target.as_bytes(), &mut buf);
    encode_label(label.as_str(), &mut buf);
    encode_uuid(edge_id.as_bytes(), &mut buf);
    buf
}

/// Encode prefix for scanning all outgoing edges of a node in a graph.
pub fn encode_adj_out_prefix(graph_id: &GraphId, source: &NodeId) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN * 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_uuid(source.as_bytes(), &mut buf);
    buf
}

/// Encode prefix for scanning all outgoing edges with a specific label.
pub fn encode_adj_out_label_prefix(
    graph_id: &GraphId,
    source: &NodeId,
    label: &Label,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN * 2 + label.as_str().len() + 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_uuid(source.as_bytes(), &mut buf);
    encode_label(label.as_str(), &mut buf);
    buf
}

/// Encode prefix for scanning all incoming edges of a node in a graph.
pub fn encode_adj_in_prefix(graph_id: &GraphId, target: &NodeId) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN * 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_uuid(target.as_bytes(), &mut buf);
    buf
}

/// Encode prefix for scanning all incoming edges with a specific label.
pub fn encode_adj_in_label_prefix(
    graph_id: &GraphId,
    target: &NodeId,
    label: &Label,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN * 2 + label.as_str().len() + 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_uuid(target.as_bytes(), &mut buf);
    encode_label(label.as_str(), &mut buf);
    buf
}

// --- Label index keys ---

/// Encode node label index key: graph_id ++ label ++ node_id
pub fn encode_node_label_key(graph_id: &GraphId, label: &Label, node_id: &NodeId) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN * 2 + label.as_str().len() + 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_label(label.as_str(), &mut buf);
    encode_uuid(node_id.as_bytes(), &mut buf);
    buf
}

/// Encode prefix for scanning all nodes with a specific label.
pub fn encode_node_label_prefix(graph_id: &GraphId, label: &Label) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN + label.as_str().len() + 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_label(label.as_str(), &mut buf);
    buf
}

/// Encode edge label index key: graph_id ++ label ++ edge_id
pub fn encode_edge_label_key(graph_id: &GraphId, label: &Label, edge_id: &EdgeId) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN * 2 + label.as_str().len() + 2);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    encode_label(label.as_str(), &mut buf);
    encode_uuid(edge_id.as_bytes(), &mut buf);
    buf
}

// --- Graph metadata keys ---

/// Encode a graph metadata key.
pub fn encode_graph_meta_key(graph_id: &GraphId) -> Vec<u8> {
    let mut buf = Vec::with_capacity(UUID_LEN);
    encode_uuid(graph_id.as_bytes(), &mut buf);
    buf
}

// --- Value serialization ---

/// Serialize a value using bincode for compact storage.
pub fn serialize_value<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, String> {
    bincode::serialize(value).map_err(|e| e.to_string())
}

/// Deserialize a value from bincode bytes.
pub fn deserialize_value<'a, T: serde::Deserialize<'a>>(data: &'a [u8]) -> Result<T, String> {
    bincode::deserialize(data).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_key_roundtrip() {
        let graph_id = GraphId::new();
        let node_id = NodeId::new();
        let key = encode_node_key(&graph_id, &node_id);
        let (g, n) = decode_node_key(&key).unwrap();
        assert_eq!(g, graph_id);
        assert_eq!(n, node_id);
    }

    #[test]
    fn test_edge_key_roundtrip() {
        let graph_id = GraphId::new();
        let edge_id = EdgeId::new();
        let key = encode_edge_key(&graph_id, &edge_id);
        let (g, e) = decode_edge_key(&key).unwrap();
        assert_eq!(g, graph_id);
        assert_eq!(e, edge_id);
    }

    #[test]
    fn test_node_prefix_is_prefix_of_node_key() {
        let graph_id = GraphId::new();
        let node_id = NodeId::new();
        let prefix = encode_node_prefix(&graph_id);
        let key = encode_node_key(&graph_id, &node_id);
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_adj_out_prefix() {
        let graph_id = GraphId::new();
        let source = NodeId::new();
        let label = Label::new("KNOWS");
        let edge_id = EdgeId::new();
        let key = encode_adj_out_key(&graph_id, &source, &label, &edge_id);
        let prefix = encode_adj_out_prefix(&graph_id, &source);
        assert!(key.starts_with(&prefix));
        let label_prefix = encode_adj_out_label_prefix(&graph_id, &source, &label);
        assert!(key.starts_with(&label_prefix));
    }

    #[test]
    fn test_label_encode_decode() {
        let label = "PERSON";
        let mut buf = Vec::new();
        encode_label(label, &mut buf);
        let (decoded, consumed) = decode_label(&buf).unwrap();
        assert_eq!(decoded, label);
        assert_eq!(consumed, buf.len());
    }

    #[test]
    fn test_serialization_roundtrip() {
        use synaptica_core::graph::Node;
        let graph_id = GraphId::new();
        let mut node = Node::new(graph_id);
        node.add_label("Person");
        node.set_property("name", synaptica_core::types::Value::String("Alice".into()));

        let bytes = serialize_value(&node).unwrap();
        let restored: Node = deserialize_value(&bytes).unwrap();
        assert_eq!(node, restored);
    }
}