use super::matchers::{
    is_close_or_cancel_button, is_first_no_option, is_first_yes_or_recommended_option,
    is_recommended_option, is_submit_button, looks_like_approval_keyword,
};
use super::parser::{
    is_pending_approval_badge, looks_like_git_commit_window, parse_observed_approval_with_context,
    title_matches_git_commit,
};
use super::ClickOutcome;
use super::ObserveDiagnostics;
use super::ObservedApproval;
use super::PlatformSnapshot;
use crate::policy::ApprovalRequest;

use std::{sync::mpsc, thread, time::Duration};
use uiautomation::patterns::{UIInvokePattern, UILegacyIAccessiblePattern, UISelectionItemPattern};
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

/// click 経路で利用する候補マッチ設定。matcher は要素名、type_filter は control_type を絞る。
type ClickTargetMatch = (
    fn(&str) -> bool,
    &'static str,
    Option<fn(ControlType) -> bool>,
);

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

pub fn click_yes_in_codex_approval(is_git_commit_hint: bool) -> Result<ClickOutcome, String> {
    run_uia_task(move || click_yes_inner(is_git_commit_hint))
}

pub fn click_no_in_codex_approval(is_git_commit_hint: bool) -> Result<ClickOutcome, String> {
    run_uia_task(move || click_no_inner(is_git_commit_hint))
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
    // タイトルだけで git commit dialog と判定できる場合、UI ツリー全走査を省略する。
    // 独立 HWND の dialog では title が直接 "変更をコミット"（または "提交更改"）等になり、collect_text を呼ばずに
    // 即座に承認候補を返せるため、観測〜自動操作までのレイテンシを大幅に短縮できる。
    if title_matches_git_commit(title) {
        diagnostics.notes.push(format!(
            "  parse: title=\"{}\" view=title-only (git commit short-circuit)",
            short(title, 40),
        ));
        return Some(ObservedApproval {
            request: ApprovalRequest {
                id: None,
                source_app: "Codex Desktop".to_string(),
                window_title: title.to_string(),
                prompt_text: title.to_string(),
                command: Some("git commit (dialog)".to_string()),
                cwd: None,
                target_paths: Vec::new(),
                requested_permission: Some("git_commit_dismiss".to_string()),
            },
            raw_text: vec![title.to_string()],
            detected_by: "Windows UI Automation title-only observer (git commit)".to_string(),
        });
    }

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

        // Git commit window check（タイトル不一致だが本文に含まれるケース。
        // この場合 dialog は WebView 内蔵 modal のため WM_CLOSE は使えない。
        // 通常の UIA 「閉じる/キャンセル」ボタンのクリックに委ねる）。
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

        // 本当に承認ダイアログが存在する場合のみ強キーワードが現れる。
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

fn click_no_inner(is_git_commit_hint: bool) -> Result<ClickOutcome, String> {
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

        // git commit dialog の判定は観測フェーズの hint を優先する。
        // dialog が独立 HWND の場合は title が直接命中するため、本文走査なしで確定できる。
        // hint が立っていない場合のみ、念のため本文を走査して救済する（fallback）。
        let title_says_git_commit = title_matches_git_commit(&candidate.title);
        let is_git_commit = if is_git_commit_hint || title_says_git_commit {
            true
        } else {
            let mut raw_text = Vec::new();
            let mut keyword_hit = false;
            collect_text(&raw_walker, window, 0, &mut raw_text, &mut keyword_hit);
            looks_like_git_commit_window(&candidate.title, &raw_text)
        };

        outcome.target_window = candidate.title.clone();
        outcome.notes.push(format!(
            "ターゲット: title=\"{}\" process={} git_commit={}",
            short(&candidate.title, 60),
            candidate.process_name.as_deref().unwrap_or("?"),
            is_git_commit,
        ));

        // 独立 HWND の git commit dialog は WM_CLOSE で即座に閉じられる（最速経路）。
        // WM_CLOSE が失敗した場合は UIA 経路へフォールバック。
        if is_git_commit
            && title_says_git_commit
            && try_close_window_via_wm_close(window, &mut outcome)
        {
            outcome.yes_invoked = true;
            return Ok(outcome);
        }

        // WebView 内蔵 modal の git commit dialog は VK_ESCAPE で閉じられる（次に速い経路）。
        // Codex Desktop の commit dialog には UIA 上「閉じる/✕」を持つボタンが存在せず、
        // UIA 検索すると無関係な「スキップ」等を誤クリックする事故が発生したため、
        // ユーザーが手動で確認した Escape キー経路を優先する。
        if is_git_commit
            && !title_says_git_commit
            && try_send_escape_to_window(window, &mut outcome)
        {
            outcome.yes_invoked = true;
            return Ok(outcome);
        }

        let (matcher, label, type_filter): ClickTargetMatch = if is_git_commit {
            (is_close_or_cancel_button, "「閉じる/キャンセル」", Some(is_close_button_type))
        } else {
            (is_first_no_option, "「3. いいえ」", None)
        };

        let no_candidates = collect_candidates_with_filter(&raw_walker, window, matcher, type_filter);
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
        outcome.method = Some(
            if is_git_commit { "uia-close-button" } else { "uia-no-button" }.to_string(),
        );

        if is_git_commit {
            outcome.notes.push("Git コミットダイアログのため「送信」処理をスキップします。".to_string());
            return Ok(outcome);
        }

        let mut submit_candidates = Vec::new();
        for _i in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            submit_candidates = collect_candidates(&raw_walker, window, is_submit_button);
            if !submit_candidates.is_empty() {
                break;
            }
        }
        log_candidates(&mut outcome, "「送信」", &submit_candidates);
        match pick_best(&submit_candidates) {
            Some(submit_target) => {
                let submit_method = invoke_candidate(submit_target, window).map_err(|error| {
                    format!(
                        "「3. いいえ」は選択しましたが、「送信」の invoke に失敗しました: {error}。手動で送信してください。"
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

        try_activate_pending_sidebar_after_main(&raw_walker, window, &mut outcome);
        return Ok(outcome);
    }

    Err("Codex プロセスのウィンドウが見つかりませんでした。".to_string())
}

fn click_yes_inner(is_git_commit_hint: bool) -> Result<ClickOutcome, String> {
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

        // git commit dialog の判定は観測フェーズの hint を優先する。
        // dialog が独立 HWND の場合は title が直接命中するため、本文走査なしで確定できる。
        // hint が立っていない場合のみ、念のため本文を走査して救済する（fallback）。
        let title_says_git_commit = title_matches_git_commit(&candidate.title);
        let is_git_commit = if is_git_commit_hint || title_says_git_commit {
            true
        } else {
            let mut raw_text = Vec::new();
            let mut keyword_hit = false;
            collect_text(&raw_walker, window, 0, &mut raw_text, &mut keyword_hit);
            looks_like_git_commit_window(&candidate.title, &raw_text)
        };

        // git commit dialog では本来の独立 HWND を優先的にターゲットしたいので、
        // 主ウィンドウ（"Codex" タイトル）を検出した場合は、後続の dialog HWND を待つ。
        // hint が立っている状態で title が一致しないということは、まだ dialog HWND が
        // top-level に列挙されていないか、WebView 内蔵 modal の可能性がある。
        // ここでは安全側で続行（UIA 検索フォールバック）。
        outcome.target_window = candidate.title.clone();
        outcome.notes.push(format!(
            "ターゲット: title=\"{}\" process={} git_commit={}",
            short(&candidate.title, 60),
            candidate.process_name.as_deref().unwrap_or("?"),
            is_git_commit,
        ));

        // 独立 HWND の git commit dialog は WM_CLOSE で即座に閉じられる（最速経路）。
        // WM_CLOSE が失敗した場合は UIA 経路へフォールバック。
        if is_git_commit
            && title_says_git_commit
            && try_close_window_via_wm_close(window, &mut outcome)
        {
            outcome.yes_invoked = true;
            return Ok(outcome);
        }

        // WebView 内蔵 modal の git commit dialog は VK_ESCAPE で閉じられる（次に速い経路）。
        // Codex Desktop の commit dialog には UIA 上「閉じる/✕」を持つボタンが存在せず、
        // UIA 検索すると無関係な「跳过」等を誤クリックする事故が発生したため、
        // ユーザーが手動で確認した Escape キー経路を優先する。
        if is_git_commit
            && !title_says_git_commit
            && try_send_escape_to_window(window, &mut outcome)
        {
            outcome.yes_invoked = true;
            return Ok(outcome);
        }

        let (matcher, label, type_filter): ClickTargetMatch = if is_git_commit {
            (is_close_or_cancel_button, "「閉じる/キャンセル」", Some(is_close_button_type))
        } else {
            (is_first_yes_or_recommended_option, "「1. はい / N. (推奨)」", None)
        };

        let yes_candidates = collect_candidates_with_filter(&raw_walker, window, matcher, type_filter);
        log_candidates(&mut outcome, label, &yes_candidates);
        let yes_target = match pick_best(&yes_candidates) {
            Some(target) => target,
            None => {
                // 承認ダイアログ本体（「1. はい」「Approve」等）が UI ツリーに無い場合、
                // 非アクティブな会話がサイドバーで承認待ちになっている可能性がある。
                // バッジを手掛かりにサイドバーの会話アイテムをアクティブ化し、
                // 次回ポーリングで通常フローに合流させる。git commit ダイアログ側
                // （is_git_commit）はサイドバーバッジを持たないため対象外。
                if !is_git_commit {
                    let sidebar_targets =
                        collect_pending_approval_sidebar_targets(&raw_walker, window);
                    log_candidates(&mut outcome, "「承認待ち」バッジ", &sidebar_targets);
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
        outcome.method = Some(
            if is_git_commit {
                "uia-close-button"
            } else if is_recommended_option(&yes_target.name) {
                "uia-recommended-option"
            } else {
                "uia-yes-button"
            }
            .to_string(),
        );

        if is_git_commit {
            outcome.notes.push("Git コミットダイアログのため「送信」処理をスキップします。".to_string());
            return Ok(outcome);
        }

        let mut submit_candidates = Vec::new();
        for _i in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            submit_candidates = collect_candidates(&raw_walker, window, is_submit_button);
            if !submit_candidates.is_empty() {
                break;
            }
        }
        log_candidates(&mut outcome, "「送信」", &submit_candidates);
        match pick_best(&submit_candidates) {
            Some(submit_target) => {
                let submit_method = invoke_candidate(submit_target, window).map_err(|error| {
                    format!(
                        "「1. はい」は選択しましたが、「送信」の invoke に失敗しました: {error}。手動で送信してください。"
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

        try_activate_pending_sidebar_after_main(&raw_walker, window, &mut outcome);
        return Ok(outcome);
    }

    Err("Codex プロセスのウィンドウが見つかりませんでした。".to_string())
}

/// アクティブ会話の承認ループに阻まれて他プロジェクトのサイドバー承認待ち項目が
/// 永続的にスキップされる餓死（starvation）を防ぐためのフォローアップ処理。
///
/// 現状の `click_yes_inner` は「アクティブ会話で『1. はい』候補が見つからない」場合のみ
/// サイドバー pending badge を辿って会話をアクティブ化する。しかしアクティブ会話が
/// 数秒間隔で承認を生成し続ける（例: `git status --short` を 3〜4 秒に 1 回）と、
/// 各ポーリング周期で常に「1. はい」候補が見つかり、サイドバー fallback が永遠に
/// 発火しない。
///
/// 本関数はメイン承認操作（「1. はい」+「送信」）成功後に追加で 1 件だけ
/// サイドバー pending 会話をアクティブ化することで、次回ポーリングで別会話の
/// 承認ダイアログが検出されるようにする。アクティブ化に失敗しても本来の承認結果には
/// 影響しないため、エラーは notes に記録するだけで握りつぶす。
///
/// 注意:
/// - サイドバーアクティブ化はアクティブ会話を切り替える副作用がある。
///   `auto_approve_observed_request` 側で `user_idle_ms >= 1500ms` のガードが
///   既に効いており、ユーザーが操作中の場合は本関数まで到達しない。
/// - 切り替え先の会話に承認待ちが残っていなければ、次サイクルで通常の監視状態に戻る。
fn try_activate_pending_sidebar_after_main(
    walker: &UITreeWalker,
    window: &UIElement,
    outcome: &mut ClickOutcome,
) {
    let sidebar_targets = collect_pending_approval_sidebar_targets(walker, window);
    if sidebar_targets.is_empty() {
        return;
    }
    log_candidates(outcome, "後続「承認待ち」サイドバー", &sidebar_targets);
    let Some(target) = pick_best(&sidebar_targets) else {
        return;
    };
    match invoke_candidate(target, window) {
        Ok(method) => {
            outcome.notes.push(format!(
                "他会話の承認待ちサイドバーをアクティブ化（次サイクル用）: type={:?} method={} name=\"{}\"",
                target.control_type,
                method,
                short(&target.name, 60),
            ));
        }
        Err(error) => {
            outcome.notes.push(format!(
                "後続サイドバーアクティブ化に失敗（メイン承認は成功済）: name=\"{}\" error={}",
                short(&target.name, 60),
                error,
            ));
        }
    }
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
    collect_candidates_with_filter(walker, root, matcher, None)
}

/// `type_filter` を指定すると、name 命中要素のうち control_type が条件を満たすものだけを
/// Candidate に格上げする。WebView2 のような巨大ツリーで「关闭/X/Close」のような汎用
/// マッチャを使う際、pattern クエリ（IPC）の回数を大幅に減らせる。
fn collect_candidates_with_filter(
    walker: &UITreeWalker,
    root: &UIElement,
    matcher: impl Fn(&str) -> bool,
    type_filter: Option<fn(ControlType) -> bool>,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    collect_candidates_recursive(walker, root, 0, &matcher, type_filter, &mut out);
    out
}

fn collect_candidates_recursive(
    walker: &UITreeWalker,
    element: &UIElement,
    depth: usize,
    matcher: &impl Fn(&str) -> bool,
    type_filter: Option<fn(ControlType) -> bool>,
    out: &mut Vec<Candidate>,
) {
    if depth > MAX_CLICK_TREE_DEPTH || out.len() >= 30 {
        return;
    }
    let name = safe_name(element);
    if matcher(&name) && !is_system_caption_or_titlebar_element(element) {
        let control_type = element.get_control_type().ok();
        let type_ok = match (type_filter, control_type) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(filter), Some(ctype)) => filter(ctype),
        };
        if type_ok {
            out.push(Candidate {
                element: element.clone(),
                name,
                control_type,
                has_invoke: element.get_pattern::<UIInvokePattern>().is_ok(),
                has_select: element.get_pattern::<UISelectionItemPattern>().is_ok(),
                has_legacy: element.get_pattern::<UILegacyIAccessiblePattern>().is_ok(),
            });
        }
    }
    if let Some(children) = walker.get_children(element) {
        for child in children {
            collect_candidates_recursive(walker, &child, depth + 1, matcher, type_filter, out);
        }
    }
}

/// git commit dialog の「关闭/取消」候補として許可する control_type。
/// WebView2 内では Hyperlink/Button が混在するため両方許可する。
fn is_close_button_type(control_type: ControlType) -> bool {
    matches!(
        control_type,
        ControlType::Button | ControlType::Hyperlink | ControlType::SplitButton
    )
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

/// VK_ESCAPE のキーコード。windows-sys では `Win32_UI_Input_KeyboardAndMouse` feature
/// に存在するが、本プロジェクトの Cargo.toml の features に明示されておらず、ハードコード
/// した方が依存を増やさずに済む（VK 値は Windows ABI で固定）。
const VK_ESCAPE_RAW: usize = 0x1B;
/// WM_KEYDOWN の lparam: 繰り返し回数=1, scan code=0x01 (Escape), 拡張ビットなし。
const ESC_LPARAM_DOWN: isize = 0x0001_0001;
/// WM_KEYUP の lparam: keydown と同じ + bit30 (前状態=押下) + bit31 (遷移=離す)。
const ESC_LPARAM_UP: isize = 0xC001_0001u32 as i32 as isize;

extern "system" fn collect_child_hwnds(
    hwnd: windows_sys::Win32::Foundation::HWND,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::core::BOOL {
    let collector = unsafe {
        &mut *(lparam as *mut Vec<windows_sys::Win32::Foundation::HWND>)
    };
    if collector.len() < 64 {
        collector.push(hwnd);
        1
    } else {
        0
    }
}

/// VK_ESCAPE を「シングル INPUT」として組み立てる小ヘルパ。
fn make_esc_input(flags: u32) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    let mut input: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT =
        unsafe { std::mem::zeroed() };
    input.r#type = windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_KEYBOARD;
    input.Anonymous.ki =
        windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYBDINPUT {
            wVk: 0x1B, // VK_ESCAPE
            wScan: 0,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
    input
}

/// `AttachThreadInput` トリックで他プロセスのウィンドウを強制的にフォアグラウンドに
/// 持ち上げる。Windows のフォアグラウンド権限制限を一時的にバイパスする標準的な手法。
/// 戻り値: SetForegroundWindow が 0 以外を返したか。
fn force_set_foreground(hwnd_raw: windows_sys::Win32::Foundation::HWND) -> bool {
    if hwnd_raw.is_null() {
        return false;
    }
    let prev_fg =
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    let fg_tid = if !prev_fg.is_null() {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                prev_fg,
                std::ptr::null_mut(),
            )
        }
    } else {
        0
    };
    let my_tid = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
    let attached = if fg_tid != 0 && fg_tid != my_tid {
        unsafe {
            windows_sys::Win32::System::Threading::AttachThreadInput(my_tid, fg_tid, 1) != 0
        }
    } else {
        false
    };
    let set_ok =
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd_raw) };
    if attached {
        unsafe {
            windows_sys::Win32::System::Threading::AttachThreadInput(my_tid, fg_tid, 0);
        }
    }
    set_ok != 0
}

/// `SendInput` で実 OS 入力として VK_ESCAPE を流し込む。Chromium はハードウェア入力を
/// 信頼するため、合成 PostMessage より遥かに確実に dialog の Escape ハンドラへ届く。
///
/// Codex が既にフォアグラウンドであればそのまま送るだけ。バックグラウンドの場合は
/// `AttachThreadInput` トリックで一時的に Codex を前面に持ち上げてから VK_ESCAPE を
/// 送信し、終わったら元のフォアグラウンドウィンドウへフォーカスを戻す（ユーザー体感は
/// 30〜50ms の Codex 一瞬表示）。
fn try_send_input_escape(
    hwnd_raw: windows_sys::Win32::Foundation::HWND,
    outcome: &mut ClickOutcome,
) -> bool {
    if hwnd_raw.is_null() {
        return false;
    }
    let prev_fg =
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    let already_fg = prev_fg == hwnd_raw;

    if !already_fg {
        if !force_set_foreground(hwnd_raw) {
            outcome.notes.push(
                "SendInput 経路: 強制フォアグラウンド化に失敗（権限制限の可能性）。".to_string(),
            );
            return false;
        }
        std::thread::sleep(Duration::from_millis(30));
    }

    let inputs = [
        make_esc_input(0),
        make_esc_input(windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_KEYUP),
    ];
    let sent = unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput(
            2,
            inputs.as_ptr(),
            std::mem::size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT>() as i32,
        )
    };
    let success = sent == 2;

    // 元のフォアグラウンドを復帰（自分が前面化していた場合のみ）。
    if !already_fg && !prev_fg.is_null() {
        std::thread::sleep(Duration::from_millis(10));
        force_set_foreground(prev_fg);
    }

    if success {
        outcome.notes.push(format!(
            "SendInput 経路: VK_ESCAPE x2 を送信成功（force_fg={}）。",
            !already_fg
        ));
        true
    } else {
        outcome.notes.push(format!(
            "SendInput 経路: SendInput が {} 個しか送れませんでした。",
            sent
        ));
        false
    }
}

/// WebView 内蔵 modal の git commit dialog に対し、`VK_ESCAPE` を複数経路で投入する。
///
/// WebView2 (Chromium) は合成キーボード入力を環境やタイミングによって取りこぼすため、
/// 単一経路では確実に届かない。ここでは以下を順に試し、成功したものを `method` に
/// 連結して記録する（一つでも成功すれば dialog は閉じる想定）。
///
/// 1. `AttachThreadInput` で Codex スレッドへ入力キューを連結 → `GetFocus()` で
///    Codex 側のフォーカス HWND を取得 → そこへ PostMessage（精度高、副作用なし）。
/// 2. Codex がフォアグラウンドにある場合のみ `SendInput` で実ハードウェア入力として
///    VK_ESCAPE を送信（最も確実、フォーカス奪取なし）。
/// 3. 親 HWND + `EnumChildWindows` で得た子孫 HWND に PostMessage をブロードキャスト
///    （最後の保険、副作用は小さい）。
fn try_send_escape_to_window(window: &UIElement, outcome: &mut ClickOutcome) -> bool {
    let handle = match window.get_native_window_handle() {
        Ok(handle) => handle,
        Err(error) => {
            outcome.notes.push(format!(
                "Escape 経路: HWND 取得失敗（{error}）。UIA 経路へフォールバック。"
            ));
            return false;
        }
    };
    let hwnd_win: windows::Win32::Foundation::HWND = handle.into();
    let hwnd_raw = hwnd_win.0 as windows_sys::Win32::Foundation::HWND;
    if hwnd_raw.is_null() {
        outcome
            .notes
            .push("Escape 経路: HWND が NULL のためフォールバック。".to_string());
        return false;
    }

    let mut succeeded: Vec<&'static str> = Vec::new();

    // 経路1: AttachThreadInput → Codex 側 GetFocus → 該当 HWND に PostMessage。
    if try_post_escape_via_attached_focus(hwnd_raw, outcome) {
        succeeded.push("attach-focus");
    }

    // 経路2: SendInput でハードウェア入力として VK_ESCAPE を送信。Chromium は合成
    // PostMessage を取りこぼすが、SendInput はほぼ確実に届く。Codex が非フォアグラウンドの
    // 場合は AttachThreadInput トリックで一時的に前面化してから送り、終了後にフォーカスを
    // 復帰させる（ユーザー体感は短い Codex フラッシュ）。
    if try_send_input_escape(hwnd_raw, outcome) {
        succeeded.push("sendinput");
    }

    // 経路3: 親+子 HWND に PostMessage をブロードキャスト（最後の保険）。
    if try_post_escape_broadcast(hwnd_raw, outcome) {
        succeeded.push("broadcast");
    }

    if succeeded.is_empty() {
        outcome
            .notes
            .push("Escape 経路: 全経路失敗。UIA 経路へフォールバック。".to_string());
        return false;
    }
    outcome.method = Some(format!("escape-multi[{}]", succeeded.join("+")));
    true
}

/// Escape 経路1: AttachThreadInput → GetFocus → PostMessage。失敗時 false。
fn try_post_escape_via_attached_focus(
    hwnd_raw: windows_sys::Win32::Foundation::HWND,
    outcome: &mut ClickOutcome,
) -> bool {
    let codex_tid = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            hwnd_raw,
            std::ptr::null_mut(),
        )
    };
    let my_tid = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
    if codex_tid == 0 || codex_tid == my_tid {
        outcome.notes.push(
            "AttachThreadInput 経路: TID 取得不可 / 自スレッドのためスキップ。".to_string(),
        );
        return false;
    }
    let attached = unsafe {
        windows_sys::Win32::System::Threading::AttachThreadInput(my_tid, codex_tid, 1)
    };
    if attached == 0 {
        outcome
            .notes
            .push("AttachThreadInput 経路: AttachThreadInput 失敗。".to_string());
        return false;
    }
    let focused_raw =
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() };
    unsafe {
        windows_sys::Win32::System::Threading::AttachThreadInput(my_tid, codex_tid, 0);
    }
    if focused_raw.is_null() {
        outcome
            .notes
            .push("AttachThreadInput 経路: GetFocus が NULL。".to_string());
        return false;
    }
    let ok_down = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            focused_raw,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
            VK_ESCAPE_RAW,
            ESC_LPARAM_DOWN,
        )
    };
    let ok_up = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            focused_raw,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYUP,
            VK_ESCAPE_RAW,
            ESC_LPARAM_UP,
        )
    };
    if ok_down != 0 && ok_up != 0 {
        outcome.notes.push(format!(
            "AttachThreadInput 経路: focused HWND={:p} に VK_ESCAPE 送信成功。",
            focused_raw
        ));
        true
    } else {
        outcome.notes.push(format!(
            "AttachThreadInput 経路: PostMessage 失敗 (down={}, up={})。",
            ok_down, ok_up
        ));
        false
    }
}

/// Escape 経路3: 親 + 子孫 HWND 全部にブロードキャスト。
fn try_post_escape_broadcast(
    hwnd_raw: windows_sys::Win32::Foundation::HWND,
    outcome: &mut ClickOutcome,
) -> bool {
    let mut targets: Vec<windows_sys::Win32::Foundation::HWND> = vec![hwnd_raw];
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::EnumChildWindows(
            hwnd_raw,
            Some(collect_child_hwnds),
            &mut targets as *mut _ as windows_sys::Win32::Foundation::LPARAM,
        );
    }
    let mut posted = 0usize;
    for target in &targets {
        let ok_down = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                *target,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
                VK_ESCAPE_RAW,
                ESC_LPARAM_DOWN,
            )
        };
        let ok_up = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                *target,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYUP,
                VK_ESCAPE_RAW,
                ESC_LPARAM_UP,
            )
        };
        if ok_down != 0 && ok_up != 0 {
            posted += 1;
        }
    }
    if posted == 0 {
        outcome
            .notes
            .push("Broadcast 経路: いずれの HWND にも PostMessage が届きませんでした。".to_string());
        return false;
    }
    outcome.notes.push(format!(
        "Broadcast 経路: {} 個の HWND に VK_ESCAPE PostMessage 送信。",
        posted
    ));
    true
}

/// 独立 HWND として開かれている git commit dialog に `WM_CLOSE` を送り、
/// UIA ツリーを再走査せずに dialog を閉じる。送信に成功した場合 `true` を返す。
/// HWND が取得できない／PostMessage に失敗した場合は `false` を返し、呼び出し側で
/// 既存の UIA 経由クリックにフォールバックする。
fn try_close_window_via_wm_close(window: &UIElement, outcome: &mut ClickOutcome) -> bool {
    let handle = match window.get_native_window_handle() {
        Ok(handle) => handle,
        Err(error) => {
            outcome.notes.push(format!(
                "WM_CLOSE 経路: HWND 取得失敗（{error}）。UIA 経路へフォールバック。"
            ));
            return false;
        }
    };
    let hwnd_win: windows::Win32::Foundation::HWND = handle.into();
    let hwnd_raw = hwnd_win.0 as windows_sys::Win32::Foundation::HWND;
    if hwnd_raw.is_null() {
        outcome
            .notes
            .push("WM_CLOSE 経路: HWND が NULL のためフォールバック。".to_string());
        return false;
    }
    let ok = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            hwnd_raw,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_CLOSE,
            0,
            0,
        )
    };
    if ok == 0 {
        outcome
            .notes
            .push("WM_CLOSE 経路: PostMessageW 失敗。UIA 経路へフォールバック。".to_string());
        return false;
    }
    outcome
        .notes
        .push("WM_CLOSE 経路: dialog HWND に WM_CLOSE を送信しました（fast-path）。".to_string());
    outcome.method = Some("wm-close".to_string());
    true
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
