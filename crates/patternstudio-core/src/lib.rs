//! PatternStudio core: layered IR, pipeline driver, and configuration.

pub mod config;
pub mod error;
pub mod ir;
pub mod lineage;
pub mod pipeline;
pub mod profiles;
pub mod session_cache;
pub mod smart;

pub use config::{PatternStudioConfig, Resolution, Simplify};
pub use error::Error;
pub use ir::{Layer, LayerAssignment, LayerRole, LayeredDoc, SemanticMetadata};
pub use lineage::{
    generated_dir, infer_entries, lineage_path, LineageEntry, LineageFile, KIND_ENHANCE,
    KIND_REDRAW,
};
pub use pipeline::PatternStudioPipeline;
pub use profiles::{apply_profiles, AutoDecision, IdealParams, LayerProfile, ProfileLevel};
pub use session_cache::{key_for_path, key_for_source, SessionCache};
pub use smart::{
    analyze_image, candidate_params, recommend, ImageStats, SmartDecision, SmartParams,
};
