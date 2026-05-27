use super::ObservedApproval;
use crate::policy::ApprovalRequest;

#[cfg(test)]
fn parse_observed_approval(
    title: &str,
    raw_text: Vec<String>,
    detected_by: &str,
) -> Option<ObservedApproval> {
    parse_observed_approval_with_context(title, raw_text, detected_by, false)
}

pub fn parse_observed_approval_with_context(
    title: &str,
    raw_text: Vec<String>,
    detected_by: &str,
    trusted_codex_context: bool,
) -> Option<ObservedApproval> {
    let raw_text = normalize_raw_text(raw_text);

    if looks_like_guard_self(title, &raw_text) {
        return None;
    }

    if (!trusted_codex_context && !looks_like_codex_context(title, &raw_text))
        || !looks_like_approval_text(title, &raw_text)
    {
        return None;
    }

    let prompt_text = raw_text.join("\n");
    let command = extract_command(&raw_text);
    let prompt_window = locate_prompt_window(&raw_text);
    let cwd = extract_cwd(&raw_text);
    let mut target_paths = extract_windows_paths(prompt_window.unwrap_or(&raw_text));
    for path in extract_changed_files(prompt_window.unwrap_or(&raw_text)) {
        if !target_paths.iter().any(|existing| existing == &path) {
            target_paths.push(path);
        }
    }

    Some(ObservedApproval {
        request: ApprovalRequest {
            id: None,
            source_app: "Codex Desktop".to_string(),
            window_title: title.to_string(),
            prompt_text,
            command,
            cwd,
            target_paths,
            requested_permission: infer_permission(&raw_text),
        },
        raw_text,
        detected_by: detected_by.to_string(),
    })
}

/// サイドバー上で「承認待ち」を示すバッジ文字列かどうかを判定する。
///
/// Codex Desktop は、現在アクティブでない会話に承認待ちが発生した場合、
/// サイドバーのアイテムに小さなバッジ（中国語: 「等待批准」/「等待审批」/
/// 日本語: 「承認待ち」/「承認が必要」/ 英語: "Awaiting approval" / "Pending approval"）
/// を表示する。承認ダイアログ本体は当該会話を選択するまで UI ツリーに現れないため、
/// このバッジを手掛かりに先にサイドバーアイテムを `Invoke` してアクティブ化する必要がある。
///
/// マッチ条件は「短いラベル文字列の中に既知のバッジ語が含まれる」こと。
/// 長文の中の偶然の一致（例: ヘルプテキストの "Awaiting approval" の説明）を拾わないよう
/// 80 文字を超えるテキストは除外する。
pub fn is_pending_approval_badge(label: &str) -> bool {
    let trimmed = label.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 80 {
        return false;
    }
    let lower = trimmed.to_lowercase();
    trimmed.contains("等待批准")
        || trimmed.contains("等待审批")
        || trimmed.contains("待批准")
        || trimmed.contains("承認待ち")
        || trimmed.contains("承認が必要")
        || trimmed.contains("要承認")
        || lower.contains("awaiting approval")
        || lower.contains("pending approval")
        || lower.contains("needs approval")
        || lower.contains("approval pending")
        || lower.contains("waiting for approval")
}

pub fn looks_like_git_commit_window(title: &str, raw_text: &[String]) -> bool {
    if title_matches_git_commit(title) {
        return true;
    }
    let combined_text = raw_text.join("\n").to_lowercase();
    combined_text.contains("提交更改")
        || combined_text.contains("commit changes")
        || combined_text.contains("変更をコミット")
        || (combined_text.contains("提交消息") && combined_text.contains("分支"))
        || (combined_text.contains("commit message") && combined_text.contains("branch"))
}

/// タイトルのみで git commit dialog かどうか判定する。
/// title だけで命中する場合、Codex Desktop は独立した HWND（Tauri Window）として
/// dialog を開いていると判断でき、WM_CLOSE 等の HWND 直接操作が安全に利用できる。
pub fn title_matches_git_commit(title: &str) -> bool {
    let title_lower = title.to_lowercase();
    title_lower.contains("提交更改")
        || title_lower.contains("commit changes")
        || title_lower.contains("変更をコミット")
        || title_lower.contains("コミット")
}

fn normalize_raw_text(raw_text: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    for value in raw_text {
        push_clean(&mut output, value);
    }
    output
}

fn push_clean(output: &mut Vec<String>, value: String) {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.len() >= 2 && !output.iter().any(|existing| existing == &normalized) {
        output.push(normalized);
    }
}

fn looks_like_codex_context(title: &str, raw_text: &[String]) -> bool {
    let text = format!("{}\n{}", title, raw_text.join("\n")).to_lowercase();
    text.contains("codex")
}

fn looks_like_guard_self(title: &str, raw_text: &[String]) -> bool {
    let text = format!("{}\n{}", title, raw_text.join("\n")).to_lowercase();
    text.contains("codex approval guard")
}

fn looks_like_approval_text(title: &str, raw_text: &[String]) -> bool {
    let text = raw_text
        .iter()
        .filter(|line| !line.eq_ignore_ascii_case(title))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    text.contains("approval request")
        || text.contains("command approval")
        || text.contains("approval required")
        || text.contains("approve")
        || text.contains("approval")
        || text.contains("requires approval")
        || text.contains("needs approval")
        || text.contains("run command")
        || text.contains("permission to run")
        || text.contains("wants permission")
        || text.contains("apply these changes")
        || text.contains("apply changes")
        || text.contains("是否应用这些更改")
        || text.contains("应用这些更改")
        || text.contains("是否应用这些改动")
        || text.contains("应用这些改动")
        || text.contains("是否应用更改")
        || text.contains("変更を適用")
        || text.contains("これらの変更")
        || text.contains("承認")
        || text.contains("审批")
        || text.contains("批准")
        || text.contains("权限")
        || text.contains("许可")
        || text.contains("是否运行")
        || text.contains("运行此命令")
        || text.contains("sandbox")
        || text.contains("permission")
        || text.contains("許可")
        // 「（推荐）/（推奨）/(recommended)」マーカーは Codex の ask_user_question 系
        // 多選プロンプトでのみ使用されるため、これも承認候補と見なす。
        || text.contains("（推荐）")
        || text.contains("(推荐)")
        || text.contains("（推奨）")
        || text.contains("(推奨)")
        || text.contains("（おすすめ）")
        || text.contains("(おすすめ)")
        || text.contains("(recommended)")
        || text.contains("（recommended）")
        || text.contains("実施しますか")
        || text.contains("実行しますか")
        || text.contains("このプラン")
        || text.contains("执行此方案")
        || text.contains("是否执行该方案")
        || text.contains("是否执行此方案")
        || text.contains("execute this plan")
        || text.contains("execute the plan")
}

fn extract_command(raw_text: &[String]) -> Option<String> {
    // ファイル編集の承認ダイアログ（apply changes 系）やプラン確認ダイアログではコマンドは存在しないため、
    // チャット履歴中の "git add" などを誤って拾わないよう早期に return する。
    if is_apply_changes_prompt(raw_text) || is_plan_prompt(raw_text) {
        return None;
    }

    if let Some(command) = extract_command_after_approval_prompt(raw_text) {
        return Some(split_concatenated_commands(&command));
    }

    for line in raw_text {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();

        if let Some(command) = trimmed
            .strip_prefix("$ ")
            .or_else(|| trimmed.strip_prefix("> "))
        {
            if looks_like_shell_command(command) {
                return Some(split_concatenated_commands(command));
            }
        }

        if let Some((label, value)) = trimmed.split_once(':') {
            let label = label.to_lowercase();
            if (label.contains("command") || label.contains("コマンド") || label.contains("実行"))
                && looks_like_shell_command(value)
            {
                return Some(split_concatenated_commands(value.trim()));
            }
        }

        if let Some(command) = trimmed
            .strip_prefix("正在运行 ")
            .or_else(|| trimmed.strip_prefix("実行中 "))
        {
            if looks_like_shell_command(command) {
                return Some(split_concatenated_commands(command.trim()));
            }
        }

        if looks_like_shell_command(&lower) {
            return Some(split_concatenated_commands(trimmed));
        }
    }

    None
}

fn is_apply_changes_prompt(raw_text: &[String]) -> bool {
    raw_text.iter().any(|line| {
        let lower = line.to_lowercase();
        lower.contains("apply these changes")
            || lower.contains("apply changes")
            || line.contains("是否应用这些更改")
            || line.contains("是否应用这些改动")
            || line.contains("是否应用更改")
            || line.contains("应用这些更改")
            || line.contains("应用这些改动")
            || line.contains("変更を適用")
            || line.contains("これらの変更を適用")
            // 日本語版 Codex Desktop の実際の表記。screenshot 確認済み:
            // 「これらの変更を行いますか?」。
            || line.contains("これらの変更を行")
            || line.contains("変更を行いますか")
    })
}

fn is_plan_prompt(raw_text: &[String]) -> bool {
    raw_text.iter().any(|line| {
        let lower = line.to_lowercase();
        lower.contains("このプランを実施しますか")
            || lower.contains("プランを実施しますか")
            || lower.contains("実行しますか")
            || lower.contains("是否执行此方案")
            || lower.contains("是否执行该方案")
            || lower.contains("是否执行方案")
            || lower.contains("execute this plan")
            || lower.contains("execute the plan")
    })
}

fn extract_command_after_approval_prompt(raw_text: &[String]) -> Option<String> {
    for (index, line) in raw_text.iter().enumerate() {
        let lower = line.to_lowercase();
        let is_prompt = lower.contains("run command") || line.contains("是否运行此命令");
        if !is_prompt {
            continue;
        }

        for candidate in raw_text.iter().skip(index + 1).take(6) {
            if looks_like_shell_command(candidate) {
                return Some(candidate.trim().to_string());
            }
        }
    }

    None
}

const SHELL_PREFIX_TOKENS: &[&str] = &[
    "pnpm ",
    "npm ",
    "cargo ",
    "cargo.exe ",
    "git ",
    "go test",
    "node ",
    "python ",
    "python.exe ",
    "pytest",
    "rg ",
    "uv ",
    "yarn ",
    "powershell ",
    "pwsh ",
    "curl ",
    "remove-item ",
    "invoke-webrequest ",
    "cmd /c ",
];

fn split_concatenated_commands(line: &str) -> String {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();
    let Some(start_prefix) = SHELL_PREFIX_TOKENS
        .iter()
        .find(|prefix| lower.starts_with(*prefix))
    else {
        return trimmed.to_string();
    };

    let head_len = start_prefix.len();
    let tail = &lower[head_len..];
    let mut cut: Option<usize> = None;
    for prefix in SHELL_PREFIX_TOKENS {
        let pattern = format!(" {prefix}");
        if let Some(pos) = tail.find(&pattern) {
            let abs = head_len + pos;
            cut = Some(cut.map_or(abs, |existing| existing.min(abs)));
        }
    }

    // 注: 旧実装は `trimmed[..pos]` を用いて元文字列を切っていたが、
    // `pos` は `lower` 上のバイト位置である。`to_lowercase` は一部の
    // Unicode 文字（土耳其語 İ → i + U+0307 等）でバイト数を変えるため、
    // 同じバイト位置を `trimmed` に流用すると UTF-8 文字境界を割って
    // panic する可能性があった。バイト長が一致する一般的なケース
    // （ASCII / 中日漢字など）では従来通り `trimmed` を切り、原大小文字を
    // 保つ。バイト長が変わる稀ケースのみ `lower` を切って安全側に倒す。
    let command = match cut {
        Some(pos) => {
            let source = if lower.len() == trimmed.len() {
                trimmed
            } else {
                lower.as_str()
            };
            source[..pos].trim_end().to_string()
        }
        None => trimmed.to_string(),
    };
    trim_trailing_command_note(&command).to_string()
}

fn trim_trailing_command_note(command: &str) -> &str {
    let tokens = indexed_tokens(command);
    if tokens.len() < 4 {
        return command.trim();
    }

    let first = tokens[0].text.to_lowercase();
    let second = tokens[1].text.to_lowercase();
    if !matches!(first.as_str(), "npm" | "pnpm" | "yarn")
        || second != "run"
        || tokens[3].text.starts_with('-')
        || !looks_like_natural_language_note(tokens[3].text)
    {
        return command.trim();
    }

    command[..tokens[2].end].trim_end()
}

#[derive(Debug)]
struct IndexedToken<'a> {
    text: &'a str,
    end: usize,
}

fn indexed_tokens(input: &str) -> Vec<IndexedToken<'_>> {
    let mut tokens = Vec::new();
    let mut start = None;

    for (index, char) in input.char_indices() {
        if char.is_whitespace() {
            if let Some(token_start) = start.take() {
                tokens.push(IndexedToken {
                    text: &input[token_start..index],
                    end: index,
                });
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }

    if let Some(token_start) = start {
        tokens.push(IndexedToken {
            text: &input[token_start..],
            end: input.len(),
        });
    }

    tokens
}

fn looks_like_natural_language_note(token: &str) -> bool {
    let trimmed = token.trim_start_matches(|char: char| {
        matches!(char, '"' | '\'' | '`' | '(' | '[' | '（' | '「')
    });
    [
        "确认", "確認", "検証", "确保", "確実", "请", "請", "为了", "為了", "ため",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

fn looks_like_shell_command(value: &str) -> bool {
    let command = value.trim().to_lowercase();
    [
        "cargo ",
        "cargo.exe ",
        "cmd /c ",
        "git ",
        "go test",
        "node ",
        "npm ",
        "pnpm ",
        "powershell ",
        "pwsh ",
        "pytest",
        "python ",
        "python.exe ",
        "rg ",
        "uv ",
        "yarn ",
        "remove-item ",
        "invoke-webrequest ",
        "curl ",
    ]
    .iter()
    .any(|prefix| command.starts_with(prefix))
        || [
            "cargo", "git", "node", "npm", "pnpm", "pytest", "python", "rg", "yarn",
        ]
        .contains(&command.as_str())
}

fn locate_prompt_window(raw_text: &[String]) -> Option<&[String]> {
    const SPAN: usize = 20;
    let index = raw_text.iter().position(|line| {
        let lower = line.to_lowercase();
        lower.contains("run command")
            || lower.contains("apply these changes")
            || lower.contains("apply changes")
            || lower.contains("permission to run")
            || lower.contains("requires approval")
            || lower.contains("needs approval")
            || line.contains("是否运行")
            || line.contains("是否应用")
            || line.contains("変更を適用")
            || line.contains("これらの変更")
            || line.contains("変更を行いますか")
    })?;
    let start = index.saturating_sub(SPAN);
    let end = (index + SPAN + 1).min(raw_text.len());
    Some(&raw_text[start..end])
}

fn extract_cwd(raw_text: &[String]) -> Option<String> {
    raw_text.iter().find_map(|line| {
        let lower = line.to_lowercase();
        let is_cwd_line = lower.contains("cwd")
            || lower.contains("working directory")
            || lower.contains("作業ディレクトリ")
            || lower.contains("作業フォルダ");
        if is_cwd_line {
            extract_windows_paths_from_line(line).into_iter().next()
        } else {
            None
        }
    })
}

fn extract_windows_paths(raw_text: &[String]) -> Vec<String> {
    let mut output = Vec::new();
    for line in raw_text {
        for path in extract_windows_paths_from_line(line) {
            if !output.iter().any(|existing| existing == &path) {
                output.push(path);
            }
        }
    }
    output
}

fn extract_changed_files(raw_text: &[String]) -> Vec<String> {
    let mut output = Vec::new();
    for line in raw_text {
        let Some(path) = extract_changed_file_from_line(line) else {
            continue;
        };
        if !output.iter().any(|existing| existing == &path) {
            output.push(path);
        }
    }
    output
}

fn extract_changed_file_from_line(line: &str) -> Option<String> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    if tokens.len() < 3 {
        return None;
    }

    let has_added = tokens.iter().any(|token| looks_like_stat_token(token, '+'));
    let has_removed = tokens.iter().any(|token| looks_like_stat_token(token, '-'));
    if !has_added || !has_removed {
        return None;
    }

    let candidate = tokens[0].trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | '`' | ',' | ';' | ')' | '(' | '[' | ']' | '<' | '>'
        )
    });
    let lower = candidate.to_lowercase();
    if candidate.len() >= 3
        && candidate.contains('.')
        && !lower.starts_with("http://")
        && !lower.starts_with("https://")
    {
        Some(candidate.replace('/', "\\"))
    } else {
        None
    }
}

fn looks_like_stat_token(token: &str, sign: char) -> bool {
    let Some(rest) = token.strip_prefix(sign) else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|char| char.is_ascii_digit())
}

fn extract_windows_paths_from_line(line: &str) -> Vec<String> {
    const PATH_BOUNDARY: &[u8] = b",;\"'`()[]<>|";
    let bytes = line.as_bytes();
    let mut paths = Vec::new();
    let mut i = 0;

    while i + 2 < bytes.len() {
        if !(bytes[i].is_ascii_alphabetic() && bytes[i + 1] == b':' && bytes[i + 2] == b'\\') {
            i += 1;
            continue;
        }
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }

        let start = i;
        let mut end = start + 3;
        while end < bytes.len() {
            let c = bytes[end];
            if c == b'\n' || c == b'\r' || c == b'\t' {
                break;
            }
            if PATH_BOUNDARY.contains(&c) {
                break;
            }
            if c == b' ' {
                if end + 1 < bytes.len() && bytes[end + 1] == b' ' {
                    break;
                }
                if end + 3 < bytes.len()
                    && bytes[end + 1].is_ascii_alphabetic()
                    && bytes[end + 2] == b':'
                    && bytes[end + 3] == b'\\'
                {
                    break;
                }
            }
            end += 1;
        }

        let raw = &line[start..end];
        let trimmed = raw.trim_end_matches(|c: char| {
            matches!(c, '.' | ',' | ';' | ' ' | '\t' | ':' | '"' | '\'')
        });
        if trimmed.len() > 3 {
            paths.push(trimmed.to_string());
        }
        i = end.max(start + 3);
    }

    paths
}

fn infer_permission(raw_text: &[String]) -> Option<String> {
    let text = raw_text.join("\n").to_lowercase();
    if text.contains("network") || text.contains("internet") || text.contains("ネットワーク")
    {
        return Some("network".to_string());
    }
    if text.contains("shell")
        || text.contains("command")
        || text.contains("コマンド")
        || text.contains("命令")
        || text.contains("是否运行")
        || text.contains("运行此命令")
    {
        return Some("shell".to_string());
    }
    if text.contains("file")
        || text.contains("write")
        || text.contains("ファイル")
        || text.contains("更改")
        || text.contains("改动")
        || text.contains("文件")
        || text.contains("apply these changes")
        || text.contains("apply changes")
        || text.contains("変更")
        || text.contains("プラン")
        || text.contains("方案")
        || text.contains("plan")
    {
        return Some("file".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_lines(text: &str) -> Vec<String> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn parses_codex_npm_test_fixture() {
        let observed = parse_observed_approval(
            "Codex approval request",
            fixture_lines(include_str!("../../fixtures/ui_text/codex_npm_test.txt")),
            "fixture",
        )
        .expect("fixture should parse");

        assert_eq!(observed.request.command.as_deref(), Some("npm test"));
        assert_eq!(
            observed.request.cwd.as_deref(),
            Some("E:\\workspace\\codex-approval-guard")
        );
        assert_eq!(
            observed.request.requested_permission.as_deref(),
            Some("shell")
        );
    }

    #[test]
    fn parses_codex_workspace_write_fixture() {
        let observed = parse_observed_approval(
            "OpenAI Codex",
            fixture_lines(include_str!(
                "../../fixtures/ui_text/codex_workspace_write.txt"
            )),
            "fixture",
        )
        .expect("fixture should parse");

        assert_eq!(
            observed.request.command.as_deref(),
            Some("cargo test --workspace")
        );
        assert!(observed
            .request
            .target_paths
            .contains(&"E:\\workspace\\codex-approval-guard".to_string()));
    }

    #[test]
    fn ignores_uac_yes_no_fixture() {
        let observed = parse_observed_approval(
            "User Account Control",
            fixture_lines(include_str!("../../fixtures/ui_text/uac_yes_no.txt")),
            "fixture",
        );
        assert!(observed.is_none());
    }

    #[test]
    fn ignores_browser_permission_fixture() {
        let observed = parse_observed_approval(
            "Google Chrome",
            fixture_lines(include_str!(
                "../../fixtures/ui_text/browser_permission.txt"
            )),
            "fixture",
        );
        assert!(observed.is_none());
    }

    #[test]
    fn ignores_generic_approval_without_codex() {
        let observed = parse_observed_approval(
            "Approval request",
            fixture_lines(include_str!("../../fixtures/ui_text/generic_approval.txt")),
            "fixture",
        );
        assert!(observed.is_none());
    }

    #[test]
    fn ignores_guard_self_unresponsive_fixture() {
        let observed = parse_observed_approval(
            "Codex Approval Guard (未响应)",
            fixture_lines(include_str!(
                "../../fixtures/ui_text/guard_self_unresponsive.txt"
            )),
            "fixture",
        );
        assert!(observed.is_none());
    }

    #[test]
    fn ignores_file_explorer_repo_window_fixture() {
        let observed = parse_observed_approval(
            "codex-approval-guard - 文件资源管理器",
            fixture_lines(include_str!(
                "../../fixtures/ui_text/file_explorer_repo_window.txt"
            )),
            "fixture",
        );
        assert!(observed.is_none());
    }

    #[test]
    fn parses_codex_desktop_chinese_pending_approval_fixture() {
        let observed = parse_observed_approval(
            "制定RAG开发计划",
            fixture_lines(include_str!(
                "../../fixtures/ui_text/codex_desktop_chinese_pending_approval.txt"
            )),
            "fixture",
        )
        .expect("fixture should parse");

        assert_eq!(
            observed.request.command.as_deref(),
            Some("pnpm --filter @ai-launchpad/browser build")
        );
        assert_eq!(
            observed.request.requested_permission.as_deref(),
            Some("shell")
        );
    }

    #[test]
    fn splits_concatenated_pnpm_commands() {
        let observed = parse_observed_approval_with_context(
            "制定RAG开发计划",
            vec![
                "已编辑 3 个文件".to_string(),
                "是否运行此命令?".to_string(),
                "pnpm --filter @ai-launchpad/browser typecheck pnpm --filter @ai-launchpad/browser lint pnpm --filter @ai-launchpad/browser test:knowledge pnpm --filter @ai-launchpad/browser build".to_string(),
                "1. 是".to_string(),
            ],
            "test",
            true,
        )
        .expect("should parse");

        assert_eq!(
            observed.request.command.as_deref(),
            Some("pnpm --filter @ai-launchpad/browser typecheck")
        );
    }

    #[test]
    fn extracts_windows_path_with_spaces() {
        let paths = extract_windows_paths_from_line(
            "cwd: D:\\Program Files\\Codex Approval Guard\\codex-approval-guard.exe",
        );
        assert_eq!(
            paths,
            vec!["D:\\Program Files\\Codex Approval Guard\\codex-approval-guard.exe".to_string()],
        );
    }

    #[test]
    fn extracts_multiple_windows_paths_separated_by_comma() {
        let paths = extract_windows_paths_from_line(
            "Targets: D:\\Program Files\\Foo Bar\\app.exe, E:\\workspace\\codex-approval-guard",
        );
        assert_eq!(
            paths,
            vec![
                "D:\\Program Files\\Foo Bar\\app.exe".to_string(),
                "E:\\workspace\\codex-approval-guard".to_string(),
            ],
        );
    }

    #[test]
    fn extracts_path_strips_trailing_punctuation() {
        let paths = extract_windows_paths_from_line(
            "保存先 C:\\Users\\thinkpad\\AppData\\Roaming\\com.engchina.codex-approval-guard.",
        );
        assert_eq!(
            paths,
            vec![
                "C:\\Users\\thinkpad\\AppData\\Roaming\\com.engchina.codex-approval-guard"
                    .to_string()
            ],
        );
    }

    #[test]
    fn changed_files_scoped_to_prompt_window() {
        let mut lines = vec![
            "ai-launchpad-for-oracle".to_string(),
            "src/store/useLaunchpadStore.ts +12 -3".to_string(),
            "apps/browser/main.ts +5 -2".to_string(),
        ];
        for index in 0..40 {
            lines.push(format!("chat history line {index}"));
        }
        lines.push("是否运行此命令?".to_string());
        lines.push("pnpm --filter @ai-launchpad/browser test:knowledge".to_string());

        let window = locate_prompt_window(&lines).expect("should locate prompt");
        let changed = extract_changed_files(window);
        assert!(
            changed.is_empty(),
            "diff history outside prompt window: {changed:?}"
        );
    }

    #[test]
    fn split_does_not_panic_on_lowercase_length_change() {
        // 土耳其語 İ (U+0130, 2 bytes UTF-8) は to_lowercase で
        // "i\u{307}" (3 bytes) に伸びる。旧実装は lower のバイト位置を
        // 元文字列に流用していたため、こうした入力で UTF-8 境界を
        // 割って panic する可能性があった。新実装はバイト長が変わる
        // 場合に lower 側を切るため、panic せず妥当な結果を返す。
        let _ = split_concatenated_commands("git status İ git log");
        // この入力では " git " が 2 か所現れるため cut が Some になり、
        // 切断結果は最初の "git status..." 相当に丸まる。具体形は
        // 大小文字正規化の影響を受けるが、ここでは panic しないことが目的。
    }

    #[test]
    fn splits_concatenated_commands_keeps_single_command_intact() {
        assert_eq!(
            split_concatenated_commands("pnpm --filter @ai-launchpad/browser build"),
            "pnpm --filter @ai-launchpad/browser build"
        );
        assert_eq!(
            split_concatenated_commands("npm test --workspace=foo"),
            "npm test --workspace=foo"
        );
    }

    #[test]
    fn trims_natural_language_note_after_npm_run_command() {
        assert_eq!(
            split_concatenated_commands("npm run web:test 确认前端现有核心行为不可回归."),
            "npm run web:test"
        );
        assert_eq!(
            split_concatenated_commands("npm run web:test -- --watch"),
            "npm run web:test -- --watch"
        );
    }

    #[test]
    fn parses_codex_desktop_chinese_apply_changes_fixture() {
        let observed = parse_observed_approval_with_context(
            "制定RAG开发计划",
            fixture_lines(include_str!(
                "../../fixtures/ui_text/codex_desktop_chinese_apply_changes.txt"
            )),
            "fixture",
            true,
        )
        .expect("fixture should parse");

        assert_eq!(observed.request.command, None);
        assert_eq!(
            observed.request.requested_permission.as_deref(),
            Some("file")
        );
        assert!(observed
            .request
            .target_paths
            .contains(&"index.ts".to_string()));
    }

    #[test]
    fn parses_codex_desktop_japanese_apply_changes_fixture() {
        let observed = parse_observed_approval_with_context(
            "RAG 開発計画の策定",
            fixture_lines(include_str!(
                "../../fixtures/ui_text/codex_desktop_japanese_apply_changes.txt"
            )),
            "fixture",
            true,
        )
        .expect("fixture should parse");

        assert_eq!(observed.request.command, None);
        assert_eq!(
            observed.request.requested_permission.as_deref(),
            Some("file")
        );
        assert!(observed
            .request
            .target_paths
            .contains(&"windows.rs".to_string()));
    }

    #[test]
    fn parses_english_command_approval_popup() {
        let lines = vec![
            "Command approval".to_string(),
            "Approval required".to_string(),
            "关闭".to_string(),
            "Approve".to_string(),
            "Approve for session".to_string(),
            "Decline".to_string(),
        ];
        let observed = parse_observed_approval_with_context("Codex", lines, "test", true)
            .expect("should parse English popup");
        assert_eq!(observed.request.command, None);
        assert_eq!(
            observed.request.requested_permission.as_deref(),
            Some("shell")
        );
    }

    #[test]
    fn apply_changes_prompt_ignores_git_add_in_chat_history() {
        // Codex メインウィンドウには過去のチャット履歴が大量に残っているため、
        // 「git add」のような shell コマンド文字列が混在する。ファイル編集の
        // 承認ダイアログ（apply changes）では command を抽出してはならない。
        let lines = vec![
            "Codex".to_string(),
            "已运行 git add".to_string(),
            "git add".to_string(),
            "已运行 git commit -m \"feat: ...\"".to_string(),
            "正在编辑 AppShell.tsx".to_string(),
            "是否应用这些更改?".to_string(),
            "AppShell.tsx +5 -0".to_string(),
            "1. 是".to_string(),
            "2. 是，且本次会话不再询问".to_string(),
            "3. 否，请告知 Codex 如何调整".to_string(),
            "跳过".to_string(),
            "提交".to_string(),
        ];
        let observed = parse_observed_approval_with_context("分析未完成任务", lines, "test", true)
            .expect("apply changes prompt should parse");

        assert_eq!(
            observed.request.command, None,
            "apply changes ダイアログでは command は抽出しない"
        );
    }

    #[test]
    fn detects_pending_approval_badges_in_known_languages() {
        for label in [
            "等待批准",
            "等待审批",
            "待批准",
            "承認待ち",
            "承認が必要",
            "要承認",
            "Awaiting approval",
            "Pending approval",
            "PENDING APPROVAL",
            "needs approval",
            "Waiting for approval",
            "approval pending",
        ] {
            assert!(
                is_pending_approval_badge(label),
                "should detect `{label}` as pending-approval badge"
            );
        }
    }

    #[test]
    fn does_not_treat_long_text_as_badge() {
        // ヘルプ文 / プロンプト本文中の偶然の一致を拾わないこと。
        let long = "This action is awaiting approval because the policy currently \
            requires manual confirmation for any command that writes outside the workspace.";
        assert!(!is_pending_approval_badge(long));
    }

    #[test]
    fn does_not_treat_unrelated_labels_as_badge() {
        for label in [
            "",
            "  ",
            "启动项目",
            "main",
            "1. 是",
            "提交",
            "Approve",
            "Codex Desktop",
        ] {
            assert!(
                !is_pending_approval_badge(label),
                "should NOT detect `{label}` as pending-approval badge"
            );
        }
    }

    #[test]
    fn parses_git_commit_window() {
        let lines = vec![
            "提交更改".to_string(),
            "分支 main".to_string(),
            "更改 46 个文件 +9,067 -0".to_string(),
            "包含取消暂存的更改".to_string(),
            "提交消息 自定义指令".to_string(),
            "留空以自动生成提交消息".to_string(),
            "继续".to_string(),
        ];
        assert!(looks_like_git_commit_window("提交更改", &lines));
        assert!(looks_like_git_commit_window("Codex", &lines));
    }

    #[test]
    fn parses_japanese_plan_confirmation_popup() {
        let lines = vec![
            "プラン".to_string(),
            "本地化改造方案".to_string(),
            "Summary".to_string(),
            "このプランを実施しますか？".to_string(),
            "1。 はい、このプランを実施します".to_string(),
            "2。 いいえ、Codex に何をすべきかを別の方法で指示してください".to_string(),
            "閉じる".to_string(),
            "送信する".to_string(),
        ];
        let observed = parse_observed_approval_with_context("Codex", lines, "test", true)
            .expect("should parse Japanese plan confirmation");

        assert_eq!(observed.request.command, None);
        assert_eq!(
            observed.request.requested_permission.as_deref(),
            Some("file")
        );
    }

    #[test]
    fn parses_chinese_plan_confirmation_popup() {
        let lines = vec![
            "方案".to_string(),
            "本地化改造方案".to_string(),
            "是否执行此方案？".to_string(),
            "1。 是，执行此方案".to_string(),
            "2。 否，请告知 Codex 如何调整".to_string(),
            "关闭".to_string(),
            "提交".to_string(),
        ];
        let observed = parse_observed_approval_with_context("Codex", lines, "test", true)
            .expect("should parse Chinese plan confirmation");

        assert_eq!(observed.request.command, None);
        assert_eq!(
            observed.request.requested_permission.as_deref(),
            Some("file")
        );
    }
}
