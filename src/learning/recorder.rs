use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use crate::learning::observation::{
    DecisionObservation, LearningContext, LearningOutcome, LearningOutcomeKind, PayloadHash,
};

pub struct LearningRecorder {
    root: PathBuf,
    connection_id: String,
}

impl LearningRecorder {
    pub async fn open(root: PathBuf, connection_id: Option<String>) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        let conn_id = connection_id.unwrap_or_else(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            root.as_os_str().hash(&mut h);
            format!("conn-{}", h.finish())
        });
        Ok(Self {
            root,
            connection_id: conn_id,
        })
    }

    pub async fn record_observation(&self, value: &DecisionObservation) -> Result<()> {
        let month = chrono::Local::now().format("%Y%m").to_string();
        let path = self.root.join(format!("observations-{}.jsonl", month));
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        let line = serde_json::to_string(value)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.flush()?;
        Ok(())
    }

    pub async fn record_outcome(&self, value: &LearningOutcome) -> Result<()> {
        let month = chrono::Local::now().format("%Y%m").to_string();
        let path = self.root.join(format!("outcomes-{}.jsonl", month));
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        let line = serde_json::to_string(value)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.flush()?;
        Ok(())
    }

    pub async fn record_blob_hash(
        &self,
        _payload_hash: &PayloadHash,
        _input: &crate::traits::MemoryPayload,
    ) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::BlobStore;
    use crate::traits::MemoryPayload;
    use serde_json::json;

    #[test]
    fn failing_blob_reuse_content_addressed() {
        let payload1 = MemoryPayload {
            content: "hello".to_string(),
            user_id: "u".to_string(),
            memory_type: "fact".to_string(),
            agent_name: None,
            location: "src/lib.rs".to_string(),
            location_lines: "1-2".to_string(),
            metadata: json!({}),
        };
        let store = BlobStore::new().expect("create store");
        let hash1 = store.put(&payload1).expect("hash 1");
        let hash2 = store.put(&payload1).expect("hash 2");
        assert_eq!(hash1, hash2, "same payload should reuse blob");
        assert!(hash1.0.starts_with("sha256:"));
    }

    #[tokio::test]
    async fn observation_jsonl_appends_records_across_reopen() {
        let tmpdir = std::env::temp_dir().join(format!("ns-learn-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmpdir).unwrap();
        let rec = LearningRecorder::open(tmpdir.clone(), None).await.unwrap();
        let v1 = DecisionObservation {
            schema_version: 1,
            observation_id: "obs-1".into(),
            occurred_at: 0,
            namespace: "probe".into(),
            context: LearningContext {
                connection_id: "conn-x".into(),
                active_task_id: None,
            },
            result_memory_id: Some("mid-1".into()),
            input_payload_hash: PayloadHash("sha256:abc123".into()),
            candidates: vec![],
            embedder_id: "e".into(),
            embedder_dimensions: 4,
            selected_provider: "auto".into(),
            policy_version: "v1".into(),
            model_version: None,
            shadow_provider: None,
            shadow_model_version: None,
            decision: crate::judgment::DedupDecision::Allowed,
        };
        rec.record_observation(&v1).await.unwrap();
        drop(rec);
        let reopened = LearningRecorder::open(tmpdir.clone(), None).await.unwrap();
        let v2 = DecisionObservation {
            schema_version: 1,
            observation_id: "obs-2".into(),
            occurred_at: 1,
            namespace: "probe".into(),
            context: LearningContext {
                connection_id: "conn-y".into(),
                active_task_id: None,
            },
            result_memory_id: Some("mid-2".into()),
            input_payload_hash: PayloadHash("sha256:def456".into()),
            candidates: vec![],
            embedder_id: "e".into(),
            embedder_dimensions: 4,
            selected_provider: "auto".into(),
            policy_version: "v1".into(),
            model_version: None,
            shadow_provider: None,
            shadow_model_version: None,
            decision: crate::judgment::DedupDecision::BlockedExact,
        };
        reopened.record_observation(&v2).await.unwrap();

        let mut entries = std::fs::read_dir(&tmpdir).unwrap();
        let path = entries.next().unwrap().unwrap().path();
        assert!(entries.next().is_none(), "one log file per month");
        let contents = std::fs::read_to_string(path).unwrap();
        let rows: Vec<DecisionObservation> = contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), 2, "each flushed append is a complete JSONL row");
        assert_eq!(rows[0].observation_id, "obs-1");
        assert_eq!(rows[1].observation_id, "obs-2");
        std::fs::remove_dir_all(tmpdir).unwrap();
    }
}
