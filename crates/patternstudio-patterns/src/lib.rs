//! PatternStudio pattern generators.
//!
//! Procedural SVG generators for traditional Chinese patterns:
//! rosette (团花), meander (回纹), scroll (卷草), taotie (饕餮), ice_crack (冰裂纹),
//! cloud (云纹), wave (水纹), greek_key (希腊回纹).

/// List all available pattern types.
pub fn list_kinds() -> Vec<(&'static str, &'static str)> {
    vec![
        ("rosette", "团花 - circular floral rosette"),
        ("meander", "回纹 - Greek key / thunder pattern"),
        ("scroll", "卷草 - scrolling vine"),
        ("taotie", "饕餮 - taotie mask"),
        ("ice_crack", "冰裂纹 - ice crack / crazed glaze"),
        ("cloud", "云纹 - auspicious cloud"),
        ("wave", "水纹 - water / wave pattern"),
        ("greek_key", "希腊回纹 - continuous key border"),
    ]
}

/// Generate a pattern SVG (default 400x400).
pub fn generate(
    kind: &str,
    n_units: u32,
    complexity: u8,
) -> Result<String, Box<dyn std::error::Error>> {
    generate_sized(kind, n_units, complexity, 400)
}

/// Generate a pattern SVG with explicit size.
pub fn generate_sized(
    kind: &str,
    n_units: u32,
    complexity: u8,
    size: u32,
) -> Result<String, Box<dyn std::error::Error>> {
    let s = size as f64;
    let svg = match kind {
        "rosette" => rosette(n_units, complexity, s),
        "meander" => meander(n_units, complexity, s),
        "scroll" => scroll(n_units, complexity, s),
        "taotie" => taotie(n_units, complexity, s),
        "ice_crack" => ice_crack(n_units, complexity, s),
        "cloud" => cloud(n_units, complexity, s),
        "wave" => wave(n_units, complexity, s),
        "greek_key" => greek_key(n_units, complexity, s),
        other => {
            let available: Vec<&str> = list_kinds().iter().map(|(k, _)| *k).collect();
            return Err(format!(
                "unknown pattern type: {other}. Available: {}",
                available.join(", ")
            )
            .into());
        }
    };
    Ok(svg)
}

fn svg_wrap(size: f64, paths: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size:.0} {size:.0}\">\n  <g id=\"pattern\" data-role=\"pattern\">\n{paths}  </g>\n</svg>\n"
    )
}

fn rosette(n_units: u32, complexity: u8, size: f64) -> String {
    let center = size / 2.0;
    let radius = size * 0.4;
    let petal_count = n_units.max(4);
    let detail = (complexity as f64 / 100.0) * 3.0 + 1.0;

    let mut paths = String::new();
    for i in 0..petal_count {
        let angle = (2.0 * std::f64::consts::PI * i as f64 / petal_count as f64)
            - std::f64::consts::PI / 2.0;
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
    if complexity > 30 {
        for i in 0..petal_count {
            let angle = (2.0 * std::f64::consts::PI * i as f64 / petal_count as f64)
                - std::f64::consts::PI / 2.0;
            let cx = center + radius * 0.35 * angle.cos();
            let cy = center + radius * 0.35 * angle.sin();
            paths.push_str(&format!(
                "    <circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{:.1}\" fill=\"none\" stroke=\"#666\" stroke-width=\"0.8\"/>\n",
                radius * 0.08
            ));
        }
    }
    svg_wrap(size, &paths)
}

fn meander(n_units: u32, complexity: u8, size: f64) -> String {
    let unit_w = size / n_units as f64;
    let depth = unit_w * 0.7;
    let sw = (complexity as f64 / 100.0) * 3.0 + 1.0;

    let mut paths = String::new();
    for i in 0..n_units {
        let x = i as f64 * unit_w;
        paths.push_str(&format!(
            "    <path d=\"M{:.1} 0 V{:.1} H{:.1} V{:.1} H{:.1} V{:.1}\" \
             fill=\"none\" stroke=\"#333\" stroke-width=\"{:.1}\"/>\n",
            x,
            depth,
            x + unit_w * 0.8,
            depth * 0.6,
            x + unit_w * 0.4,
            depth * 0.3,
            sw
        ));
    }
    svg_wrap(size, &paths)
}

fn scroll(n_units: u32, complexity: u8, size: f64) -> String {
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
    svg_wrap(size, &paths)
}

fn taotie(n_units: u32, complexity: u8, size: f64) -> String {
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
    svg_wrap(size, &paths)
}

fn ice_crack(n_units: u32, complexity: u8, size: f64) -> String {
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
    svg_wrap(size, &paths)
}

fn cloud(n_units: u32, complexity: u8, size: f64) -> String {
    let detail = complexity as f64 / 100.0;
    let mut paths = String::new();
    let count = n_units.max(2) as usize;
    let cell_w = size / count as f64;

    for i in 0..count {
        let cx = (i as f64 + 0.5) * cell_w;
        let cy = size * 0.5;
        let r = cell_w * 0.3;
        paths.push_str(&format!(
            "    <circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.1}\" fill=\"none\" stroke=\"#333\" stroke-width=\"1.5\"/>\n", cx, cy, r
        ));
        paths.push_str(&format!(
            "    <circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.1}\" fill=\"none\" stroke=\"#333\" stroke-width=\"1.5\"/>\n", cx - r * 0.7, cy + r * 0.2, r * 0.7
        ));
        paths.push_str(&format!(
            "    <circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.1}\" fill=\"none\" stroke=\"#333\" stroke-width=\"1.5\"/>\n", cx + r * 0.7, cy + r * 0.2, r * 0.7
        ));
        if detail > 0.3 {
            let tl = r * 1.5;
            paths.push_str(&format!(
                "    <path d=\"M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}\" fill=\"none\" stroke=\"#333\" stroke-width=\"1\"/>\n",
                cx + r, cy, cx + r + tl * 0.5, cy - r * 0.3, cx + r + tl * 0.8, cy + r * 0.2,
                cx + r + tl, cy - r * 0.1, cx + r + tl * 1.2, cy + r * 0.3
            ));
        }
    }
    svg_wrap(size, &paths)
}

fn wave(n_units: u32, complexity: u8, size: f64) -> String {
    let detail = complexity as f64 / 100.0;
    let mut paths = String::new();
    let rows = (n_units / 2).max(2) as usize;
    let row_h = size / rows as f64;
    let wavelength = size / 4.0;

    for row in 0..rows {
        let y_base = (row as f64 + 0.5) * row_h;
        let amplitude = row_h * 0.3 * (0.5 + detail * 0.5);
        let mut d = String::from("    <path d=\"");
        d.push_str(&format!("M0 {:.1}", y_base));
        for i in 0..8 {
            let _x0 = i as f64 * wavelength / 2.0;
            let x1 = (i as f64 + 0.5) * wavelength / 2.0;
            let x2 = (i as f64 + 1.0) * wavelength / 2.0;
            let dir = if i % 2 == 0 { -1.0 } else { 1.0 };
            d.push_str(&format!(
                " Q{:.1} {:.1} {:.1} {:.1}",
                x1,
                y_base + dir * amplitude,
                x2,
                y_base
            ));
        }
        d.push_str("\" fill=\"none\" stroke=\"#333\" stroke-width=\"1.5\"/>\n");
        paths.push_str(&d);
    }
    svg_wrap(size, &paths)
}

fn greek_key(n_units: u32, complexity: u8, size: f64) -> String {
    let _sw = (complexity as f64 / 100.0) * 2.0 + 1.0;
    let unit = size / n_units as f64;
    let inset = unit * 0.25;
    let mut paths = String::new();

    for i in 0..n_units {
        let x = i as f64 * unit;
        paths.push_str(&format!(
            "    <path d=\"M{:.1} 0 V{:.1} H{:.1} V{:.1} H{:.1} V{:.1} H{:.1}\" \
             fill=\"none\" stroke=\"#333\" stroke-width=\"{:.1}\"/>\n",
            x,
            unit - inset,
            x + inset,
            unit - inset * 2.0,
            x + inset * 2.0,
            inset,
            x + unit,
            inset
        ));
    }
    svg_wrap(size, &paths)
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
    fn generate_cloud() {
        let svg = generate("cloud", 4, 70).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("circle"));
    }

    #[test]
    fn generate_wave() {
        let svg = generate("wave", 8, 50).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("Q"));
    }

    #[test]
    fn generate_greek_key() {
        let svg = generate("greek_key", 6, 50).unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn generate_with_custom_size() {
        let svg = generate_sized("rosette", 8, 50, 800).unwrap();
        assert!(svg.contains("viewBox=\"0 0 800 800\""));
    }

    #[test]
    fn list_kinds_has_all() {
        let kinds = list_kinds();
        assert!(kinds.len() >= 8);
        let names: Vec<&str> = kinds.iter().map(|(k, _)| *k).collect();
        assert!(names.contains(&"rosette"));
        assert!(names.contains(&"cloud"));
        assert!(names.contains(&"wave"));
        assert!(names.contains(&"greek_key"));
    }

    #[test]
    fn unknown_pattern_errors() {
        let err = generate("unknown", 4, 50).unwrap_err();
        assert!(err.to_string().contains("Available"));
    }
}
