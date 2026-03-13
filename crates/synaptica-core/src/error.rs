use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid node id: {0}")]
    InvalidNodeId(String),

    #[error("invalid edge id: {0}")]
    InvalidEdgeId(String),

    #[error("invalid graph id: {0}")]
    InvalidGraphId(String),

    #[error("type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    #[error("property not found: {0}")]
    PropertyNotFound(String),

    #[error("label not found: {0}")]
    LabelNotFound(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("deserialization error: {0}")]
    Deserialization(String),
}

pub type CoreResult<T> = std::result::Result<T, CoreError>;
