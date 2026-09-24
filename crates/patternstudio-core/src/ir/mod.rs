//! Core intermediate representation for PatternStudio.

mod layered_doc;

pub use layered_doc::{
    shape_bbox, EditabilityFlags, Layer, LayerAssignment, LayerRole, LayerSpec, LayeredDoc,
    SemanticMetadata,
};

pub use vtracer::ir::{MultiPath, Paint, PathCmd, Shape, SubPath, VectorDoc};
