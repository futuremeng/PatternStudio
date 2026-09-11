//! PenPaw configuration: wraps VTracer Config with high-level presets.

use vtracer::config::Preset;
use vtracer::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Resolution {
    Coarse,
    #[default]
    Medium,
    Fine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Simplify {
    Minimal,
    #[default]
    Low,
    Medium,
    High,
    Maximum,
}

#[derive(Debug, Clone)]
pub struct PenPawConfig {
    pub resolution: Resolution,
    pub simplify: Simplify,
    pub vtracer: Config,
}

impl PenPawConfig {
    pub fn from_preset(preset: Preset) -> Self {
        Self {
            resolution: Resolution::Medium,
            simplify: Simplify::Low,
            vtracer: Config::from_preset(preset),
        }
    }

    pub fn to_vtracer_config(&self) -> Config {
        let mut cfg = self.vtracer.clone();
        match self.resolution {
            Resolution::Coarse => {
                cfg.filter_speckle = cfg.filter_speckle.max(4);
                cfg.path_precision = Some(3);
            }
            Resolution::Medium => {}
            Resolution::Fine => {
                cfg.filter_speckle = cfg.filter_speckle.min(1);
                cfg.path_precision = Some(6);
            }
        }
        match self.simplify {
            Simplify::Minimal => cfg.simplify = Some(0.1),
            Simplify::Low => cfg.simplify = Some(0.5),
            Simplify::Medium => cfg.simplify = Some(1.0),
            Simplify::High => cfg.simplify = Some(2.0),
            Simplify::Maximum => cfg.simplify = Some(5.0),
        }
        cfg
    }

    pub fn build_pipeline(&self) -> Result<vtracer::Pipeline, vtracer::Error> {
        self.to_vtracer_config().build()
    }
}

impl Default for PenPawConfig {
    fn default() -> Self {
        Self::from_preset(Preset::Poster)
    }
}
