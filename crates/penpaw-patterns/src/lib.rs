//! PatternStudio pattern generators.
//!
//! Procedural SVG generators for traditional Chinese patterns:
//! rosette (团花), meander (回纹), scroll (卷草), taotie (饕餮), ice_crack (冰裂纹).

/// Generate a pattern SVG.
pub fn generate(kind: &str, n_units: u32, complexity: u8) -> Result<String, Box<dyn std::error::Error>> {
    let svg = match kind {
        "rosette" => rosette(n_units, complexity),
        "meander" => meander(n_units, complexity),
        "scroll" => scroll(n_units, complexity),
        "taotie" => taotie(n_units, complexity),
        "ice_crack" => ice_crack(n_units, complexity),
        other => return Err(format!("unknown pattern type: {other}").into()),
    };
    Ok(svg)
}

fn rosette(n_units: u32, complexity: u8) -> String {
    let size = 400.0;
    let center = size / 2.0;
    let radius = size * 0.4;
    let petal_count = n_units.max(4);
    let detail = (complexity as f64 / 100.0) * 3.0 + 1.0;

    let mut paths = String::new();
    for i in 0..petal_count {
        let angle = (2.0 * std::f64::consts::PI * i as f64 / petal_count as f64) - std::f64::consts::PI / 2.0;
        let cx = center + radius * 0.6 * angle.cos();
        let cy = center + radius * 0.6 * angle.sin();
        let petal_r = radius * 0.35 * (0.8 + 0.2 * detail / 4.0);
        paths.push_str(&format!(
            "    <ellipse cx=\"{cx:.1}\" cy=\"{cy:.1}\" rx=\"{:.1}\" ry=\"{:.1}\" \
             transform=\"rotate({:.1} {cx:.1} {cy:.1})\" fill=\"none\" stroke=\"#333\" stroke-width=\"1.5\"/>\n",
            petal_r, petal_r * 0.6, angle.to_degrees()
        ));
    }
    paths.push_str(&format!(
        "    <circle cx=\"{center}\" cy=\"{center}\" r=\"{:.1}\" fill=\"none\" stroke=\"#333\" stroke-width=\"2\"/>\n",
        radius * 0.2
    ));

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size:.0} {size:.0}\">\n  <g id=\"pattern\" data-role=\"pattern\">\n{paths}  </g>\n</svg>\n"
    )
}

fn meander(n_units: u32, complexity: u8) -> String {
    let size = 400.0;
    let unit_w = size / n_units as f64;
    let depth = unit_w * 0.7;
    let sw = (complexity as f64 / 100.0) * 3.0 + 1.0;

    let mut paths = String::new();
    for i in 0..n_units {
        let x = i as f64 * unit_w;
        paths.push_str(&format!(
            "    <path d=\"M{:.1} 0 V{:.1} H{:.1} V{:.1} H{:.1} V{:.1}\" \
             fill=\"none\" stroke=\"#333\" stroke-width=\"{:.1}\"/>\n",
            x, depth, x + unit_w * 0.8, depth * 0.6, x + unit_w * 0.4, depth * 0.3, sw
        ));
    }

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size:.0} {size:.0}\">\n  <g id=\"pattern\" data-role=\"pattern\">\n{paths}  </g>\n</svg>\n"
    )
}

fn scroll(n_units: u32, complexity: u8) -> String {
    let size = 400.0;
    let _detail = complexity as f64 / 100.0;
    let mut paths = String::new();
    let segments = n_units.max(2) as usize;
    let seg_w = size / segments as f64;
    for i in 0..segments {
        let x0 = i as f64 * seg_w;
        let x1 = x0 + seg_w;
        let mid = (x0 + x1) / 2.0;
        let y = size / 2.0;
        paths.push_str(&format!(
            "    <path d=\"M{:.1} {y:.1} C{:.1} {y:.1} {:.1} {y:.1} {:.1} {y:.1}\" fill=\"none\" stroke=\"#333\" stroke-width=\"2\"/>\n",
            x0, x0 + seg_w * 0.3, x1 - seg_w * 0.3, x1
        ));
        paths.push_str(&format!(
            "    <ellipse cx=\"{mid:.1}\" cy=\"{y:.1}\" rx=\"{:.1}\" ry=\"{:.1}\" fill=\"none\" stroke=\"#333\" stroke-width=\"1\"/>\n",
            seg_w * 0.15, seg_w * 0.08
        ));
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size:.0} {size:.0}\">\n  <g id=\"pattern\" data-role=\"pattern\">\n{paths}  </g>\n</svg>\n"
    )
}

fn taotie(n_units: u32, complexity: u8) -> String {
    let size = 400.0;
    let center = size / 2.0;
    let _detail = complexity as f64 / 100.0;
    let _units = n_units;
    let mut paths = String::new();
    let eye_r = size * 0.08;
    let eye_dx = size * 0.15;
    paths.push_str(&format!(
        "    <circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.1}\" fill=\"none\" stroke=\"#333\" stroke-width=\"2\"/>\n",
        center - eye_dx, center - size * 0.05, eye_r
    ));
    paths.push_str(&format!(
        "    <circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.1}\" fill=\"none\" stroke=\"#333\" stroke-width=\"2\"/>\n",
        center + eye_dx, center - size * 0.05, eye_r
    ));
    paths.push_str(&format!(
        "    <path d=\"M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}\" fill=\"none\" stroke=\"#333\" stroke-width=\"2\"/>\n",
        center - size * 0.1, center + size * 0.1,
        center, center + size * 0.2,
        center + size * 0.1, center + size * 0.1
    ));
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size:.0} {size:.0}\">\n  <g id=\"pattern\" data-role=\"pattern\">\n{paths}  </g>\n</svg>\n"
    )
}

fn ice_crack(n_units: u32, complexity: u8) -> String {
    let size = 400.0;
    let _detail = complexity as f64 / 100.0;
    let cols = (n_units.max(2) as f64).sqrt().ceil() as u32;
    let cell = size / cols as f64;
    let mut paths = String::new();
    for row in 0..cols {
        for col in 0..cols {
            let x = col as f64 * cell;
            let y = row as f64 * cell;
            let jitter = cell * 0.15;
            let dx1 = jitter * (((col * 7 + row * 3) % 5) as f64 / 5.0 - 0.5);
            let dy1 = jitter * (((col * 3 + row * 7) % 5) as f64 / 5.0 - 0.5);
            let dx2 = jitter * (((col * 5 + row * 2) % 5) as f64 / 5.0 - 0.5);
            let dy2 = jitter * (((col * 2 + row * 5) % 5) as f64 / 5.0 - 0.5);
            paths.push_str(&format!(
                "    <path d=\"M{:.1} {:.1} L{:.1} {:.1} L{:.1} {:.1} L{:.1} {:.1} Z\" fill=\"none\" stroke=\"#333\" stroke-width=\"1\"/>\n",
                x + dx1, y + dy1, x + cell + dx2, y + dy2,
                x + cell - dx1, y + cell - dy2, x - dx2, y + cell - dy1
            ));
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size:.0} {size:.0}\">\n  <g id=\"pattern\" data-role=\"pattern\">\n{paths}  </g>\n</svg>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_rosette() {
        let svg = generate("rosette", 8, 50).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("data-role=\"pattern\""));
    }

    #[test]
    fn generate_meander() {
        let svg = generate("meander", 6, 50).unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn generate_ice_crack() {
        let svg = generate("ice_crack", 9, 50).unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn unknown_pattern_errors() {
        assert!(generate("unknown", 4, 50).is_err());
    }
}