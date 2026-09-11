//! PatternStudio tracing: VTracer integration and custom frontends.
//!
//! Provides `MaskFrontend` (background removal) and image loading utilities.

pub mod frontend;
pub mod image_io;

pub use frontend::MaskFrontend;
pub use image_io::{load_image, load_image_from_memory};