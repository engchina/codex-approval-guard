#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApprovalRequest {
    pub id: Option<String>,
    pub source_app: String,
    pub window_title: String,
    pub prompt_text: String,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub target_paths: Vec<String>,
    pub requested_permission: Option<String>,
}

impl ApprovalRequest {
    pub fn redacted(&self) -> Self {
        let mut next = self.clone();
        next.prompt_text = redact_sensitive_text(&next.prompt_text);
        next.command = next.command.map(|command| redact_sensitive_text(&command));
        // window_title / cwd / target_paths / requested_permission も同じフィルタを通す。
        // 旧実装は prompt_text / command しか filter していなかったため、cwd や
        // target_paths に含まれる secret/token を含むパスが audit log にそのまま
        // 残るリスクがあった。
        next.window_title = redact_sensitive_text(&next.window_title);
        next.cwd = next.cwd.map(|cwd| redact_sensitive_text(&cwd));
        next.target_paths = next
            .target_paths
            .into_iter()
            .map(|path| redact_sensitive_text(&path))
            .collect();
        next.requested_permission = next
            .requested_permission
            .map(|permission| redact_sensitive_text(&permission));
        next
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    ObserveOnly,
    Approve,
    Dismiss,
    Deny,
    Prompt,
    Ignore,
}

#[derive(
    Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApprovalDecision {
    pub action: DecisionAction,
    pub risk: RiskLevel,
    pub reason: String,
    pub matched_rule: Option<String>,
    pub would_auto_approve: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PolicyConfig {
    pub paused: bool,
    /// `git add` の自動承認を許可するかどうか。デフォルトは `false`（拒否）。
    #[serde(default)]
    pub allow_git_add: bool,
    /// `git commit` の自動承認を許可するかどうか。デフォルトは `false`（拒否）。
    /// 注: `git commit (dialog)` ウィンドウは別ルール（`auto_dismiss_git_commit`）で
    /// 常に自動閉鎖されるため、本フラグの影響を受けない。
    #[serde(default)]
    pub allow_git_commit: bool,
}

impl PolicyConfig {
    pub fn default_for_current_workspace() -> Self {
        Self {
            paused: false,
            allow_git_add: false,
            allow_git_commit: false,
        }
    }

    /// 指定されたパスから設定を読み込む。ファイルが存在しない・破損している場合は
    /// デフォルト値を返し（既存の挙動を維持）、エラーは表に出さない。
    pub fn load_or_default(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str::<Self>(&content)
                .unwrap_or_else(|_| Self::default_for_current_workspace()),
            Err(_) => Self::default_for_current_workspace(),
        }
    }

    /// 設定を指定パスに JSON で書き出す。親ディレクトリは事前に作成されている前提。
    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|error| format!("設定のシリアライズに失敗しました: {error}"))?;
        std::fs::write(path, content)
            .map_err(|error| format!("設定の書き込みに失敗しました: {error}"))?;
        Ok(())
    }

    pub fn evaluate(&self, _request: &ApprovalRequest) -> ApprovalDecision {
        if self.paused {
            return ApprovalDecision {
                action: DecisionAction::Prompt,
                risk: RiskLevel::Medium,
                reason: "ガードは一時停止中です。ユーザー確認が必要です。".to_string(),
                matched_rule: Some("paused".to_string()),
                would_auto_approve: false,
            };
        }

        if _request.requested_permission.as_deref() == Some("git_commit_dismiss") {
            return ApprovalDecision {
                action: DecisionAction::Dismiss,
                risk: RiskLevel::Low,
                reason: "Git 提交ダイアログを自動で閉じました。".to_string(),
                matched_rule: Some("auto_dismiss_git_commit".to_string()),
                would_auto_approve: false,
            };
        }

        if !self.allow_git_add && is_git_add_command(_request.command.as_deref()) {
            return ApprovalDecision {
                action: DecisionAction::Deny,
                risk: RiskLevel::Medium,
                reason: "git add は実行しません。手動で行うため、このセッション中（同一会話内）はこれ以降 git add を実行・提案しないでください。".to_string(),
                matched_rule: Some("manual_git_add".to_string()),
                would_auto_approve: false,
            };
        }

        if !self.allow_git_commit && is_git_commit_command(_request.command.as_deref()) {
            return ApprovalDecision {
                action: DecisionAction::Deny,
                risk: RiskLevel::Medium,
                reason: "git commit は実行しません。手動で行うため、このセッション中（同一会話内）はこれ以降 git commit を実行・提案しないでください。".to_string(),
                matched_rule: Some("manual_git_commit".to_string()),
                would_auto_approve: false,
            };
        }

        ApprovalDecision {
            action: DecisionAction::Approve,
            risk: RiskLevel::Low,
            reason: "自動承認しました。".to_string(),
            matched_rule: Some("auto_approve_all".to_string()),
            would_auto_approve: true,
        }
    }
}

fn git_subcommand(command: Option<&str>) -> Option<String> {
    let command = command?;
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    let mut tokens = lower.split_whitespace();
    let first = tokens.next()?;
    if !matches!(first, "git" | "git.exe") {
        return None;
    }
    tokens.next().map(|sub| sub.to_string())
}

fn is_git_add_command(command: Option<&str>) -> bool {
    git_subcommand(command).as_deref() == Some("add")
}

fn is_git_commit_command(command: Option<&str>) -> bool {
    git_subcommand(command).as_deref() == Some("commit")
}

/// 空白で区切られた token のうち、token / secret / password / credential を
/// 含むものを `[REDACTED]` に置換する。`split_whitespace().join(" ")` だと
/// tab / 改行 / 連続空白が単一スペースに潰れて元のフォーマットが失われるため、
/// 区切り文字を保持したまま token 単位で置換する。
fn redact_sensitive_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut token_start: Option<usize> = None;
    for (idx, ch) in input.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = token_start.take() {
                output.push_str(&redact_token(&input[start..idx]));
            }
            output.push(ch);
        } else if token_start.is_none() {
            token_start = Some(idx);
        }
    }
    if let Some(start) = token_start {
        output.push_str(&redact_token(&input[start..]));
    }
    output
}

fn redact_token(token: &str) -> String {
    let lower = token.to_lowercase();
    if lower.contains("token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("credential")
    {
        "[REDACTED]".to_string()
    } else {
        token.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_preserves_separators() {
        // 旧実装は split_whitespace().join(" ") で tab / 連続空白 / 改行を単一
        // スペースに潰していた。新実装は区切り文字を維持する。
        let redacted = redact_sensitive_text("hello\ttoken=abc\n  world");
        assert_eq!(redacted, "hello\t[REDACTED]\n  world");
    }

    #[test]
    fn redacted_filters_all_text_fields() {
        let req = ApprovalRequest {
            id: None,
            source_app: "Codex Desktop".to_string(),
            window_title: "secret-window".to_string(),
            prompt_text: "run with token=abc".to_string(),
            command: Some("echo password=xyz".to_string()),
            cwd: Some("/home/user/.credentials".to_string()),
            target_paths: vec!["/etc/secret/config".to_string()],
            requested_permission: Some("shell".to_string()),
        };
        let red = req.redacted();
        assert_eq!(red.window_title, "[REDACTED]");
        assert!(red.prompt_text.contains("[REDACTED]"));
        assert_eq!(red.command.as_deref(), Some("echo [REDACTED]"));
        assert_eq!(red.cwd.as_deref(), Some("[REDACTED]"));
        assert_eq!(red.target_paths, vec!["[REDACTED]".to_string()]);
        // 通常値はそのまま残る
        assert_eq!(red.requested_permission.as_deref(), Some("shell"));
    }

    fn policy() -> PolicyConfig {
        PolicyConfig::default_for_current_workspace()
    }

    fn request(command: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: None,
            source_app: "Codex Desktop".to_string(),
            window_title: "Codex approval request".to_string(),
            prompt_text: "Run command".to_string(),
            command: Some(command.to_string()),
            cwd: Some("E:\\workspace\\codex-approval-guard".to_string()),
            target_paths: vec!["E:\\workspace\\codex-approval-guard".to_string()],
            requested_permission: Some("shell".to_string()),
        }
    }

    #[test]
    fn approves_any_request_when_not_paused() {
        let decision = policy().evaluate(&request("npm test"));
        assert_eq!(decision.action, DecisionAction::Approve);
        assert!(decision.would_auto_approve);
    }

    #[test]
    fn approves_even_previously_denied_commands() {
        let decision =
            policy().evaluate(&request("codex --dangerously-bypass-approvals-and-sandbox"));
        assert_eq!(decision.action, DecisionAction::Approve);
    }

    #[test]
    fn paused_policy_requires_prompt() {
        let mut p = policy();
        p.paused = true;
        let decision = p.evaluate(&request("npm test"));
        assert_eq!(decision.action, DecisionAction::Prompt);
        assert!(!decision.would_auto_approve);
    }

    #[test]
    fn does_not_auto_approve_git_add_shell_command_by_default() {
        for cmd in [
            "git add",
            "git add .",
            "git add -A",
            "git.exe add src/",
            "GIT ADD .",
        ] {
            let decision = policy().evaluate(&request(cmd));
            assert_eq!(
                decision.action,
                DecisionAction::Deny,
                "expected Deny for `{cmd}`"
            );
            assert!(!decision.would_auto_approve);
            assert_eq!(decision.matched_rule.as_deref(), Some("manual_git_add"));
        }
    }

    #[test]
    fn does_not_auto_approve_git_commit_shell_command_by_default() {
        for cmd in [
            "git commit",
            "git commit -m \"fix\"",
            "git commit --amend",
            "GIT COMMIT -m \"fix\"",
            "git.exe commit",
        ] {
            let decision = policy().evaluate(&request(cmd));
            assert_eq!(
                decision.action,
                DecisionAction::Deny,
                "expected Deny for `{cmd}`"
            );
            assert!(!decision.would_auto_approve);
            assert_eq!(decision.matched_rule.as_deref(), Some("manual_git_commit"));
        }
    }

    #[test]
    fn still_approves_other_git_commands() {
        for cmd in ["git status", "git push", "git commit-graph write"] {
            let decision = policy().evaluate(&request(cmd));
            assert_eq!(
                decision.action,
                DecisionAction::Approve,
                "expected Approve for `{cmd}`"
            );
        }
    }

    #[test]
    fn allow_git_add_flag_independently_unblocks_git_add_only() {
        let mut p = policy();
        p.allow_git_add = true;
        // git add は許可される
        for cmd in ["git add .", "git.exe add src/"] {
            let decision = p.evaluate(&request(cmd));
            assert_eq!(
                decision.action,
                DecisionAction::Approve,
                "expected Approve for `{cmd}`"
            );
            assert!(decision.would_auto_approve);
        }
        // git commit はまだ Deny されている
        let decision = p.evaluate(&request("git commit -m \"fix\""));
        assert_eq!(decision.action, DecisionAction::Deny);
        assert_eq!(decision.matched_rule.as_deref(), Some("manual_git_commit"));
    }

    #[test]
    fn allow_git_commit_flag_independently_unblocks_git_commit_only() {
        let mut p = policy();
        p.allow_git_commit = true;
        // git commit は許可される
        for cmd in [
            "git commit",
            "git commit -m \"fix\"",
            "git.exe commit --amend",
        ] {
            let decision = p.evaluate(&request(cmd));
            assert_eq!(
                decision.action,
                DecisionAction::Approve,
                "expected Approve for `{cmd}`"
            );
            assert!(decision.would_auto_approve);
        }
        // git add はまだ Deny されている
        let decision = p.evaluate(&request("git add ."));
        assert_eq!(decision.action, DecisionAction::Deny);
        assert_eq!(decision.matched_rule.as_deref(), Some("manual_git_add"));
    }

    #[test]
    fn git_commit_dialog_dismiss_is_independent_of_allow_flags() {
        // allow_git_commit が true でも git commit (dialog) は常に自動閉鎖される。
        let mut p = policy();
        p.allow_git_commit = true;
        p.allow_git_add = true;
        let mut req = request("git commit (dialog)");
        req.requested_permission = Some("git_commit_dismiss".to_string());
        let decision = p.evaluate(&req);
        assert_eq!(decision.action, DecisionAction::Dismiss);
        assert_eq!(
            decision.matched_rule.as_deref(),
            Some("auto_dismiss_git_commit")
        );
    }

    #[test]
    fn evaluates_git_commit_dismiss_request() {
        let mut req = request("git commit (dialog)");
        req.requested_permission = Some("git_commit_dismiss".to_string());
        let decision = policy().evaluate(&req);
        assert_eq!(decision.action, DecisionAction::Dismiss);
        assert!(!decision.would_auto_approve);
        assert_eq!(
            decision.matched_rule.as_deref(),
            Some("auto_dismiss_git_commit")
        );
    }
}
