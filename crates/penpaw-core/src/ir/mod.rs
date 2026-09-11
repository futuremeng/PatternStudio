//! Core intermediate representation for PatternStudio.

mod layered_doc;

pub use layered_doc::{EditabilityFlags, Layer, LayerAssignment, LayeredDoc, LayerRole, LayerSpec, SemanticMetadata};

pub use vtracer::ir::{MultiPath, Paint, PathCmd, Shape, SubPath, VectorDoc};
