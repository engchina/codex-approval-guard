use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::OnceLock,
};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::policy::{ApprovalDecision, ApprovalRequest};

static APP_START_TIME: OnceLock<DateTime<Utc>> = OnceLock::new();

pub fn get_app_start_time() -> DateTime<Utc> {
    *APP_START_TIME.get_or_init(Utc::now)
}

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
        let is_startup = request.command.as_ref().map_or(false, |cmd| {
            let cmd_lower = cmd.to_lowercase();
            cmd_lower.contains("tauri dev")
                || cmd_lower.contains("tauri:dev")
                || cmd_lower.contains("npm run dev")
                || cmd_lower.contains("npm run tauri")
                || cmd_lower.contains("cargo tauri dev")
        });

        let created_at = if is_startup {
            get_app_start_time().to_rfc3339()
        } else {
            Utc::now().to_rfc3339()
        };

        // Check if there is already an entry with the same created_at and command/window_title
        if let Ok(recent) = self.list_recent(10) {
            if let Some(existing) = recent.iter().find(|entry| {
                entry.created_at == created_at && entry.request.command == request.command
            }) {
                return Ok(existing.clone());
            }
        }

        let entry = AuditEntry {
            id: Uuid::new_v4().to_string(),
            created_at,
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
    fn test_audit_deduplication() {
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

        // First append
        let entry1 = store.append(&req, &decision).unwrap();
        assert_eq!(entry1.created_at, get_app_start_time().to_rfc3339());

        // Second append (should deduplicate and return the same entry)
        let entry2 = store.append(&req, &decision).unwrap();
        assert_eq!(entry1.id, entry2.id);

        let list = store.list_recent(10).unwrap();
        assert_eq!(list.len(), 1);

        // A different command should not deduplicate
        let req2 = ApprovalRequest {
            command: Some("npm test".to_string()),
            ..req.clone()
        };
        let entry3 = store.append(&req2, &decision).unwrap();
        assert_ne!(entry1.id, entry3.id);

        let list2 = store.list_recent(10).unwrap();
        assert_eq!(list2.len(), 2);

        let _ = std::fs::remove_file(path);
    }
}
