//! Multi-level layering strategy.
//!
//! The user's "ideal" trace parameter combination is treated as an anchor:
//! it declares how much color, detail, and noise tolerance the image is
//! worth. From that anchor we derive
//!
//! 1. an **intent profile** ([`IdealParams`] → detail budget),
//! 2. an **automatic layer strategy decision** ([`AutoDecision`]: which
//!    strategy, how many layers, which complexity threshold, and why), and
//! 3. **three parameter profiles** ([`ProfileLevel`] L0/L1/L2) applied per
//!    layer: the background layer is coarsened, middle layers keep the ideal
//!    result verbatim, and the details layer is refined.
//!
//! Re-fitting works on the already-traced geometry (topology is preserved,
//! so the combined result stays visually identical to the ideal single-layer
//! trace): L0 coarsens cubic chains with a larger simplification tolerance,
//! L2 keeps full coordinate precision and removes redundant segments only.

use serde::{Deserialize, Serialize};
use vtracer::fitter::FittedGeom;
use vtracer::ir::{MultiPath, PathCmd, SubPath, VectorDoc};
use vtracer::optimize::{CleanupPass, OptimizerPass, QuantizePass};
use vtracer::simplify::{CurvePass, SimplifyCurves};
use visioncortex::PointF64;

use crate::ir::{LayerAssignment, LayeredDoc, LayerRole};

/// The user's ideal parameter combination — the quality anchor for auto
/// layering. Defaults mirror the web editor's `DEFAULT_PARAMS`.
#[derive(Debug, Clone, Copy)]
pub struct IdealParams {
    /// "binary" | "color" | "watershed"
    pub clustering: &'static str,
    pub line_art: bool,
    /// 1..=8
    pub color_precision: i32,
    /// 0..=128
    pub layer_difference: i32,
    pub max_colors: Option<usize>,
    /// 0..=0.25
    pub simplify: f64,
    /// 0..=6
    pub path_precision: i32,
    /// 0..=180 (degrees)
    pub corner_threshold: i32,
    /// 0..=20 (px)
    pub length_threshold: f64,
    /// 0..=20
    pub filter_speckle: i32,
    /// 0..=2
    pub optimize: i32,
}

impl Default for IdealParams {
    fn default() -> Self {
        Self {
            clustering: "color",
            line_art: false,
            color_precision: 6,
            layer_difference: 16,
            max_colors: None,
            simplify: 0.1,
            path_precision: 2,
            corner_threshold: 60,
            length_threshold: 4.0,
            filter_speckle: 4,
            optimize: 1,
        }
    }
}

impl IdealParams {
    /// Build from the raw (optional) trace params, falling back to the web
    /// editor defaults for anything the caller did not set.
    #[allow(clippy::too_many_arguments)]
    pub fn from_options(
        clustering: Option<&str>,
        line_art: bool,
        color_precision: Option<u8>,
        layer_difference: Option<u8>,
        max_colors: Option<usize>,
        simplify: Option<f64>,
        path_precision: Option<u8>,
        corner_threshold: Option<i32>,
        length_threshold: Option<f64>,
        filter_speckle: Option<u32>,
        optimize: Option<u8>,
    ) -> Self {
        let d = Self::default();
        Self {
            clustering: match clustering {
                Some("binary") => "binary",
                Some("watershed") => "watershed",
                _ => d.clustering,
            },
            line_art,
            color_precision: color_precision.map(|v| v as i32).unwrap_or(d.color_precision),
            layer_difference: layer_difference.map(|v| v as i32).unwrap_or(d.layer_difference),
            max_colors,
            simplify: simplify.unwrap_or(d.simplify),
            path_precision: path_precision.map(|v| v as i32).unwrap_or(d.path_precision),
            corner_threshold: corner_threshold.unwrap_or(d.corner_threshold),
            length_threshold: length_threshold.unwrap_or(d.length_threshold),
            filter_speckle: filter_speckle.map(|v| v as i32).unwrap_or(d.filter_speckle),
            optimize: optimize.map(|v| v as i32).unwrap_or(d.optimize),
        }
    }

    /// Normalized detail budget in `[0, 100]`: how much geometric detail the
    /// ideal combination asks for. Low simplify / high path precision /
    /// preserved corners push it up; speckle tolerance and short-segment
    /// filtering push it down.
    pub fn detail_budget(&self) -> i32 {
        let mut b = 50.0f64;
        b += (2.0 - (self.simplify / 0.25).min(2.0)) * 15.0;
        b += (self.path_precision - 2) as f64 * 5.0;
        b += (60 - self.corner_threshold) as f64 * 0.3;
        b -= self.filter_speckle as f64 * 2.0;
        b -= (20.0 - self.length_threshold.min(20.0)) * 1.5;
        b.clamp(0.0, 100.0) as i32
    }
}

/// Per-level parameter profile for one layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileLevel {
    /// Coarse: background / base layer — simplified, quantized.
    L0,
    /// Standard: the ideal result, kept verbatim.
    L1,
    /// Fine: details layer — full precision, redundant segments removed.
    L2,
}

impl Default for ProfileLevel {
    fn default() -> Self {
        ProfileLevel::L1
    }
}

impl ProfileLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProfileLevel::L0 => "L0",
            ProfileLevel::L1 => "L1",
            ProfileLevel::L2 => "L2",
        }
    }

    /// Profile for a layer role: background is coarsened, details are
    /// refined, everything else stays at the ideal level.
    pub fn for_role(role: &LayerRole) -> Self {
        match role {
            LayerRole::Background => ProfileLevel::L0,
            LayerRole::Details => ProfileLevel::L2,
            _ => ProfileLevel::L1,
        }
    }
}

/// Concrete re-fit parameters for one profile level, derived from the ideal
/// anchor.
#[derive(Debug, Clone, Copy)]
pub struct LayerProfile {
    pub level: ProfileLevel,
    /// SimplifyCurves tolerance (px); 0 = no simplification.
    pub simplify_tolerance: f64,
    /// SimplifyCurves corner threshold (radians).
    pub corner_threshold: f64,
    /// QuantizePass decimal precision.
    pub quantize_precision: u32,
}

impl LayerProfile {
    /// The three-level cascade anchored on the ideal combination:
    /// L0 coarser than ideal, L1 = ideal, L2 finer than ideal.
    pub fn derive(ideal: &IdealParams) -> [LayerProfile; 3] {
        let rad = |deg: i32| deg as f64 * std::f64::consts::PI / 180.0;
        [
            LayerProfile {
                level: ProfileLevel::L0,
                simplify_tolerance: (ideal.simplify + 0.1).min(0.25),
                corner_threshold: rad(ideal.corner_threshold + 20),
                quantize_precision: ideal.path_precision.clamp(0, 2) as u32,
            },
            LayerProfile {
                level: ProfileLevel::L1,
                simplify_tolerance: 0.0,
                corner_threshold: rad(ideal.corner_threshold),
                quantize_precision: ideal.path_precision.clamp(0, 6) as u32,
            },
            LayerProfile {
                level: ProfileLevel::L2,
                simplify_tolerance: 0.0,
                corner_threshold: rad(ideal.corner_threshold.saturating_sub(20)),
                quantize_precision: (ideal.path_precision + 1).clamp(0, 6) as u32,
            },
        ]
    }
}


/// Outcome of the automatic strategy decision, including the human-readable
/// reason (surfaced in the UI and layer metadata).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoDecision {
    /// "none" | "color" | "complexity" | "hybrid"
    pub strategy: String,
    pub layers: usize,
    pub complexity_threshold: u8,
    pub reason: String,
}

impl AutoDecision {
    /// Decision table over the traced doc's distinct color count `c` and the
    /// ideal combination's detail budget.
    pub fn decide(c: usize, ideal: &IdealParams) -> Self {
        let budget = ideal.detail_budget();
        let binary = ideal.clustering == "binary" || ideal.line_art;
        let threshold = (70 + (40 - budget) as i32 / 2).clamp(10, 90) as u8;

        if binary {
            if budget >= 70 {
                return Self {
                    strategy: "complexity".into(),
                    layers: 4,
                    complexity_threshold: threshold,
                    reason: format!("二值/线稿 + 高细节预算({budget}) → 复杂度分层"),
                };
            }
            return Self {
                strategy: "none".into(),
                layers: 1,
                complexity_threshold: threshold,
                reason: format!("二值/线稿 + 细节预算 {budget} → 单层"),
            };
        }

        if c <= 1 {
            return Self {
                strategy: "none".into(),
                layers: 1,
                complexity_threshold: threshold,
                reason: format!("仅 {c} 色 → 单层"),
            };
        }

        if c <= 4 {
            if budget >= 40 {
                return Self {
                    strategy: "hybrid".into(),
                    layers: (c + 1).min(8),
                    complexity_threshold: threshold,
                    reason: format!("{c} 色 + 细节预算 {budget} ≥ 40 → 混合分层"),
                };
            }
            return Self {
                strategy: "color".into(),
                layers: c,
                complexity_threshold: threshold,
                reason: format!("{c} 色 + 细节预算 {budget} < 40 → 颜色分层"),
            };
        }

        if budget >= 60 {
            let layers = (c / 2 + 1).min(8);
            Self {
                strategy: "hybrid".into(),
                layers,
                complexity_threshold: threshold,
                reason: format!("{c} 色 + 细节预算 {budget} ≥ 60 → 混合分层（颜色预算减半）"),
            }
        } else {
            Self {
                strategy: "complexity".into(),
                layers: 4,
                complexity_threshold: threshold,
                reason: format!("{c} 色 + 细节预算 {budget} < 60 → 复杂度分层"),
            }
        }
    }

    /// Count distinct fill colors in a traced doc.
    pub fn distinct_colors(doc: &VectorDoc) -> usize {
        let mut colors = std::collections::HashSet::new();
        for shape in &doc.shapes {
            colors.insert(shape.paint.color().rgb_u8());
        }
        colors.len()
    }

    /// Full auto path: decide from doc statistics, then build the assignment.
    pub fn assign(doc: &VectorDoc, ideal: &IdealParams) -> (LayerAssignment, Self) {
        let c = Self::distinct_colors(doc);
        let decision = Self::decide(c, ideal);
        let assignment = Self::assignment_for(doc, &decision);
        (assignment, decision)
    }

    /// Build the layer assignment for a decision (role-based profile levels
    /// are stamped on each layer spec).
    pub fn assignment_for(doc: &VectorDoc, decision: &AutoDecision) -> LayerAssignment {
        let mut assignment = match decision.strategy.as_str() {
            "color" => LayerAssignment::by_color(doc, decision.layers.max(2)),
            "complexity" => LayerAssignment::by_complexity(
                doc,
                decision.complexity_threshold,
                decision.layers.max(2),
            ),
            "hybrid" => {
                LayerAssignment::hybrid(doc, decision.layers.max(2), decision.complexity_threshold)
            }
            _ => LayerAssignment::single_layer(),
        };
        for spec in &mut assignment.layers {
            spec.profile = ProfileLevel::for_role(&spec.role);
        }
        assignment
    }
}


/// Apply the per-layer parameter profiles to a layered doc, in place.
///
/// L0 layers are coarsened (larger simplification tolerance + coarser
/// quantization), L2 layers are refined (finer quantization + cleanup), L1
/// layers are left untouched. Topology is never changed, so stacking all
/// layers still reproduces the ideal result.
pub fn apply_profiles(layered: &mut LayeredDoc, ideal: &IdealParams) {
    let profiles = LayerProfile::derive(ideal);
    for layer in &mut layered.layers {
        // Derive the level from the role so this also works for layers built
        // outside the auto strategy (which leave the spec profile at L1).
        let level = ProfileLevel::for_role(&layer.role);
        layer.profile = level;
        let profile = profiles[level as usize];
        for shape in &mut layer.shapes {
            refine_shape(shape, &profile);
        }
        layer
            .metadata
            .insert("profile".into(), serde_json::json!(level.as_str()));
    }
}

fn refine_shape(shape: &mut vtracer::ir::Shape, profile: &LayerProfile) {
    if profile.level == ProfileLevel::L1 {
        return;
    }
    let mut subpaths: Vec<SubPath> = Vec::with_capacity(shape.path.subpaths.len());
    for sub in std::mem::take(&mut shape.path.subpaths) {
        subpaths.push(refit_subpath(sub, profile));
    }
    shape.path = MultiPath { subpaths };

    let mut doc = VectorDoc::new(0, 0);
    doc.shapes.push(shape.clone());
    QuantizePass::new(profile.quantize_precision).run(&mut doc);
    if profile.level == ProfileLevel::L2 {
        CleanupPass.run(&mut doc);
    }
    *shape = doc.shapes.pop().expect("shape survives optimizer passes");
}

/// Re-fit one subpath through the level's curve pass.
fn refit_subpath(sub: SubPath, profile: &LayerProfile) -> SubPath {
    let (start, chain, closed) = subpath_to_chain(sub);
    if chain.is_empty() {
        return rebuild_passthrough(start, closed);
    }
    let pass = SimplifyCurves {
        tolerance: profile.simplify_tolerance,
        corner_threshold: profile.corner_threshold,
    };
    let geom = if closed {
        pass.ring(FittedGeom::Beziers(chain))
    } else {
        pass.open(FittedGeom::Beziers(chain))
    };
    match geom {
        FittedGeom::Beziers(out) => beziers_to_subpath(out, closed),
        FittedGeom::Polyline(points) => polyline_to_subpath(points),
    }
}

/// Convert a subpath into a cubic chain. Line segments become degenerate
/// cubics; a subpath is "closed" when it ends with `Close` or returns to its
/// start point.
fn subpath_to_chain(sub: SubPath) -> (Option<PointF64>, Vec<[PointF64; 4]>, bool) {
    let mut start: Option<PointF64> = None;
    let mut current: Option<PointF64> = None;
    let mut chain: Vec<[PointF64; 4]> = Vec::new();
    let mut has_cubic = false;
    let mut closed = false;

    for cmd in &sub.commands {
        match cmd {
            PathCmd::MoveTo(p) => {
                if start.is_none() {
                    start = Some(*p);
                }
                current = Some(*p);
            }
            PathCmd::LineTo(p) => {
                if let Some(from) = current {
                    chain.push([from, from, *p, *p]);
                }
                current = Some(*p);
            }
            PathCmd::CubicTo(c1, c2, e) => {
                if let Some(from) = current {
                    chain.push([from, *c1, *c2, *e]);
                    has_cubic = true;
                }
                current = Some(*e);
            }
            PathCmd::Close => {
                closed = true;
                if let (Some(from), Some(s)) = (current, start) {
                    if from != s {
                        chain.push([from, from, s, s]);
                    }
                }
                current = start;
            }
        }
    }
    if !closed {
        if let (Some(from), Some(s)) = (current, start) {
            if has_cubic && from == s {
                closed = true;
            }
        }
    }
    (start, chain, closed)
}

fn rebuild_passthrough(start: Option<PointF64>, closed: bool) -> SubPath {
    let mut sub = SubPath::new();
    if let Some(p) = start {
        sub.commands.push(PathCmd::MoveTo(p));
    }
    if closed {
        sub.commands.push(PathCmd::Close);
    }
    sub
}

fn beziers_to_subpath(chain: Vec<[PointF64; 4]>, closed: bool) -> SubPath {
    let mut sub = SubPath::new();
    if chain.is_empty() {
        return sub;
    }
    sub.commands.push(PathCmd::MoveTo(chain[0][0]));
    for c in &chain {
        sub.commands.push(PathCmd::CubicTo(c[1], c[2], c[3]));
    }
    if closed {
        sub.commands.push(PathCmd::Close);
    }
    sub
}

fn polyline_to_subpath(points: Vec<PointF64>) -> SubPath {
    let mut sub = SubPath::new();
    if let Some(first) = points.first() {
        sub.commands.push(PathCmd::MoveTo(*first));
        for p in points.iter().skip(1) {
            sub.commands.push(PathCmd::LineTo(*p));
        }
    }
    sub
}


#[cfg(test)]
mod tests {
    use super::*;

    fn ideal() -> IdealParams {
        IdealParams::default()
    }

    fn cubic_shape() -> vtracer::ir::Shape {
        // A rounded square as 16 collinear cubic segments (4 per side).
        let mut sub = SubPath::new();
        sub.commands.push(PathCmd::MoveTo(PointF64 { x: 0.0, y: 0.0 }));
        for (x, y) in [
            (10.0, 0.0),
            (20.0, 0.0),
            (30.0, 0.0),
            (40.0, 0.0),
            (40.0, 10.0),
            (40.0, 20.0),
            (40.0, 30.0),
            (40.0, 40.0),
            (30.0, 40.0),
            (20.0, 40.0),
            (10.0, 40.0),
            (0.0, 40.0),
            (0.0, 30.0),
            (0.0, 20.0),
            (0.0, 10.0),
            (0.0, 0.0),
        ] {
            sub.commands.push(PathCmd::CubicTo(
                PointF64 { x, y },
                PointF64 { x, y },
                PointF64 { x, y },
            ));
        }
        sub.commands.push(PathCmd::Close);
        vtracer::ir::Shape {
            paint: vtracer::ir::Paint::Solid(visioncortex::Color::new(10, 20, 30)),
            path: MultiPath { subpaths: vec![sub] },
        }
    }

    fn cmd_count(shape: &vtracer::ir::Shape) -> usize {
        shape.path.subpaths.iter().map(|s| s.commands.len()).sum()
    }

    fn detail_hungry() -> IdealParams {
        IdealParams {
            simplify: 0.0,
            path_precision: 6,
            corner_threshold: 0,
            length_threshold: 0.0,
            filter_speckle: 0,
            ..ideal()
        }
    }

    fn coarse() -> IdealParams {
        IdealParams {
            simplify: 0.25,
            path_precision: 0,
            corner_threshold: 180,
            length_threshold: 20.0,
            filter_speckle: 20,
            ..ideal()
        }
    }

    #[test]
    fn detail_budget_range_and_direction() {
        let b_detail = detail_hungry().detail_budget();
        let b_coarse = coarse().detail_budget();
        assert!((0..=100).contains(&b_detail));
        assert!((0..=100).contains(&b_coarse));
        assert!(b_detail > b_coarse, "{b_detail} should exceed {b_coarse}");
    }

    #[test]
    fn binary_low_budget_is_single_layer() {
        let p = IdealParams {
            clustering: "binary",
            filter_speckle: 10,
            simplify: 0.2,
            ..ideal()
        };
        let d = AutoDecision::decide(2, &p);
        assert_eq!(d.strategy, "none");
    }

    #[test]
    fn few_colors_low_budget_is_color() {
        let p = IdealParams {
            filter_speckle: 12,
            simplify: 0.2,
            path_precision: 1,
            ..ideal()
        };
        let d = AutoDecision::decide(3, &p);
        assert_eq!(d.strategy, "color");
        assert_eq!(d.layers, 3);
    }

    #[test]
    fn few_colors_high_budget_is_hybrid() {
        let d = AutoDecision::decide(3, &detail_hungry());
        assert_eq!(d.strategy, "hybrid");
        assert_eq!(d.layers, 4);
    }

    #[test]
    fn many_colors_high_budget_is_hybrid_with_halved_budget() {
        let d = AutoDecision::decide(12, &detail_hungry());
        assert_eq!(d.strategy, "hybrid");
        assert_eq!(d.layers, 7); // 12/2 + 1
    }

    #[test]
    fn many_colors_low_budget_is_complexity() {
        let d = AutoDecision::decide(10, &coarse());
        assert_eq!(d.strategy, "complexity");
        assert_eq!(d.layers, 4);
    }

    #[test]
    fn threshold_tracks_detail_budget() {
        let t_detail = AutoDecision::decide(3, &detail_hungry()).complexity_threshold;
        let t_coarse = AutoDecision::decide(3, &coarse()).complexity_threshold;
        assert!(t_detail < t_coarse, "{t_detail} should be < {t_coarse}");
    }


    #[test]
    fn auto_assign_stamps_profiles_by_role() {
        let mut doc = VectorDoc::new(100, 100);
        // Two simple shapes of the dominant color + one complex other color.
        for _ in 0..2 {
            let mut sub = SubPath::new();
            sub.commands.push(PathCmd::MoveTo(PointF64 { x: 0.0, y: 0.0 }));
            sub.commands.push(PathCmd::LineTo(PointF64 { x: 10.0, y: 0.0 }));
            sub.commands.push(PathCmd::LineTo(PointF64 { x: 10.0, y: 10.0 }));
            sub.commands.push(PathCmd::Close);
            doc.shapes.push(vtracer::ir::Shape {
                paint: vtracer::ir::Paint::Solid(visioncortex::Color::new(255, 255, 255)),
                path: MultiPath { subpaths: vec![sub] },
            });
        }
        let mut complex = SubPath::new();
        for i in 0..30 {
            complex.commands.push(PathCmd::MoveTo(PointF64 { x: i as f64, y: 0.0 }));
            complex.commands.push(PathCmd::LineTo(PointF64 { x: i as f64, y: 1.0 }));
        }
        doc.shapes.push(vtracer::ir::Shape {
            paint: vtracer::ir::Paint::Solid(visioncortex::Color::new(50, 50, 50)),
            path: MultiPath { subpaths: vec![complex] },
        });

        let (assignment, decision) = AutoDecision::assign(&doc, &detail_hungry());
        assert_eq!(decision.strategy, "hybrid");
        assert!(!assignment.layers.is_empty());
        let bg = assignment
            .layers
            .iter()
            .find(|l| l.role == LayerRole::Background)
            .expect("background layer");
        assert_eq!(bg.profile, ProfileLevel::L0);
        let details = assignment
            .layers
            .iter()
            .find(|l| l.role == LayerRole::Details)
            .expect("details layer");
        assert_eq!(details.profile, ProfileLevel::L2);
    }

    #[test]
    fn l0_coarsens_cubic_chains() {
        let shape = cubic_shape();
        let before = cmd_count(&shape);
        let profiles = LayerProfile::derive(&ideal());
        let mut shape = shape;
        refine_shape(&mut shape, &profiles[0]);
        let after = cmd_count(&shape);
        assert!(after < before, "L0 should reduce commands: {before} -> {after}");
    }

    #[test]
    fn l1_is_a_noop() {
        let shape = cubic_shape();
        let before = cmd_count(&shape);
        let profiles = LayerProfile::derive(&ideal());
        let mut shape = shape;
        refine_shape(&mut shape, &profiles[1]);
        assert_eq!(cmd_count(&shape), before);
    }

    #[test]
    fn l2_preserves_topology() {
        let shape = cubic_shape();
        let profiles = LayerProfile::derive(&ideal());
        let mut shape = shape;
        refine_shape(&mut shape, &profiles[2]);
        // Same subpath count; the leading MoveTo survives quantization.
        assert_eq!(shape.path.subpaths.len(), 1);
        assert!(matches!(
            shape.path.subpaths[0].commands.first(),
            Some(PathCmd::MoveTo(_))
        ));
    }

    #[test]
    fn apply_profiles_writes_metadata() {
        let mut doc = VectorDoc::new(100, 100);
        doc.shapes.push(cubic_shape());
        let mut layered = LayeredDoc::from_vector_doc(&doc, &LayerAssignment::single_layer());
        layered.layers[0].role = LayerRole::Background;
        apply_profiles(&mut layered, &ideal());
        assert_eq!(
            layered.layers[0]
                .metadata
                .get("profile")
                .and_then(|v| v.as_str()),
            Some("L0")
        );
    }
}

