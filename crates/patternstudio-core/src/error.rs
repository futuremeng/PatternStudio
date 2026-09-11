//! Error types for PatternStudio.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("vtracer error: {0}")]
    VTracer(#[from] vtracer::Error),

    #[error("image decode error: {0}")]
    Image(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}