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
    /// Multi-level parameter profile applied to this layer (auto strategy).
    pub profile: crate::profiles::ProfileLevel,
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
    /// Multi-level parameter profile for this layer (auto strategy).
    pub profile: crate::profiles::ProfileLevel,
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
                profile: crate::profiles::ProfileLevel::default(),
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
                profile: crate::profiles::ProfileLevel::default(),
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
                        profile: crate::profiles::ProfileLevel::default(),
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
                    profile: crate::profiles::ProfileLevel::default(),
                }
            })
            .collect();

        // Add "other" layer with all remaining shapes
        layers.push(LayerSpec {
            name: "other".into(),
            role: LayerRole::Details,
            shape_indices: other_indices,
            editability: EditabilityFlags::full(),
            profile: crate::profiles::ProfileLevel::default(),
        });

        Self { layers }
    }

    /// Assign shapes to layers by geometric complexity (path command count).
    pub fn by_complexity(doc: &VectorDoc, threshold_percent: u8, max_layers: usize) -> Self {
        if max_layers <= 1 {
            return Self::single_layer();
        }
        let complexities: Vec<(usize, usize)> = doc
            .shapes
            .iter()
            .enumerate()
            .map(|(i, shape)| {
                let cmd_count: usize = shape.path.subpaths.iter().map(|sp| sp.commands.len()).sum();
                (i, cmd_count)
            })
            .collect();
        let mut sorted_counts: Vec<usize> = complexities.iter().map(|(_, c)| *c).collect();
        sorted_counts.sort();
        let idx = (threshold_percent as usize * sorted_counts.len()) / 100;
        let threshold = sorted_counts.get(idx).copied().unwrap_or(0);

        let mut simple_indices: Vec<usize> = Vec::new();
        let mut complex_indices: Vec<usize> = Vec::new();
        for (i, count) in &complexities {
            if *count > threshold {
                complex_indices.push(*i);
            } else {
                simple_indices.push(*i);
            }
        }
        if complex_indices.is_empty() {
            return Self::single_layer();
        }

        let mut layers: Vec<LayerSpec> = Vec::new();
        let simple_budget = max_layers.saturating_sub(1);
        if simple_budget >= 2 && simple_indices.len() > 1 {
            let sub = Self::by_color_indices(&doc.shapes, &simple_indices, simple_budget);
            layers.extend(sub);
        } else {
            layers.push(LayerSpec {
                name: "base".into(),
                role: LayerRole::Structure,
                shape_indices: simple_indices,
                editability: EditabilityFlags::full(),
                profile: crate::profiles::ProfileLevel::default(),
            });
        }
        layers.push(LayerSpec {
            name: "details".into(),
            role: LayerRole::Details,
            shape_indices: complex_indices,
            editability: EditabilityFlags::full(),
            profile: crate::profiles::ProfileLevel::default(),
        });
        Self { layers }
    }

    /// Hybrid: group by color, then split complex shapes into "details".
    pub fn hybrid(doc: &VectorDoc, max_layers: usize, threshold_percent: u8) -> Self {
        if max_layers <= 1 {
            return Self::single_layer();
        }
        let complexities: Vec<usize> = doc
            .shapes
            .iter()
            .map(|s| s.path.subpaths.iter().map(|sp| sp.commands.len()).sum())
            .collect();
        let mut sorted: Vec<usize> = complexities.clone();
        sorted.sort();
        let idx = (threshold_percent as usize * sorted.len()) / 100;
        let threshold = sorted.get(idx).copied().unwrap_or(0);

        use std::collections::HashMap;
        let mut color_groups: HashMap<[u8; 3], Vec<usize>> = HashMap::new();
        for (i, shape) in doc.shapes.iter().enumerate() {
            let [r, g, b] = shape.paint.color().rgb_u8();
            color_groups.entry([r, g, b]).or_default().push(i);
        }
        let mut groups: Vec<([u8; 3], Vec<usize>)> = color_groups.into_iter().collect();
        groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

        let color_budget = max_layers.saturating_sub(1).max(1);
        let mut layers: Vec<LayerSpec> = Vec::new();
        let mut details_indices: Vec<usize> = Vec::new();

        for (idx, (color, indices)) in groups.into_iter().enumerate().take(color_budget) {
            let mut base: Vec<usize> = Vec::new();
            for &i in &indices {
                if complexities[i] > threshold {
                    details_indices.push(i);
                } else {
                    base.push(i);
                }
            }
            if !base.is_empty() {
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
                layers.push(LayerSpec {
                    name, role, shape_indices: base,
                    editability: EditabilityFlags::full(),
                    profile: crate::profiles::ProfileLevel::default(),
                });
            }
        }
        if !details_indices.is_empty() {
            layers.push(LayerSpec {
                name: "details".into(),
                role: LayerRole::Details,
                shape_indices: details_indices,
                editability: EditabilityFlags::full(),
                profile: crate::profiles::ProfileLevel::default(),
            });
        }
        Self { layers }
    }

    /// Helper: assign a subset of shapes to layers by color.
    fn by_color_indices(shapes: &[Shape], indices: &[usize], max_layers: usize) -> Vec<LayerSpec> {
        use std::collections::HashMap;
        let mut color_groups: HashMap<[u8; 3], Vec<usize>> = HashMap::new();
        for &i in indices {
            let [r, g, b] = shapes[i].paint.color().rgb_u8();
            color_groups.entry([r, g, b]).or_default().push(i);
        }
        let mut groups: Vec<([u8; 3], Vec<usize>)> = color_groups.into_iter().collect();
        groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

        if groups.len() <= max_layers {
            return groups.into_iter().enumerate().map(|(idx, (color, indices))| {
                let name = if idx == 0 { "base".into() } else {
                    format!("color-{:02x}{:02x}{:02x}", color[0], color[1], color[2])
                };
                let role = if idx == 0 { LayerRole::Structure } else { LayerRole::Custom(name.clone()) };
                LayerSpec { name, role, shape_indices: indices, editability: EditabilityFlags::full(), profile: crate::profiles::ProfileLevel::default() }
            }).collect();
        }

        let keep = max_layers.saturating_sub(1);
        let top: Vec<([u8; 3], Vec<usize>)> = groups[..keep].to_vec();
        let mut other: Vec<usize> = Vec::new();
        for (_, indices) in &groups[keep..] { other.extend(indices.iter().copied()); }
        let mut result: Vec<LayerSpec> = top.into_iter().enumerate().map(|(idx, (color, indices))| {
            let name = if idx == 0 { "base".into() } else {
                format!("color-{:02x}{:02x}{:02x}", color[0], color[1], color[2])
            };
            let role = if idx == 0 { LayerRole::Structure } else { LayerRole::Custom(name.clone()) };
            LayerSpec { name, role, shape_indices: indices, editability: EditabilityFlags::full(), profile: crate::profiles::ProfileLevel::default() }
        }).collect();
        result.push(LayerSpec {
            name: "other".into(), role: LayerRole::Details,
            shape_indices: other, editability: EditabilityFlags::full(), profile: crate::profiles::ProfileLevel::default(),
        });
        result
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
                profile: crate::profiles::ProfileLevel::default(),
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
                    profile: spec.profile,
                    metadata: {
                        let mut m = serde_json::Map::new();
                        m.insert(
                            "profile".into(),
                            serde_json::json!(spec.profile.as_str()),
                        );
                        m
                    },
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

    /// Remove all shapes whose fill color is within `tolerance` (per channel)
    /// of `color`. Returns the number of shapes removed.
    pub fn remove_color(&mut self, color: Color, tolerance: u8) -> usize {
        let target = color.rgb_u8();
        let tol = tolerance as i32;
        let matches = |shape: &Shape| {
            let [r, g, b] = shape.paint.color().rgb_u8();
            (r as i32 - target[0] as i32).abs() <= tol
                && (g as i32 - target[1] as i32).abs() <= tol
                && (b as i32 - target[2] as i32).abs() <= tol
        };
        let mut removed = 0;
        for layer in &mut self.layers {
            removed += layer.shapes.len() - layer.shapes.iter().filter(|s| !matches(s)).count();
            layer.shapes.retain(|s| !matches(s));
        }
        removed
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
            profile: crate::profiles::ProfileLevel::default(),
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

    #[test]
    fn by_complexity_splits_simple_and_complex() {
        // Create shapes with varying complexity
        let mut doc = VectorDoc::new(100, 100);
        // Simple shape: 4 commands (move, line, line, close)
        doc.shapes.push(test_shape(255));
        doc.shapes.push(test_shape(200));
        // Complex shape: many commands
        let mut complex_sub = SubPath::new();
        for i in 0..20 {
            complex_sub.commands.push(vtracer::ir::PathCmd::MoveTo(PointF64 { x: i as f64, y: 0.0 }));
            complex_sub.commands.push(vtracer::ir::PathCmd::LineTo(PointF64 { x: i as f64, y: 1.0 }));
        }
        doc.shapes.push(Shape {
            paint: vtracer::ir::Paint::Solid(Color::new(100, 100, 100)),
            path: MultiPath { subpaths: vec![complex_sub] },
        });

        let assignment = LayerAssignment::by_complexity(&doc, 50, 3);
        assert!(assignment.layers.len() >= 2);
        // Last layer should be "details"
        assert_eq!(assignment.layers.last().unwrap().name, "details");
    }

    #[test]
    fn hybrid_produces_details_layer() {
        let mut doc = VectorDoc::new(100, 100);
        doc.shapes.push(test_shape(255));
        doc.shapes.push(test_shape(255));
        // Complex shape with different color
        let mut complex_sub = SubPath::new();
        for i in 0..30 {
            complex_sub.commands.push(vtracer::ir::PathCmd::MoveTo(PointF64 { x: i as f64, y: 0.0 }));
            complex_sub.commands.push(vtracer::ir::PathCmd::LineTo(PointF64 { x: i as f64, y: 1.0 }));
        }
        doc.shapes.push(Shape {
            paint: vtracer::ir::Paint::Solid(Color::new(50, 50, 50)),
            path: MultiPath { subpaths: vec![complex_sub] },
        });

        let assignment = LayerAssignment::hybrid(&doc, 4, 50);
        assert!(!assignment.layers.is_empty());
        let has_details = assignment.layers.iter().any(|l| l.name == "details");
        assert!(has_details);
    }

    #[test]
    fn single_layer_strategy_returns_one_layer() {
        let mut doc = VectorDoc::new(10, 10);
        doc.shapes.push(test_shape(255));
        let assignment = LayerAssignment::by_complexity(&doc, 50, 1);
        assert_eq!(assignment.layers.len(), 1);
    }
}