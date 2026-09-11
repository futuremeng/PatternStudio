//! Layered document IR: the core output type for PatternStudio.
//!
//! Wraps VTracer's flat `VectorDoc` with semantic layers, editability flags,
//! and metadata. Each layer maps to a `<g>` element in the output SVG.

use serde::{Deserialize, Serialize};
use vtracer::ir::{Shape, VectorDoc};
use visioncortex::Color;

/// Semantic role of a layer — determines SVG `<g id>` and edit behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerRole {
    Background,
    Structure,
    Details,
    Pattern,
    Custom(String),
}

impl std::fmt::Display for LayerRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayerRole::Background => write!(f, "background"),
            LayerRole::Structure => write!(f, "structure"),
            LayerRole::Details => write!(f, "details"),
            LayerRole::Pattern => write!(f, "pattern"),
            LayerRole::Custom(name) => write!(f, "{name}"),
        }
    }
}

/// Editability flags controlling which operations are allowed on a layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditabilityFlags {
    pub movable: bool,
    pub recolorable: bool,
    pub resizable: bool,
    pub deletable: bool,
    pub editable_paths: bool,
}

impl EditabilityFlags {
    pub fn full() -> Self {
        Self {
            movable: true,
            recolorable: true,
            resizable: true,
            deletable: true,
            editable_paths: true,
        }
    }

    pub fn to_svg_attr(&self) -> String {
        let mut parts = Vec::new();
        if self.movable { parts.push("move"); }
        if self.recolorable { parts.push("recolor"); }
        if self.resizable { parts.push("resize"); }
        if self.deletable { parts.push("delete"); }
        if self.editable_paths { parts.push("paths"); }
        parts.join(",")
    }
}

/// Semantic metadata written to SVG comments and `data-*` attributes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SemanticMetadata {
    #[serde(default)]
    pub pattern_type: String,
    #[serde(default)]
    pub n_units: u32,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub engine: String,
    #[serde(default)]
    pub model_version: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A single semantic layer — corresponds to one `<g id="...">` in SVG.
#[derive(Debug, Clone)]
pub struct Layer {
    pub name: String,
    pub role: LayerRole,
    pub shapes: Vec<Shape>,
    pub editability: EditabilityFlags,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

/// Layer assignment: maps shape indices in a `VectorDoc` to named layers.
#[derive(Debug, Clone, Default)]
pub struct LayerAssignment {
    pub layers: Vec<LayerSpec>,
}

/// Specification for one layer in the assignment.
#[derive(Debug, Clone)]
pub struct LayerSpec {
    pub name: String,
    pub role: LayerRole,
    pub shape_indices: Vec<usize>,
    pub editability: EditabilityFlags,
}

impl LayerAssignment {
    /// Default assignment for pattern tracing: all shapes go to "structure".
    pub fn default_pattern() -> Self {
        Self {
            layers: vec![LayerSpec {
                name: "structure".into(),
                role: LayerRole::Structure,
                shape_indices: Vec::new(),
                editability: EditabilityFlags::full(),
            }],
        }
    }

    /// Single-layer assignment: everything in one "artwork" layer.
    pub fn single_layer() -> Self {
        Self {
            layers: vec![LayerSpec {
                name: "artwork".into(),
                role: LayerRole::Custom("artwork".into()),
                shape_indices: Vec::new(),
                editability: EditabilityFlags::full(),
            }],
        }
    }

    /// Auto-assign shapes to layers by their fill color.
    ///
    /// Groups shapes with the same color into one layer.
    /// The most common color becomes "background", the rest become "color-N".
    /// If there are more distinct colors than `max_layers`, the smallest groups
    /// are merged into the last layer ("other").
    pub fn by_color(doc: &VectorDoc, max_layers: usize) -> Self {
        use std::collections::HashMap;

        // Group shape indices by color
        let mut color_groups: HashMap<[u8; 3], Vec<usize>> = HashMap::new();
        for (i, shape) in doc.shapes.iter().enumerate() {
            let [r, g, b] = shape.paint.color().rgb_u8();
            color_groups.entry([r, g, b]).or_default().push(i);
        }

        // Sort by group size descending (most common = background)
        let mut groups: Vec<([u8; 3], Vec<usize>)> = color_groups.into_iter().collect();
        groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

        if groups.len() <= max_layers {
            // No overflow: each color gets its own layer
            let layers = groups
                .into_iter()
                .enumerate()
                .map(|(idx, (color, indices))| {
                    let name = if idx == 0 {
                        "background".into()
                    } else {
                        format!("color-{:02x}{:02x}{:02x}", color[0], color[1], color[2])
                    };
                    let role = if idx == 0 {
                        LayerRole::Background
                    } else {
                        LayerRole::Custom(name.clone())
                    };
                    LayerSpec {
                        name,
                        role,
                        shape_indices: indices,
                        editability: EditabilityFlags::full(),
                    }
                })
                .collect();
            return Self { layers };
        }

        // Overflow: keep top (max_layers - 1) colors, merge rest into "other"
        let keep = max_layers.saturating_sub(1);
        let top: Vec<([u8; 3], Vec<usize>)> = groups[..keep].to_vec();
        let mut other_indices: Vec<usize> = Vec::new();
        for (_, indices) in &groups[keep..] {
            other_indices.extend(indices.iter().copied());
        }

        let mut layers: Vec<LayerSpec> = top
            .into_iter()
            .enumerate()
            .map(|(idx, (color, indices))| {
                let name = if idx == 0 {
                    "background".into()
                } else {
                    format!("color-{:02x}{:02x}{:02x}", color[0], color[1], color[2])
                };
                let role = if idx == 0 {
                    LayerRole::Background
                } else {
                    LayerRole::Custom(name.clone())
                };
                LayerSpec {
                    name,
                    role,
                    shape_indices: indices,
                    editability: EditabilityFlags::full(),
                }
            })
            .collect();

        // Add "other" layer with all remaining shapes
        layers.push(LayerSpec {
            name: "other".into(),
            role: LayerRole::Details,
            shape_indices: other_indices,
            editability: EditabilityFlags::full(),
        });

        Self { layers }
    }
}

/// Layered document IR — PatternStudio's core output type.
#[derive(Debug, Clone)]
pub struct LayeredDoc {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
    pub metadata: SemanticMetadata,
}

impl LayeredDoc {
    /// Flatten to VTracer's `VectorDoc` (concatenate all layers' shapes).
    pub fn to_vector_doc(&self) -> VectorDoc {
        let mut doc = VectorDoc::new(self.width, self.height);
        for layer in &self.layers {
            doc.shapes.extend(layer.shapes.iter().cloned());
        }
        doc
    }

    /// Build a `LayeredDoc` from a `VectorDoc` and a layer assignment.
    pub fn from_vector_doc(doc: &VectorDoc, assignment: &LayerAssignment) -> Self {
        let mut layers = Vec::new();

        if assignment.layers.is_empty() {
            layers.push(Layer {
                name: "artwork".into(),
                role: LayerRole::Custom("artwork".into()),
                shapes: doc.shapes.clone(),
                editability: EditabilityFlags::full(),
                metadata: serde_json::Map::new(),
            });
        } else {
            for spec in &assignment.layers {
                let shapes: Vec<Shape> = if spec.shape_indices.is_empty() {
                    if layers.is_empty() {
                        doc.shapes.clone()
                    } else {
                        Vec::new()
                    }
                } else {
                    spec.shape_indices
                        .iter()
                        .filter(|&&i| i < doc.shapes.len())
                        .map(|&i| doc.shapes[i].clone())
                        .collect()
                };
                layers.push(Layer {
                    name: spec.name.clone(),
                    role: spec.role.clone(),
                    shapes,
                    editability: spec.editability,
                    metadata: serde_json::Map::new(),
                });
            }
        }

        Self {
            width: doc.width,
            height: doc.height,
            layers,
            metadata: SemanticMetadata::default(),
        }
    }

    pub fn layer_by_role(&self, role: &LayerRole) -> Option<&Layer> {
        self.layers.iter().find(|l| &l.role == role)
    }

    pub fn layer_by_name(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }

    pub fn insert_layer(&mut self, index: usize, layer: Layer) {
        self.layers.insert(index, layer);
    }

    pub fn remove_layer(&mut self, index: usize) -> Option<Layer> {
        if index < self.layers.len() {
            Some(self.layers.remove(index))
        } else {
            None
        }
    }

    pub fn recolor_layer(&mut self, index: usize, color: Color) -> bool {
        if let Some(layer) = self.layers.get_mut(index) {
            for shape in &mut layer.shapes {
                shape.paint = vtracer::ir::Paint::Solid(color);
            }
            true
        } else {
            false
        }
    }

    pub fn total_shapes(&self) -> usize {
        self.layers.iter().map(|l| l.shapes.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vtracer::ir::{MultiPath, SubPath};
    use visioncortex::PointF64;

    fn test_shape(color: u8) -> Shape {
        let mut sub = SubPath::new();
        sub.commands.push(vtracer::ir::PathCmd::MoveTo(PointF64 { x: 0.0, y: 0.0 }));
        sub.commands.push(vtracer::ir::PathCmd::LineTo(PointF64 { x: 10.0, y: 0.0 }));
        sub.commands.push(vtracer::ir::PathCmd::LineTo(PointF64 { x: 10.0, y: 10.0 }));
        sub.commands.push(vtracer::ir::PathCmd::Close);
        Shape {
            paint: vtracer::ir::Paint::Solid(Color::new(color, color, color)),
            path: MultiPath { subpaths: vec![sub] },
        }
    }

    #[test]
    fn layered_doc_to_vector_doc_roundtrip() {
        let mut doc = VectorDoc::new(100, 100);
        doc.shapes.push(test_shape(255));
        doc.shapes.push(test_shape(128));

        let assignment = LayerAssignment::single_layer();
        let layered = LayeredDoc::from_vector_doc(&doc, &assignment);

        assert_eq!(layered.layers.len(), 1);
        assert_eq!(layered.layers[0].shapes.len(), 2);
        assert_eq!(layered.total_shapes(), 2);

        let flat = layered.to_vector_doc();
        assert_eq!(flat.shapes.len(), 2);
        assert_eq!(flat.width, 100);
        assert_eq!(flat.height, 100);
    }

    #[test]
    fn layer_insert_remove() {
        let mut doc = VectorDoc::new(50, 50);
        doc.shapes.push(test_shape(200));
        let layered = LayeredDoc::from_vector_doc(&doc, &LayerAssignment::single_layer());

        let mut result = layered;
        let new_layer = Layer {
            name: "overlay".into(),
            role: LayerRole::Details,
            shapes: vec![test_shape(50)],
            editability: EditabilityFlags::full(),
            metadata: serde_json::Map::new(),
        };
        result.insert_layer(1, new_layer);
        assert_eq!(result.layers.len(), 2);
        assert_eq!(result.layers[1].name, "overlay");

        let removed = result.remove_layer(1);
        assert!(removed.is_some());
        assert_eq!(result.layers.len(), 1);
    }

    #[test]
    fn recolor_layer_works() {
        let mut doc = VectorDoc::new(10, 10);
        doc.shapes.push(test_shape(255));
        let mut layered = LayeredDoc::from_vector_doc(&doc, &LayerAssignment::single_layer());

        let new_color = Color::new(10, 20, 30);
        assert!(layered.recolor_layer(0, new_color));
        assert_eq!(layered.layers[0].shapes[0].paint.color(), new_color);
    }

    #[test]
    fn editability_flags_svg_attr() {
        let flags = EditabilityFlags::full();
        assert_eq!(flags.to_svg_attr(), "move,recolor,resize,delete,paths");

        let none = EditabilityFlags::default();
        assert_eq!(none.to_svg_attr(), "");
    }
}