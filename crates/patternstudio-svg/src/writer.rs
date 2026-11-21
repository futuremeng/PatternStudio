//! Layered SVG writer: converts LayeredDoc to SVG string.

use patternstudio_core::ir::LayeredDoc;
use visioncortex::Color;
use vtracer::ir::Shape;

pub struct LayeredWriter;

impl LayeredWriter {
    pub fn write(doc: &LayeredDoc) -> String {
        let mut out = String::with_capacity(4096);
        out.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\">\n",
            doc.width, doc.height, doc.width, doc.height
        ));
        out.push_str(&format!(
            "<!-- patternstudio:metadata pattern_type={} n_units={} branch={} engine={} -->\n",
            doc.metadata.pattern_type,
            doc.metadata.n_units,
            doc.metadata.branch,
            doc.metadata.engine
        ));
        for layer in &doc.layers {
            let editable = layer.editability.to_svg_attr();
            out.push_str(&format!(
                "  <g id=\"{}\" data-role=\"{}\" data-editable=\"{}\">\n",
                layer.name, layer.role, editable
            ));
            for shape in &layer.shapes {
                out.push_str(&Self::shape_to_svg(shape));
            }
            out.push_str("  </g>\n");
        }
        out.push_str("</svg>\n");
        out
    }

    /// Write SVG in line-art mode: paths use stroke instead of fill.
    pub fn write_line_art(doc: &LayeredDoc) -> String {
        let mut out = String::with_capacity(4096);
        out.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\">\n",
            doc.width, doc.height, doc.width, doc.height
        ));
        for layer in &doc.layers {
            out.push_str(&format!(
                "  <g id=\"{}\" data-role=\"{}\">\n",
                layer.name, layer.role
            ));
            for shape in &layer.shapes {
                let d = Self::path_to_d(&shape.path);
                out.push_str(&format!(
                    "    <path d=\"{}\" fill=\"none\" stroke=\"#000000\" stroke-width=\"1.5\"/>\n",
                    d
                ));
            }
            out.push_str("  </g>\n");
        }
        out.push_str("</svg>\n");
        out
    }

    /// Write SVG containing only the specified layers (matched by name).
    pub fn write_selected_layers(doc: &LayeredDoc, layer_names: &[String]) -> String {
        let mut out = String::with_capacity(4096);
        out.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\">\n",
            doc.width, doc.height, doc.width, doc.height
        ));
        for layer in &doc.layers {
            if layer_names.iter().any(|n| n == &layer.name) {
                let editable = layer.editability.to_svg_attr();
                out.push_str(&format!(
                    "  <g id=\"{}\" data-role=\"{}\" data-editable=\"{}\">\n",
                    layer.name, layer.role, editable
                ));
                for shape in &layer.shapes {
                    out.push_str(&Self::shape_to_svg(shape));
                }
                out.push_str("  </g>\n");
            }
        }
        out.push_str("</svg>\n");
        out
    }

    /// Generate a small preview SVG for a single layer (for layer cards).
    pub fn layer_preview_svg(doc: &LayeredDoc, layer_index: usize) -> String {
        let layer = &doc.layers[layer_index];
        let mut out = String::with_capacity(1024);
        out.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\">\n",
            doc.width, doc.height
        ));
        for shape in &layer.shapes {
            out.push_str(&Self::shape_to_svg(shape));
        }
        out.push_str("</svg>\n");
        out
    }

    /// Get the dominant (most common) fill color of a layer as a hex string.
    pub fn layer_dominant_color(layer: &patternstudio_core::ir::Layer) -> String {
        use std::collections::HashMap;
        let mut colors: HashMap<[u8; 3], usize> = HashMap::new();
        for shape in &layer.shapes {
            let [r, g, b] = shape.paint.color().rgb_u8();
            *colors.entry([r, g, b]).or_insert(0) += 1;
        }
        colors
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|([r, g, b], _)| format!("#{:02x}{:02x}{:02x}", r, g, b))
            .unwrap_or_else(|| "#000000".into())
    }

    /// Write a symmetry-assembled SVG (QWP7): the traced sector lives once
    /// in `<defs>`, and the artwork is rebuilt by `<use>` copies rotated
    /// about the symmetry center (plus a mirror fold when planned), so an
    /// edit to the sector syncs every copy and the symmetry is exact.
    pub fn write_symmetric(
        doc: &LayeredDoc,
        plan: &patternstudio_core::assembly::AssemblyPlan,
    ) -> String {
        let mut out = String::with_capacity(4096);
        let (cx, cy) = plan.center;
        out.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\">\n",
            doc.width, doc.height, doc.width, doc.height
        ));
        out.push_str(&format!(
            "<!-- patternstudio:symmetry kind={} n={} mirror={} angle={:.1} center={:.1},{:.1} -->\n",
            plan.kind(),
            plan.n,
            plan.mirror,
            plan.base_deg,
            cx,
            cy
        ));
        out.push_str("  <defs>\n    <g id=\"sector-0\">\n");
        for layer in &doc.layers {
            for shape in &layer.shapes {
                out.push_str(&Self::shape_to_svg(shape));
            }
        }
        out.push_str("    </g>\n  </defs>\n");
        let editable = doc
            .layers
            .first()
            .map(|l| l.editability.to_svg_attr())
            .unwrap_or_else(|| "none".to_string());
        out.push_str(&format!(
            "  <g id=\"symmetry-0\" data-role=\"pattern\" data-editable=\"{editable}\">\n"
        ));
        // Reflection across the axis at base_deg through the center: move the
        // center to the origin, counter-rotate, flip y, rotate back, restore.
        let mirror_tf = format!(
            "translate({cx:.2} {cy:.2}) rotate({:.2}) matrix(1 0 0 -1 0 0) rotate({:.2}) translate({:.2} {:.2})",
            plan.base_deg,
            -plan.base_deg,
            -cx,
            -cy
        );
        let n = plan.n.max(1);
        for m in 0..n {
            let ang = 360.0 * m as f64 / n as f64;
            if ang == 0.0 && !plan.mirror {
                out.push_str("    <g>\n");
            } else {
                out.push_str(&format!(
                    "    <g transform=\"rotate({ang:.2} {cx:.2} {cy:.2})\">\n"
                ));
            }
            out.push_str("      <use href=\"#sector-0\"/>\n");
            if plan.mirror {
                out.push_str(&format!(
                    "      <use href=\"#sector-0\" transform=\"{mirror_tf}\"/>\n"
                ));
            }
            out.push_str("    </g>\n");
        }
        out.push_str("  </g>\n</svg>\n");
        out
    }

    fn shape_to_svg(shape: &Shape) -> String {
        let fill = Self::color_to_hex(&shape.paint.color());
        let d = Self::path_to_d(&shape.path);
        format!("    <path d=\"{}\" fill=\"{}\"/>\n", d, fill)
    }

    fn path_to_d(path: &vtracer::ir::MultiPath) -> String {
        let mut d = String::new();
        for subpath in &path.subpaths {
            for cmd in &subpath.commands {
                match cmd {
                    vtracer::ir::PathCmd::MoveTo(p) => {
                        d.push_str(&format!("M{:.2} {:.2}", p.x, p.y));
                    }
                    vtracer::ir::PathCmd::LineTo(p) => {
                        d.push_str(&format!("L{:.2} {:.2}", p.x, p.y));
                    }
                    vtracer::ir::PathCmd::CubicTo(c1, c2, p) => {
                        d.push_str(&format!(
                            "C{:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                            c1.x, c1.y, c2.x, c2.y, p.x, p.y
                        ));
                    }
                    vtracer::ir::PathCmd::Close => {
                        d.push('Z');
                    }
                }
            }
        }
        d
    }

    fn color_to_hex(color: &Color) -> String {
        let [r, g, b] = color.rgb_u8();
        format!("#{:02x}{:02x}{:02x}", r, g, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patternstudio_core::ir::{LayerAssignment, LayeredDoc};
    use visioncortex::PointF64;
    use vtracer::ir::{MultiPath, Paint, SubPath, VectorDoc};

    fn make_test_doc() -> LayeredDoc {
        let mut doc = VectorDoc::new(100, 100);
        let mut sub = SubPath::new();
        sub.commands
            .push(vtracer::ir::PathCmd::MoveTo(PointF64 { x: 0.0, y: 0.0 }));
        sub.commands
            .push(vtracer::ir::PathCmd::LineTo(PointF64 { x: 50.0, y: 0.0 }));
        sub.commands
            .push(vtracer::ir::PathCmd::LineTo(PointF64 { x: 50.0, y: 50.0 }));
        sub.commands.push(vtracer::ir::PathCmd::Close);
        doc.shapes.push(Shape {
            paint: Paint::Solid(Color::new(255, 0, 0)),
            path: MultiPath {
                subpaths: vec![sub],
            },
        });
        LayeredDoc::from_vector_doc(&doc, &LayerAssignment::single_layer())
    }

    #[test]
    fn write_produces_valid_svg() {
        let doc = make_test_doc();
        let svg = LayeredWriter::write(&doc);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("viewBox="));
        assert!(svg.contains("<g id="));
        assert!(svg.contains("fill=\"#ff0000\""));
        assert!(svg.ends_with("</svg>\n"));
    }

    #[test]
    fn write_symmetric_emits_defs_and_copies() {
        let doc = make_test_doc();
        let plan = patternstudio_core::assembly::AssemblyPlan {
            center: (50.0, 50.0),
            n: 4,
            mirror: true,
            base_deg: 90.0,
            fold_f1: 1.0,
        };
        let svg = LayeredWriter::write_symmetric(&doc, &plan);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("<defs>"));
        assert!(svg.contains("<g id=\"sector-0\">"));
        assert_eq!(svg.matches("<use").count(), 8, "4 rotations x (1 + mirror)");
        assert!(svg.contains("rotate(90.00 50.00 50.00)"));
        assert!(svg.contains("matrix(1 0 0 -1 0 0)"));
        assert!(
            svg.contains("translate(-50.00 -50.00)"),
            "mirror chain must end by moving the center back"
        );
        assert!(svg.ends_with("</svg>\n"));
    }

    #[test]
    fn write_symmetric_cyclic_has_no_mirror_use() {
        let doc = make_test_doc();
        let plan = patternstudio_core::assembly::AssemblyPlan {
            center: (50.0, 50.0),
            n: 3,
            mirror: false,
            base_deg: 0.0,
            fold_f1: 1.0,
        };
        let svg = LayeredWriter::write_symmetric(&doc, &plan);
        assert_eq!(svg.matches("<use").count(), 3);
        assert!(!svg.contains("matrix(1 0 0 -1 0 0)"));
    }

    /// A layer holds a *subset* of the shapes, not a crop of the artwork: every
    /// SVG a separated layer produces must stay on the full document canvas,
    /// whatever corner of it the shapes happen to occupy.
    fn corner_shape_doc() -> LayeredDoc {
        let mut doc = VectorDoc::new(1000, 400);
        for i in 0..2 {
            let mut sub = SubPath::new();
            sub.commands
                .push(vtracer::ir::PathCmd::MoveTo(PointF64 { x: 10.0, y: 10.0 }));
            sub.commands.push(vtracer::ir::PathCmd::LineTo(PointF64 {
                x: 10.0 + 20.0 * i as f64,
                y: 10.0,
            }));
            sub.commands
                .push(vtracer::ir::PathCmd::LineTo(PointF64 { x: 10.0, y: 30.0 }));
            sub.commands.push(vtracer::ir::PathCmd::Close);
            doc.shapes.push(Shape {
                paint: Paint::Solid(Color::new(0, 0, 0)),
                path: MultiPath {
                    subpaths: vec![sub],
                },
            });
        }
        LayeredDoc::from_vector_doc(&doc, &LayerAssignment::single_layer())
    }

    #[test]
    fn layer_preview_keeps_the_document_canvas() {
        let doc = corner_shape_doc();
        let svg = LayeredWriter::layer_preview_svg(&doc, 0);
        assert!(
            svg.contains("viewBox=\"0 0 1000 400\""),
            "layer preview must not shrink to the shapes' bbox: {svg}"
        );
    }

    #[test]
    fn selected_layer_export_keeps_the_document_canvas() {
        let doc = corner_shape_doc();
        let svg = LayeredWriter::write_selected_layers(&doc, &["artwork".to_string()]);
        assert!(
            svg.contains("viewBox=\"0 0 1000 400\" width=\"1000\" height=\"400\""),
            "selected-layer export must not shrink to the shapes' bbox: {svg}"
        );
    }
}
