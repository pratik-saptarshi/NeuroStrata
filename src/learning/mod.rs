pub mod blob_store;
pub mod calibration;
pub mod control;
pub mod dataset;
pub mod evaluator;
pub mod labels;
pub mod model_bundle;
pub mod observation;
pub mod recorder;
pub mod runtime;
pub mod split;
pub mod status;
pub mod trainer;

pub use blob_store::BlobStore;
pub use calibration::TemperatureFitter;
pub use dataset::{build_dedup_dataset, DatasetBuildReport, DatasetManifest, DistancePolicy};
pub use evaluator::{EvaluationReport as EvaluatorReport, SuiteReport};
pub use labels::{derive_labels, DerivedLabel, LabelRule, LabelTier};
pub use model_bundle::ModelBundle;
pub use observation::{
    CandidateSource, DecisionObservation, LearningContext, LearningOutcome, LearningOutcomeKind,
    ObservedCandidate, PayloadHash,
};
pub use recorder::LearningRecorder;
pub use runtime::{LearningRuntimeManager, ModelVersion, RuntimeState};
pub use split::{split_observations, DatasetSplits, Split};
pub use trainer::{EvaluationReport, TrainingJobState};

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Every `.rs` file under `src/learning/` must be declared in this
    /// `mod.rs`; an undeclared file is dead weight that never compiles and
    /// misleads readers about what the learning surface contains.
    #[test]
    fn every_source_file_is_in_the_module_tree() {
        let learning_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("learning");
        let mod_rs = std::fs::read_to_string(learning_dir.join("mod.rs")).unwrap();
        let mut missing = Vec::new();
        for entry in std::fs::read_dir(&learning_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().map(|e| e == "rs") != Some(true) {
                continue;
            }
            let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
            if stem == "mod" {
                continue;
            }
            if !mod_rs.contains(&format!("mod {stem};")) {
                missing.push(stem);
            }
        }
        assert!(
            missing.is_empty(),
            "files under src/learning/ not declared in mod.rs: {missing:?}"
        );
    }
}
