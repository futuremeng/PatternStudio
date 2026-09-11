//! Layered SVG writer: converts LayeredDoc to SVG string.

use penpaw_core::ir::LayeredDoc;
use vtracer::ir::Shape;
use visioncortex::Color;

pub struct LayeredWriter;

impl LayeredWriter {
    pub fn write(doc: &LayeredDoc) -> String {
        let mut out = String::with_capacity(4096);
        out.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\">\n",
            doc.width, doc.height, doc.width, doc.height
        ));
        out.push_str(&format!(
            "<!-- penpaw:metadata pattern_type={} n_units={} branch={} engine={} -->\n",
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
    use penpaw_core::ir::{EditabilityFlags, LayerAssignment, LayeredDoc};
    use vtracer::ir::{MultiPath, SubPath, VectorDoc};
    use visioncortex::PointF64;

    fn make_test_doc() -> LayeredDoc {
        let mut doc = VectorDoc::new(100, 100);
        let mut sub = SubPath::new();
        sub.commands.push(vtracer::ir::PathCmd::MoveTo(PointF64 { x: 0.0, y: 0.0 }));
        sub.commands.push(vtracer::ir::PathCmd::LineTo(PointF64 { x: 50.0, y: 0.0 }));
        sub.commands.push(vtracer::ir::PathCmd::LineTo(PointF64 { x: 50.0, y: 50.0 }));
        sub.commands.push(vtracer::ir::PathCmd::Close);
        doc.shapes.push(Shape {
            paint: Paint::Solid(Color::new(255, 0, 0)),
            path: MultiPath { subpaths: vec![sub] },
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
}
