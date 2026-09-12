//! PatternStudio core: layered IR, pipeline driver, and configuration.

pub mod config;
pub mod error;
pub mod ir;
pub mod pipeline;
pub mod profiles;

pub use config::{PatternStudioConfig, Resolution, Simplify};
pub use error::Error;
pub use ir::{Layer, LayerAssignment, LayeredDoc, LayerRole, SemanticMetadata};
pub use pipeline::PatternStudioPipeline;
pub use profiles::{apply_profiles, AutoDecision, IdealParams, LayerProfile, ProfileLevel};
