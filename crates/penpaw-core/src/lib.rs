//! PatternStudio core: layered IR, pipeline driver, and configuration.

pub mod config;
pub mod error;
pub mod ir;
pub mod pipeline;

pub use config::{PenPawConfig, Resolution, Simplify};
pub use error::Error;
pub use ir::{Layer, LayerAssignment, LayeredDoc, LayerRole, SemanticMetadata};
pub use pipeline::PenPawPipeline;
