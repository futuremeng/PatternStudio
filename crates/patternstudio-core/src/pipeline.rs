//! Pipeline driver: orchestrates VTracer into a patternstudio pipeline.

use crate::config::PatternStudioConfig;
use crate::error::Error;
use crate::ir::{LayerAssignment, LayeredDoc};
use vtracer::{ColorImage, Pipeline, VectorDoc};

/// The patternstudio pipeline: a composable wrapper around VTracer.
pub struct PatternStudioPipeline {
    pipeline: Pipeline,
}

impl PatternStudioPipeline {
    /// Create a pipeline from a PatternStudioConfig.
    pub fn new(config: &PatternStudioConfig) -> Result<Self, Error> {
        let pipeline = config.build_pipeline()?;
        Ok(Self { pipeline })
    }

    /// Run the full pipeline: Image -> VectorDoc.
    pub fn trace(&self, img: &ColorImage) -> Result<VectorDoc, Error> {
        Ok(self.pipeline.run(img)?)
    }

    /// Stage 1: Segment the image.
    pub fn segment(&self, img: &ColorImage) -> Result<vtracer::Segmentation, Error> {
        Ok(self.pipeline.segment(img)?)
    }

    /// Stage 2: Finish (compose + curve fit) a segmentation into VectorDoc.
    pub fn finish(&self, seg: &vtracer::Segmentation) -> Result<VectorDoc, Error> {
        Ok(self.pipeline.finish(seg)?)
    }

    /// Run the full pipeline and produce a LayeredDoc.
    pub fn trace_layered(
        &self,
        img: &ColorImage,
        assignment: &LayerAssignment,
    ) -> Result<LayeredDoc, Error> {
        let doc = self.trace(img)?;
        Ok(LayeredDoc::from_vector_doc(&doc, assignment))
    }
}

impl Default for PatternStudioPipeline {
    fn default() -> Self {
        let config = PatternStudioConfig::default();
        Self::new(&config).expect("default pipeline should build")
    }
}
