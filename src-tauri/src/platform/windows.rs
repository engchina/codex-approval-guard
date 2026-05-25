use crate::policy::ApprovalRequest;
use super::parser::{parse_observed_approval_with_context, looks_like_git_commit_window, is_pending_approval_badge};
use super::ClickOutcome;
use super::ObserveDiagnostics;
use super::ObservedApproval;
use super::PlatformSnapshot;

use std::{sync::mpsc, thread, time::Duration};
use uiautomation::patterns::{UIInvokePattern, UISelectionItemPattern, UILegacyIAccessiblePattern};
use uiautomation::types::ControlType;
use uiautomation::{UIAutomation, UIElement, UITreeWalker};
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        SystemInformation::GetTickCount,
        Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION},
    },
    UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
};

const UIA_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_TOP_LEVEL_WINDOWS: usize = 120;
const MAX_TEXT_LINES: usize = 1200;
const MAX_TREE_DEPTH: usize = 20;

pub fn snapshot_active_window() -> PlatformSnapshot {
    match run_uia_task(focused_window_summary) {
        Ok(Some(title)) => PlatformSnapshot {
            backend: "Windows UI Automation".to_string(),
            available: true,
            focused_window_title: Some(title),
            details: "UI Automation 監視は専用スレッドで利用可能です。".to_string(),
        },
        Ok(None) => PlatformSnapshot {
            backend: "Windows UI Automation".to_string(),
            available: true,
            focused_window_title: None,
            details: "Focused window を特定できませんでした。".to_string(),
        },
        Err(error) => PlatformSnapshot {
            backend: "Windows UI Automation".to_string(),
            available: false,
            focused_window_title: None,
            details: format!("UI Automation を初期化できません: {error}"),
        },
    }
}

pub fn observe_approval_request() -> Result<(Option<ObservedApproval>, ObserveDiagnostics), String>
{
    run_uia_task(observe_approval_request_inner)
}

pub fn click_yes_in_codex_approval() -> Result<ClickOutcome, String> {
    run_uia_task(click_yes_inner)
}

pub fn click_no_in_codex_approval() -> Result<ClickOutcome, String> {
    run_uia_task(click_no_inner)
}

/// 直近のユーザー入力（キーボード/マウス）から現在までの経過ミリ秒。
/// 取得に失敗した場合は 0 を返し「直前まで操作していた」とみなす（安全側）。
pub fn user_idle_ms() -> u32 {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    let ok = unsafe { GetLastInputInfo(&mut info) };
    if ok == 0 {
        return 0;
    }
    let now = unsafe { GetTickCount() };
    now.wrapping_sub(info.dwTime)
}

fn observe_approval_request_inner() -> Result<(Option<ObservedApproval>, ObserveDiagnostics), String>
{
    let automation = UIAutomation::new().map_err(|error| error.to_string())?;
    let control_walker = automation
        .get_control_view_walker()
        .map_err(|error| error.to_string())?;
    let content_walker = automation
        .get_content_view_walker()
        .map_err(|error| error.to_string())?;
    let raw_walker = automation
        .get_raw_view_walker()
        .map_err(|error| error.to_string())?;
    let mut diagnostics = ObserveDiagnostics::default();
    let result = find_observed_approval(
        &automation,
        &control_walker,
        &control_walker,
        &content_walker,
        &raw_walker,
        &mut diagnostics,
    )?;
    Ok((result, diagnostics))
}

fn run_uia_task<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("codex-approval-guard-uia".to_string())
        .spawn(move || {
            let _ = sender.send(task());
        })
        .map_err(|error| format!("UI Automation スレッドを開始できません: {error}"))?;

    receiver
        .recv_timeout(UIA_TIMEOUT)
        .map_err(|_| "UI Automation の読み取りがタイムアウトしました。".to_string())?
}

fn focused_window_summary() -> Result<Option<String>, String> {
    let automation = UIAutomation::new().map_err(|error| error.to_string())?;
    Ok(automation.get_focused_element().ok().map(|element| {
        let name = safe_name(&element);
        if name.trim().is_empty() {
            "Focused element name is empty".to_string()
        } else {
            name
        }
    }))
}

fn find_observed_approval(
    automation: &UIAutomation,
    top_level_walker: &UITreeWalker,
    control_walker: &UITreeWalker,
    content_walker: &UITreeWalker,
    raw_walker: &UITreeWalker,
    diagnostics: &mut ObserveDiagnostics,
) -> Result<Option<ObservedApproval>, String> {
    let root = automation
        .get_root_element()
        .map_err(|error| error.to_string())?;
    let Some(windows) = top_level_walker.get_children(&root) else {
        diagnostics
            .notes
            .push("Top-level walker から子要素を取得できません。".to_string());
        return Ok(None);
    };
    diagnostics.windows_scanned = windows.len();

    for window in windows.iter().take(MAX_TOP_LEVEL_WINDOWS) {
        let candidate = window_candidate(window);
        let codex_process = candidate.looks_like_codex_process();
        let skip = candidate.should_skip();
        let title_lower = candidate.title.to_lowercase();
        let title_mentions_codex = title_lower.contains("codex") || title_lower.contains("openai");
        if codex_process || title_mentions_codex {
            diagnostics.notes.push(format!(
                "候補: title=\"{}\" class=\"{}\" process={} codex_process={} skip={}",
                short(&candidate.title, 60),
                short(&candidate.class_name, 40),
                candidate.process_name.as_deref().unwrap_or("?"),
                codex_process,
                skip,
            ));
        }
        if skip || !codex_process {
            continue;
        }

        if let Some(observed) = parse_window(
            window,
            &candidate.title,
            true,
            &[
                ("content", content_walker),
                ("control", control_walker),
                ("raw-codex", raw_walker),
            ],
            diagnostics,
        ) {
            return Ok(Some(observed));
        }
    }

    Ok(None)
}

fn parse_window(
    window: &UIElement,
    title: &str,
    trusted_codex_context: bool,
    walkers: &[(&str, &UITreeWalker)],
    diagnostics: &mut ObserveDiagnostics,
) -> Option<ObservedApproval> {
    for (view_name, walker) in walkers {
        let mut raw_text = Vec::new();
        let mut keyword_hit = false;
        collect_text(walker, window, 0, &mut raw_text, &mut keyword_hit);
        diagnostics.notes.push(format!(
            "  parse: title=\"{}\" view={} lines={} keyword={}",
            short(title, 40),
            view_name,
            raw_text.len(),
            keyword_hit,
        ));

        // Git commit window check
        if looks_like_git_commit_window(title, &raw_text) {
            let prompt_text = raw_text.join("\n");
            return Some(ObservedApproval {
                request: ApprovalRequest {
                    id: None,
                    source_app: "Codex Desktop".to_string(),
                    window_title: title.to_string(),
                    prompt_text,
                    command: Some("git commit (dialog)".to_string()),
                    cwd: None,
                    target_paths: Vec::new(),
                    requested_permission: Some("git_commit_dismiss".to_string()),
                },
                raw_text,
                detected_by: format!("Windows UI Automation {view_name} view observer (git commit)"),
            });
        }

        // 真正に审批弹窗が存在する場合のみ強キーワードが現れる。
        // 単なる Codex メインウィンドウ（説明文に "approve" / "permission" などが
        // 散在する）を誤検出しないよう、ここで弾く。
        if !keyword_hit {
            continue;
        }
        if let Some(observed) = parse_observed_approval_with_context(
            title,
            raw_text,
            &format!("Windows UI Automation {view_name} view observer"),
            trusted_codex_context,
        ) {
            return Some(observed);
        }
    }

    None
}

const MAX_CLICK_TREE_DEPTH: usize = 60;
const MAX_DUMP_ENTRIES: usize = 80;

fn click_no_inner() -> Result<ClickOutcome, String> {
    let automation = UIAutomation::new().map_err(|error| error.to_string())?;
    let control_walker = automation
        .get_control_view_walker()
        .map_err(|error| error.to_string())?;
    let raw_walker = automation
        .get_raw_view_walker()
        .map_err(|error| error.to_string())?;
    let root = automation
        .get_root_element()
        .map_err(|error| error.to_string())?;
    let Some(windows) = control_walker.get_children(&root) else {
        return Err("Top-level walker から子要素を取得できません。".to_string());
    };

    let mut outcome = ClickOutcome::default();

    for window in windows.iter().take(MAX_TOP_LEVEL_WINDOWS) {
        let candidate = window_candidate(window);
        if candidate.should_skip() || !candidate.looks_like_codex_process() {
            continue;
        }

        outcome.target_window = candidate.title.clone();
        outcome.notes.push(format!(
            "ターゲット: title=\"{}\" process={}",
            short(&candidate.title, 60),
            candidate.process_name.as_deref().unwrap_or("?"),
        ));

        // Collect raw text to determine if it is a git commit window
        let mut raw_text = Vec::new();
        let mut keyword_hit = false;
        collect_text(&raw_walker, window, 0, &mut raw_text, &mut keyword_hit);

        let is_git_commit = looks_like_git_commit_window(&candidate.title, &raw_text);

        let (matcher, label) = if is_git_commit {
            (is_close_or_cancel_button as fn(&str) -> bool, "「关闭/取消」")
        } else {
            (is_first_no_option as fn(&str) -> bool, "「3. 否」")
        };

        let no_candidates = collect_candidates(&raw_walker, window, matcher);
        log_candidates(&mut outcome, label, &no_candidates);
        let no_target = match pick_best(&no_candidates) {
            Some(target) => target,
            None => {
                let dump = dump_approval_tree(&raw_walker, window);
                let dump_text = if dump.is_empty() {
                    "（承認関連要素なし）".to_string()
                } else {
                    dump.join("\n")
                };
                return Err(format!(
                    "{}相当の要素が見つかりませんでした。Codex のダイアログが現在表示されていない可能性があります。\n\nUIA dump (承認関連 / interactive 要素):\n{}",
                    label, dump_text
                ));
            }
        };
        let no_method = invoke_candidate(no_target, window)
            .map_err(|error| format!("{}の invoke に失敗しました: {}", label, error))?;
        outcome.notes.push(format!(
            "{}invoke: type={:?} method={}",
            label, no_target.control_type, no_method
        ));
        outcome.yes_invoked = true;

        if is_git_commit {
            outcome.notes.push("Git 提交ダイアログのため「提交」処理をスキップします。".to_string());
            return Ok(outcome);
        }

        std::thread::sleep(std::time::Duration::from_millis(150));

        let submit_candidates = collect_candidates(&raw_walker, window, is_submit_button);
        log_candidates(&mut outcome, "「提交」", &submit_candidates);
        match pick_best(&submit_candidates) {
            Some(submit_target) => {
                let submit_method = invoke_candidate(submit_target, window).map_err(|error| {
                    format!(
                        "「3. 否」は選択しましたが、「提交」の invoke に失敗しました: {error}。手動で送信してください。"
                    )
                })?;
                outcome.notes.push(format!(
                    "「提交」invoke: type={:?} method={}",
                    submit_target.control_type, submit_method
                ));
                outcome.submit_invoked = true;
            }
            None => {
                outcome.notes.push(
                    "「提交」ボタンが見つかりませんでした（既に送信された可能性）。".to_string(),
                );
            }
        }

        return Ok(outcome);
    }

    Err("Codex プロセスのウィンドウが見つかりませんでした。".to_string())
}

fn click_yes_inner() -> Result<ClickOutcome, String> {
    let automation = UIAutomation::new().map_err(|error| error.to_string())?;
    let control_walker = automation
        .get_control_view_walker()
        .map_err(|error| error.to_string())?;
    let raw_walker = automation
        .get_raw_view_walker()
        .map_err(|error| error.to_string())?;
    let root = automation
        .get_root_element()
        .map_err(|error| error.to_string())?;
    let Some(windows) = control_walker.get_children(&root) else {
        return Err("Top-level walker から子要素を取得できません。".to_string());
    };

    let mut outcome = ClickOutcome::default();

    for window in windows.iter().take(MAX_TOP_LEVEL_WINDOWS) {
        let candidate = window_candidate(window);
        if candidate.should_skip() || !candidate.looks_like_codex_process() {
            continue;
        }

        outcome.target_window = candidate.title.clone();
        outcome.notes.push(format!(
            "ターゲット: title=\"{}\" process={}",
            short(&candidate.title, 60),
            candidate.process_name.as_deref().unwrap_or("?"),
        ));

        // Collect raw text to determine if it is a git commit window
        let mut raw_text = Vec::new();
        let mut keyword_hit = false;
        collect_text(&raw_walker, window, 0, &mut raw_text, &mut keyword_hit);

        let is_git_commit = looks_like_git_commit_window(&candidate.title, &raw_text);

        let (matcher, label) = if is_git_commit {
            (is_close_or_cancel_button as fn(&str) -> bool, "「关闭/取消」")
        } else {
            (is_first_yes_option as fn(&str) -> bool, "「1. 是」")
        };

        let yes_candidates = collect_candidates(&raw_walker, window, matcher);
        log_candidates(&mut outcome, label, &yes_candidates);
        let yes_target = match pick_best(&yes_candidates) {
            Some(target) => target,
            None => {
                // 承認ダイアログ本体（「1. 是」「Approve」等）が UI ツリーに無い場合、
                // 非アクティブな会話がサイドバーで承認待ちになっている可能性がある。
                // バッジを手掛かりにサイドバーの会話アイテムをアクティブ化し、
                // 次回ポーリングで通常フローに合流させる。git commit ダイアログ側
                // （is_git_commit）はサイドバーバッジを持たないため対象外。
                if !is_git_commit {
                    let sidebar_targets =
                        collect_pending_approval_sidebar_targets(&raw_walker, window);
                    log_candidates(&mut outcome, "「等待批准」バッジ", &sidebar_targets);
                    if let Some(sidebar_target) = pick_best(&sidebar_targets) {
                        let activate_method = invoke_candidate(sidebar_target, window).map_err(
                            |error| {
                                format!(
                                    "サイドバーの承認待ち会話の自動アクティブ化に失敗しました: {error}"
                                )
                            },
                        )?;
                        outcome.notes.push(format!(
                            "サイドバーの承認待ち会話をアクティブ化しました: type={:?} method={} name=\"{}\"。次回ポーリングで承認操作を試行します。",
                            sidebar_target.control_type,
                            activate_method,
                            short(&sidebar_target.name, 60),
                        ));
                        // ここではダイアログ本体がまだ描画されていないため、
                        // yes_invoked / submit_invoked は false のまま返す。
                        // 次サイクルで `click_yes_inner` が再度呼ばれた時に通常フローが走る。
                        return Ok(outcome);
                    }
                }
                let dump = dump_approval_tree(&raw_walker, window);
                let dump_text = if dump.is_empty() {
                    "（承認関連要素なし）".to_string()
                } else {
                    dump.join("\n")
                };
                return Err(format!(
                    "{}相当の要素が見つかりませんでした。Codex のダイアログが現在表示されていない可能性があります。\n\nUIA dump (承認関連 / interactive 要素):\n{}",
                    label, dump_text
                ));
            }
        };
        let yes_method = invoke_candidate(yes_target, window)
            .map_err(|error| format!("{}の invoke に失敗しました: {}", label, error))?;
        outcome.notes.push(format!(
            "{}invoke: type={:?} method={}",
            label, yes_target.control_type, yes_method
        ));
        outcome.yes_invoked = true;

        if is_git_commit {
            outcome.notes.push("Git 提交ダイアログのため「提交」処理をスキップします。".to_string());
            return Ok(outcome);
        }

        std::thread::sleep(std::time::Duration::from_millis(150));

        let submit_candidates = collect_candidates(&raw_walker, window, is_submit_button);
        log_candidates(&mut outcome, "「提交」", &submit_candidates);
        match pick_best(&submit_candidates) {
            Some(submit_target) => {
                let submit_method = invoke_candidate(submit_target, window).map_err(|error| {
                    format!(
                        "「1. 是」は選択しましたが、「提交」の invoke に失敗しました: {error}。手動で送信してください。"
                    )
                })?;
                outcome.notes.push(format!(
                    "「提交」invoke: type={:?} method={}",
                    submit_target.control_type, submit_method
                ));
                outcome.submit_invoked = true;
            }
            None => {
                outcome.notes.push(
                    "「提交」ボタンが見つかりませんでした（既に送信された可能性）。".to_string(),
                );
            }
        }

        return Ok(outcome);
    }

    Err("Codex プロセスのウィンドウが見つかりませんでした。".to_string())
}

struct Candidate {
    element: UIElement,
    name: String,
    control_type: Option<ControlType>,
    has_invoke: bool,
    has_select: bool,
    has_legacy: bool,
}

impl Candidate {
    fn is_interactive_type(&self) -> bool {
        matches!(
            self.control_type,
            Some(ControlType::Button)
                | Some(ControlType::ListItem)
                | Some(ControlType::RadioButton)
                | Some(ControlType::MenuItem)
                | Some(ControlType::TabItem)
                | Some(ControlType::Hyperlink)
                | Some(ControlType::CheckBox)
                | Some(ControlType::SplitButton)
        )
    }

    fn score(&self) -> i32 {
        let mut score = 0;
        if self.is_interactive_type() {
            score += 10;
        }
        if self.has_invoke {
            score += 5;
        }
        if self.has_select {
            score += 3;
        }
        if self.has_legacy {
            score += 2;
        }
        score
    }
}

fn is_system_caption_or_titlebar_element(element: &UIElement) -> bool {
    if let Ok(class_name) = element.get_classname() {
        let class_lower = class_name.to_lowercase();
        if class_lower.contains("captionbutton")
            || class_lower.contains("titlebar")
            || class_lower.contains("sysmenu")
        {
            return true;
        }
    }
    if let Ok(control_type) = element.get_control_type() {
        if control_type == ControlType::TitleBar || control_type == ControlType::MenuBar {
            return true;
        }
    }
    false
}

fn collect_candidates(
    walker: &UITreeWalker,
    root: &UIElement,
    matcher: impl Fn(&str) -> bool,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    collect_candidates_recursive(walker, root, 0, &matcher, &mut out);
    out
}

fn collect_candidates_recursive(
    walker: &UITreeWalker,
    element: &UIElement,
    depth: usize,
    matcher: &impl Fn(&str) -> bool,
    out: &mut Vec<Candidate>,
) {
    if depth > MAX_CLICK_TREE_DEPTH || out.len() >= 30 {
        return;
    }
    let name = safe_name(element);
    if matcher(&name) && !is_system_caption_or_titlebar_element(element) {
        out.push(Candidate {
            element: element.clone(),
            name,
            control_type: element.get_control_type().ok(),
            has_invoke: element.get_pattern::<UIInvokePattern>().is_ok(),
            has_select: element.get_pattern::<UISelectionItemPattern>().is_ok(),
            has_legacy: element.get_pattern::<UILegacyIAccessiblePattern>().is_ok(),
        });
    }
    if let Some(children) = walker.get_children(element) {
        for child in children {
            collect_candidates_recursive(walker, &child, depth + 1, matcher, out);
        }
    }
}

fn pick_best(candidates: &[Candidate]) -> Option<&Candidate> {
    candidates.iter().max_by_key(|c| c.score())
}

/// サイドバーから「承認待ち」バッジを持つ会話アイテムを探し、
/// クリック可能な祖先（ListItem / TabItem 等）を `Candidate` として返す。
///
/// バッジ自体（TextBlock）には Invoke パターンが無いことが多いため、
/// 祖先方向に最も近い「Invoke もしくは Select 可能なコンテナ」を選ぶ。
/// 見つからなければバッジ要素そのものを返す（最後の手段）。
fn collect_pending_approval_sidebar_targets(
    walker: &UITreeWalker,
    window: &UIElement,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    collect_pending_approval_recursive(walker, window, 0, &mut Vec::new(), &mut out);
    out
}

fn collect_pending_approval_recursive(
    walker: &UITreeWalker,
    element: &UIElement,
    depth: usize,
    ancestor_stack: &mut Vec<UIElement>,
    out: &mut Vec<Candidate>,
) {
    if depth > MAX_CLICK_TREE_DEPTH || out.len() >= 10 {
        return;
    }

    let name = safe_name(element);
    if is_pending_approval_badge(&name) {
        let target = pick_clickable_ancestor(ancestor_stack).unwrap_or_else(|| element.clone());
        let candidate = Candidate {
            name: safe_name(&target),
            control_type: target.get_control_type().ok(),
            has_invoke: target.get_pattern::<UIInvokePattern>().is_ok(),
            has_select: target.get_pattern::<UISelectionItemPattern>().is_ok(),
            has_legacy: target.get_pattern::<UILegacyIAccessiblePattern>().is_ok(),
            element: target,
        };
        out.push(candidate);
    }

    ancestor_stack.push(element.clone());
    if let Some(children) = walker.get_children(element) {
        for child in children {
            collect_pending_approval_recursive(walker, &child, depth + 1, ancestor_stack, out);
        }
    }
    ancestor_stack.pop();
}

fn pick_clickable_ancestor(stack: &[UIElement]) -> Option<UIElement> {
    for ancestor in stack.iter().rev() {
        let control_type = ancestor.get_control_type().ok();
        let is_clickable_container = matches!(
            control_type,
            Some(ControlType::ListItem)
                | Some(ControlType::TabItem)
                | Some(ControlType::TreeItem)
                | Some(ControlType::Button)
                | Some(ControlType::Hyperlink)
                | Some(ControlType::DataItem)
                | Some(ControlType::MenuItem)
        );
        if !is_clickable_container {
            continue;
        }
        let has_invoke = ancestor.get_pattern::<UIInvokePattern>().is_ok();
        let has_select = ancestor.get_pattern::<UISelectionItemPattern>().is_ok();
        let has_legacy = ancestor.get_pattern::<UILegacyIAccessiblePattern>().is_ok();
        if has_invoke || has_select || has_legacy {
            return Some(ancestor.clone());
        }
    }
    None
}

fn log_candidates(outcome: &mut ClickOutcome, label: &str, candidates: &[Candidate]) {
    outcome
        .notes
        .push(format!("{label} 候補: {} 件", candidates.len()));
    for (i, c) in candidates.iter().take(5).enumerate() {
        outcome.notes.push(format!(
            "  [{}] name=\"{}\" type={:?} invoke={} select={} legacy={} score={}",
            i,
            short(&c.name, 60),
            c.control_type,
            c.has_invoke,
            c.has_select,
            c.has_legacy,
            c.score(),
        ));
    }
}

fn dump_approval_tree(walker: &UITreeWalker, window: &UIElement) -> Vec<String> {
    let mut out = Vec::new();
    dump_approval_tree_recursive(walker, window, 0, &mut out);
    out
}

fn dump_approval_tree_recursive(
    walker: &UITreeWalker,
    element: &UIElement,
    depth: usize,
    out: &mut Vec<String>,
) {
    if depth > MAX_CLICK_TREE_DEPTH || out.len() >= MAX_DUMP_ENTRIES {
        return;
    }
    let name = safe_name(element);
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    let control_type = element.get_control_type().ok();
    let is_interactive_type = matches!(
        control_type,
        Some(ControlType::Button)
            | Some(ControlType::ListItem)
            | Some(ControlType::RadioButton)
            | Some(ControlType::CheckBox)
            | Some(ControlType::MenuItem)
            | Some(ControlType::Hyperlink)
            | Some(ControlType::TabItem)
            | Some(ControlType::SplitButton)
    );
    let has_approval_keyword = !trimmed.is_empty()
        && (trimmed.contains("是")
            || trimmed.contains("否")
            || trimmed.contains("提交")
            || trimmed.contains("跳过")
            || trimmed.contains("承認")
            || trimmed.contains("拒否")
            || trimmed.contains("送信")
            || trimmed.contains("確認")
            || trimmed.contains("はい")
            || trimmed.contains("いいえ")
            || lower.contains("yes")
            || lower.contains("no")
            || lower.contains("approve")
            || lower.contains("deny")
            || lower.contains("submit")
            || lower.contains("ok")
            || lower.contains("cancel"));
    let interactive_with_name = is_interactive_type && !trimmed.is_empty();
    if has_approval_keyword || interactive_with_name {
        let has_invoke = element.get_pattern::<UIInvokePattern>().is_ok();
        let has_select = element.get_pattern::<UISelectionItemPattern>().is_ok();
        out.push(format!(
            "  d={:02} name=\"{}\" type={:?} invoke={} select={}",
            depth,
            short(trimmed, 80),
            control_type,
            has_invoke,
            has_select,
        ));
    }
    if let Some(children) = walker.get_children(element) {
        for child in children {
            dump_approval_tree_recursive(walker, &child, depth + 1, out);
        }
    }
}

#[allow(dead_code)] // 物理クリックフォールバック無効化のため現状未使用。将来復活時に使用する。
fn ensure_window_active(window: &UIElement) {
    let Ok(handle) = window.get_native_window_handle() else {
        return;
    };
    let hwnd_win: windows::Win32::Foundation::HWND = handle.into();
    let hwnd_raw = hwnd_win.0 as windows_sys::Win32::Foundation::HWND;
    if hwnd_raw.is_null() {
        return;
    }
    unsafe {
        if windows_sys::Win32::UI::WindowsAndMessaging::IsIconic(hwnd_raw) != 0 {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd_raw,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_RESTORE,
            );
            std::thread::sleep(Duration::from_millis(150));
        }
        windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
            hwnd_raw,
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
        );
        windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd_raw);
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn click_element_background(element: &UIElement) -> Result<(), String> {
    let handle = element.get_native_window_handle()
        .map_err(|error| format!("ウィンドウハンドル取得失敗: {error}"))?;
    let hwnd_win: windows::Win32::Foundation::HWND = handle.into();
    let hwnd_raw = hwnd_win.0 as windows_sys::Win32::Foundation::HWND;
    if hwnd_raw.is_null() {
        return Err("HWND が NULL です。".to_string());
    }

    let pt = element.get_clickable_point()
        .map_err(|error| format!("Clickable point 取得失敗: {error}"))?
        .ok_or_else(|| "Clickable point が取得できませんでした。".to_string())?;

    let mut client_pt = windows_sys::Win32::Foundation::POINT { x: pt.get_x(), y: pt.get_y() };
    unsafe {
        if windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd_raw, &mut client_pt) == 0 {
            return Err("ScreenToClient 変換に失敗しました。".to_string());
        }
    }

    let l_param = ((client_pt.y as u32) << 16) | ((client_pt.x as u32) & 0xFFFF);
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            hwnd_raw,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN,
            1, // MK_LBUTTON
            l_param as isize,
        );
        std::thread::sleep(Duration::from_millis(50));
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            hwnd_raw,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
            0,
            l_param as isize,
        );
    }

    Ok(())
}

fn invoke_candidate(candidate: &Candidate, parent_window: &UIElement) -> Result<&'static str, String> {
    if candidate.has_invoke {
        if let Ok(pattern) = candidate.element.get_pattern::<UIInvokePattern>() {
            if let Err(error) = pattern.invoke() {
                return Err(format!("InvokePattern.invoke 失敗: {error}"));
            }
            return Ok("invoke");
        }
    }
    if candidate.has_select {
        if let Ok(pattern) = candidate.element.get_pattern::<UISelectionItemPattern>() {
            if let Err(error) = pattern.select() {
                return Err(format!("SelectionItemPattern.select 失敗: {error}"));
            }
            return Ok("selection-item");
        }
    }
    if candidate.has_legacy {
        if let Ok(pattern) = candidate.element.get_pattern::<UILegacyIAccessiblePattern>() {
            if let Ok(()) = pattern.do_default_action() {
                return Ok("legacy-accessible");
            }
        }
    }

    // UIA パターンが失敗した場合、PostMessage によるバックグラウンドクリックを試行
    if let Ok(()) = click_element_background(&candidate.element) {
        return Ok("background-click");
    }

    // 物理クリックフォールバックは無効化されているため、UIA / BackgroundClick 共に失敗した場合は手動操作を委ねる
    let _ = parent_window;
    Err(
        "UIA パターンおよびバックグラウンドクリックによる自動操作が利用できませんでした。手動で操作してください。"
            .to_string(),
    )
}

fn is_close_or_cancel_button(name: &str) -> bool {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    trimmed == "关闭"
        || trimmed == "閉じる"
        || trimmed == "取消"
        || trimmed == "キャンセル"
        || trimmed == "跳过"
        || trimmed == "スキップ"
        || trimmed == "X"
        || trimmed == "x"
        || trimmed == "✕"
        || trimmed == "✖"
        || trimmed == "×"
        || trimmed == "⨯"
        || lower == "close"
        || lower == "cancel"
        || lower == "dismiss"
        || lower == "skip"
        || lower == "close dialog"
        || lower == "close modal"
        || lower == "关闭对话框"
}

fn is_first_yes_option(name: &str) -> bool {
    let trimmed = name.trim();
    if is_standalone_primary_approval_label(trimmed) {
        return true;
    }

    let starts_with_one = trimmed.starts_with("1.")
        || trimmed.starts_with("1、")
        || trimmed.starts_with("1。")
        || trimmed.starts_with("1)")
        || trimmed.starts_with("1 .");

    starts_with_one && looks_like_primary_approval_option(trimmed)
}

fn is_standalone_primary_approval_label(label: &str) -> bool {
    let lower = label.to_lowercase();
    matches!(label, "是" | "承認" | "批准" | "はい")
        || lower == "yes"
        || lower == "approve"
        || lower == "allow"
}

fn looks_like_primary_approval_option(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.contains("是")
        || name.contains("承認")
        || name.contains("批准")
        || name.contains("確認")
        || name.contains("はい")
        || lower.contains("approve")
        || lower.contains("yes")
        || lower.contains("allow")
}

fn is_first_no_option(name: &str) -> bool {
    let trimmed = name.trim();
    if is_standalone_primary_rejection_label(trimmed) {
        return true;
    }

    let starts_with_three = trimmed.starts_with("3.")
        || trimmed.starts_with("3、")
        || trimmed.starts_with("3。")
        || trimmed.starts_with("3)")
        || trimmed.starts_with("3 .");

    starts_with_three && looks_like_primary_rejection_option(trimmed)
}

fn is_standalone_primary_rejection_label(label: &str) -> bool {
    let lower = label.to_lowercase();
    matches!(label, "否" | "拒否" | "拒绝" | "いいえ")
        || lower == "no"
        || lower == "deny"
        || lower == "decline"
        || lower == "reject"
}

fn looks_like_primary_rejection_option(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.contains("否")
        || name.contains("拒否")
        || name.contains("拒绝")
        || name.contains("いいえ")
        || lower.contains("deny")
        || lower.contains("no")
        || lower.contains("decline")
        || lower.contains("reject")
}

fn is_submit_button(name: &str) -> bool {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    trimmed == "提交"
        || trimmed.starts_with("提交 ")
        || trimmed == "送信"
        || trimmed.starts_with("送信 ")
        || trimmed == "確認"
        || trimmed.starts_with("確認 ")
        || lower == "submit"
        || lower.starts_with("submit ")
        || lower == "ok"
        || lower.starts_with("ok ")
}

fn looks_like_approval_keyword(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("approval request")
        || lower.contains("command approval")
        || lower.contains("approval required")
        || lower.contains("approve")
        || lower.contains("approval")
        || lower.contains("apply these changes")
        || lower.contains("apply changes")
        || lower.contains("run command")
        || lower.contains("permission to run")
        || line.contains("是否应用")
        || line.contains("是否运行")
        || line.contains("変更を適用")
        || line.contains("これらの変更")
        || line.contains("承認")
        // サイドバーのバッジ（非アクティブ会話での承認待ち）も拾うため、
        // バッジ文字列単体を keyword_hit のトリガに含める。判定本体は
        // parser::is_pending_approval_badge と整合させる。
        || super::parser::is_pending_approval_badge(line)
}

fn short(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        text.to_string()
    } else {
        let mut out: String = chars.iter().take(max).collect();
        out.push('…');
        out
    }
}

fn collect_text(
    walker: &UITreeWalker,
    element: &UIElement,
    depth: usize,
    output: &mut Vec<String>,
    keyword_hit: &mut bool,
) {
    if depth > MAX_TREE_DEPTH || output.len() >= MAX_TEXT_LINES {
        return;
    }

    if !element.is_password().unwrap_or(false) {
        for line in push_clean(output, safe_name(element)) {
            if !*keyword_hit && looks_like_approval_keyword(&line) {
                *keyword_hit = true;
            }
        }
    }

    if let Some(children) = walker.get_children(element) {
        for child in children {
            collect_text(walker, &child, depth + 1, output, keyword_hit);
        }
    }
}

fn push_clean(output: &mut Vec<String>, value: String) -> Vec<String> {
    let mut pushed = Vec::new();
    for line in value.split(|character: char| character == '\n' || character == '\r') {
        let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.len() >= 2 && !output.iter().any(|existing| existing == &normalized) {
            output.push(normalized.clone());
            pushed.push(normalized);
        }
    }
    pushed
}

fn safe_name(element: &UIElement) -> String {
    element.get_name().unwrap_or_default()
}

fn safe_class_name(element: &UIElement) -> String {
    element.get_classname().unwrap_or_default()
}

fn safe_process_id(element: &UIElement) -> Option<u32> {
    element.get_process_id().ok()
}

fn safe_process_image(element: &UIElement) -> Option<String> {
    safe_process_id(element).and_then(process_image_path)
}

fn safe_process_name(element: &UIElement) -> Option<String> {
    safe_process_id(element).and_then(process_executable_name)
}

fn process_image_path(process_id: u32) -> Option<String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if handle.is_null() {
        return None;
    }

    let mut buffer = vec![0u16; 32_768];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) };
    unsafe {
        CloseHandle(handle);
    }

    if ok == 0 || size == 0 {
        return None;
    }

    Some(String::from_utf16_lossy(&buffer[..size as usize]))
}

fn process_executable_name(process_id: u32) -> Option<String> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut found = None;
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while ok {
        if entry.th32ProcessID == process_id {
            found = Some(wide_array_to_string(&entry.szExeFile));
            break;
        }

        ok = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }

    unsafe {
        CloseHandle(snapshot);
    }

    found
}

fn wide_array_to_string(value: &[u16]) -> String {
    let len = value
        .iter()
        .position(|char| *char == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..len])
}

fn window_candidate(window: &UIElement) -> WindowCandidate {
    WindowCandidate {
        title: safe_name(window),
        class_name: safe_class_name(window),
        process_image: safe_process_image(window),
        process_name: safe_process_name(window),
    }
}

#[derive(Debug)]
struct WindowCandidate {
    title: String,
    class_name: String,
    process_image: Option<String>,
    process_name: Option<String>,
}

impl WindowCandidate {
    fn should_skip(&self) -> bool {
        let title = self.title.to_lowercase();
        let class_name = self.class_name.to_lowercase();
        let process_image = self
            .process_image
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();
        let process_name = self
            .process_name
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();

        title.contains("codex approval guard")
            || process_image.contains("codex-approval-guard")
            || process_name.contains("codex-approval-guard")
            || title.contains("file explorer")
            || title.contains("文件资源管理器")
            || title.contains("エクスプローラー")
            || title == "program manager"
            || class_name == "cabinetwclass"
            || class_name == "explorewclass"
    }

    fn looks_like_codex_process(&self) -> bool {
        let process_image = self
            .process_image
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();
        let process_name = self
            .process_name
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();
        let is_codex_process_name =
            process_name == "codex.exe" || process_name == "codex" || process_name == "chatgpt.exe";

        is_codex_process_name
            || process_image.contains("\\openai.codex_")
            || process_image.contains("/openai.codex_")
            || process_image.contains("\\openai\\codex\\")
            || process_image.contains("/openai/codex/")
            || process_image.contains("\\app\\codex.exe")
            || process_image.contains("/app/codex.exe")
            || process_image.contains("\\resources\\codex.exe")
            || process_image.contains("/resources/codex.exe")
            || process_image.ends_with("\\codex.exe")
            || process_image.ends_with("/codex.exe")
            || process_image.ends_with("\\chatgpt.exe")
            || process_image.ends_with("/chatgpt.exe")
            || process_image.contains("\\codex\\")
            || process_image.contains("/codex/")
            || process_image.contains("\\chatgpt\\")
            || process_image.contains("/chatgpt/")
            || process_image.contains("\\openai\\")
            || process_image.contains("/openai/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_numbered_and_accessibility_stripped_yes_option() {
        assert!(is_first_yes_option("1. 是"));
        assert!(is_first_yes_option("1。是"));
        assert!(is_first_yes_option("是"));
        assert!(is_first_yes_option("Yes"));
        assert!(is_first_yes_option("Approve"));
    }

    #[test]
    fn matches_numbered_and_accessibility_stripped_no_option() {
        assert!(is_first_no_option("3. 否"));
        assert!(is_first_no_option("3。否"));
        assert!(is_first_no_option("否"));
        assert!(is_first_no_option("No"));
        assert!(is_first_no_option("Decline"));
        assert!(is_first_no_option("3. 否，请告知 Codex 如何調整"));
    }

    #[test]
    fn does_not_match_remembered_approval_or_denial_options() {
        assert!(!is_first_yes_option("是，且本次会话不再询问"));
        assert!(!is_first_yes_option("2. 是，且本次会话不再询问"));
        assert!(!is_first_yes_option("3. 否，请告知 Codex 如何调整"));
        assert!(!is_first_yes_option("提交"));
    }

    #[test]
    fn matches_close_or_cancel_button_variants() {
        for name in [
            "关闭", "閉じる", "取消", "キャンセル", "跳过", "スキップ",
            "X", "x", "✕", "✖", "×", "⨯",
            "Close", "close", "Cancel", "Dismiss", "Skip",
            "Close dialog", "Close Modal", "关闭对话框",
        ] {
            assert!(is_close_or_cancel_button(name), "should match `{name}`");
        }
    }

    #[test]
    fn does_not_match_unrelated_buttons_as_close() {
        for name in ["提交", "继续", "Submit", "Continue", "确认", "1. 是"] {
            assert!(
                !is_close_or_cancel_button(name),
                "should not match `{name}` as close/cancel"
            );
        }
    }

    #[test]
    fn matches_submit_button_with_shortcut_hint() {
        assert!(is_submit_button("提交"));
        assert!(is_submit_button("提交 ⏎"));
        assert!(is_submit_button("Submit Enter"));
        assert!(!is_submit_button("跳过 提交 ⏎"));
    }

    /// 非アクティブな会話のサイドバーに「等待批准」バッジしか出ていない
    /// ケースでも、observe フェーズの keyword_hit が立つことを担保する。
    /// これが false のままだと parse_window の早期 continue で観察結果が
    /// 返らず、自動承認のトリガが掛からない。
    #[test]
    fn pending_approval_badge_triggers_keyword_hit() {
        assert!(looks_like_approval_keyword("等待批准"));
        assert!(looks_like_approval_keyword("Pending approval"));
        assert!(looks_like_approval_keyword("承認待ち"));
        // 無関係なサイドバーラベルは引っ掛けない。
        assert!(!looks_like_approval_keyword("启动项目"));
        assert!(!looks_like_approval_keyword("main"));
    }
}
