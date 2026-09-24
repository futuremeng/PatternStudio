//! PatternStudio SVG serialization: layered SVG writer.
//!
//! Converts `LayeredDoc` to SVG with semantic `<g id="...">` groups,
//! editability attributes, and metadata comments.

mod writer;

pub use writer::LayeredWriter;
