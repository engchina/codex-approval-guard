use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};

use chrono::Utc;
use uuid::Uuid;

use crate::policy::{ApprovalDecision, ApprovalRequest};

#[derive(Clone)]
pub struct AuditStore {
    path: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub created_at: String,
    pub request: ApprovalRequest,
    pub decision: ApprovalDecision,
}

impl AuditStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn append(
        &self,
        request: &ApprovalRequest,
        decision: &ApprovalDecision,
    ) -> Result<AuditEntry, String> {
        // 注: 旧実装は append のたびに list_recent(10) で全量 jsonl を読み込んで
        // created_at + command の組み合わせで dedup を試みていたが、created_at は
        // nano 精度の Utc::now() を含むため非起動イベントでは事実上 hit せず、
        // 起動イベントでのみ偶発的に dedup が走るだけの dead code だった。
        // 現状 append() は auto_approve_observed_request 経路でしか呼ばれず、
        // polling での重複書き込みは発生しないため、ここでの dedup は撤去する。
        let entry = AuditEntry {
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now().to_rfc3339(),
            request: request.redacted(),
            decision: decision.clone(),
        };

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("監査ログディレクトリを作成できません: {error}"))?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| format!("監査ログを開けません: {error}"))?;

        let line = serde_json::to_string(&entry)
            .map_err(|error| format!("監査ログをシリアライズできません: {error}"))?;
        writeln!(file, "{line}").map_err(|error| format!("監査ログを書き込めません: {error}"))?;

        Ok(entry)
    }

    pub fn list_recent(&self, limit: usize) -> Result<Vec<AuditEntry>, String> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file =
            File::open(&self.path).map_err(|error| format!("監査ログを読み込めません: {error}"))?;
        let reader = BufReader::new(file);

        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|error| format!("監査ログ行を読み込めません: {error}"))?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<AuditEntry>(&line) {
                entries.push(entry);
            }
        }

        entries.reverse();
        entries.truncate(limit);
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{ApprovalDecision, ApprovalRequest, DecisionAction, RiskLevel};

    #[test]
    fn append_writes_each_call_as_distinct_entry() {
        // 旧実装の偽 dedup を撤去したため、同じ request を 2 回 append すると
        // それぞれが独立した id / created_at を持つ別エントリとして残る。
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("test_audit_{}.jsonl", uuid::Uuid::new_v4()));
        let store = AuditStore::new(path.clone());

        let req = ApprovalRequest {
            id: None,
            source_app: "Codex Desktop".to_string(),
            window_title: "Codex approval request".to_string(),
            prompt_text: "Run command".to_string(),
            command: Some("npm run tauri dev".to_string()),
            cwd: Some("C:\\test".to_string()),
            target_paths: vec![],
            requested_permission: None,
        };

        let decision = ApprovalDecision {
            action: DecisionAction::Approve,
            risk: RiskLevel::Low,
            reason: "Auto-approve test".to_string(),
            matched_rule: None,
            would_auto_approve: true,
        };

        let entry1 = store.append(&req, &decision).unwrap();
        let entry2 = store.append(&req, &decision).unwrap();
        assert_ne!(entry1.id, entry2.id);

        let req2 = ApprovalRequest {
            command: Some("npm test".to_string()),
            ..req.clone()
        };
        let entry3 = store.append(&req2, &decision).unwrap();
        assert_ne!(entry1.id, entry3.id);

        let list = store.list_recent(10).unwrap();
        assert_eq!(list.len(), 3);

        let _ = std::fs::remove_file(path);
    }
}
