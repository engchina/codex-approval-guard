mod audit;
mod circuit;
mod platform;
mod policy;
mod update;

use std::sync::Mutex;

use audit::{AuditEntry, AuditStore};
use circuit::GuardCircuit;
use platform::{ClickOutcome, ObserveDiagnostics, ObservedApproval, PlatformSnapshot};
use policy::{ApprovalDecision, ApprovalRequest, DecisionAction, PolicyConfig, RiskLevel};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{async_runtime, Manager, State, WindowEvent};

struct AppState {
    policy: Mutex<PolicyConfig>,
    policy_path: std::path::PathBuf,
    audit: AuditStore,
    circuit: GuardCircuit,
}

impl AppState {
    fn new(app: &tauri::AppHandle) -> Result<Self, String> {
        // app_data_dir → app_config_dir の順で試す。
        // 旧実装は両方失敗時に std::env::current_dir() へ fallback していたが、
        // 配布版（NSIS で Program Files 配下にインストール）では cwd が
        // 書込不可なディレクトリになることが多く、policy.json / audit.jsonl の
        // 書込みが silent に失敗する原因になっていた。fallback を打ち切って
        // 起動時に明示的エラーを出すほうがデバッグしやすい。
        let app_data_dir = app
            .path()
            .app_data_dir()
            .or_else(|_| app.path().app_config_dir())
            .map_err(|error| format!("アプリデータディレクトリを取得できません: {error}"))?;

        std::fs::create_dir_all(&app_data_dir).map_err(|error| {
            format!(
                "アプリデータディレクトリを作成できません ({}): {error}",
                app_data_dir.display()
            )
        })?;

        let policy_path = app_data_dir.join("policy.json");
        // 起動時にディスクから設定を復元する（存在しない場合はデフォルト）。
        let policy = PolicyConfig::load_or_default(&policy_path);

        Ok(Self {
            policy: Mutex::new(policy),
            policy_path,
            audit: AuditStore::new(app_data_dir.join("audit.jsonl")),
            circuit: GuardCircuit::new(),
        })
    }

    /// 回路ブレーカー作動時の共通処理: policy を paused に倒し、その変更を
    /// ディスクへ書き戻し、システム由来のエントリとして audit log に追記する。
    /// 失敗してもプロセスは継続する（個別のエラーだけ stderr へ出す）。
    fn trigger_circuit_breaker(&self, kind: &str, reason: &str) {
        let updated = {
            let mut policy = match self.policy.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            if policy.paused {
                // すでに paused なら状態変更は不要。audit だけ書く。
                policy.clone()
            } else {
                policy.paused = true;
                policy.clone()
            }
        };
        if let Err(error) = self.persist_policy(&updated) {
            eprintln!("[circuit] policy 永続化に失敗しました（メモリ上では paused 済）: {error}");
        }

        let synthetic_request = ApprovalRequest {
            id: None,
            source_app: "Codex Approval Guard".to_string(),
            window_title: format!("（システム）回路ブレーカー作動: {kind}"),
            prompt_text: reason.to_string(),
            command: None,
            cwd: None,
            target_paths: Vec::new(),
            requested_permission: Some(format!("circuit_breaker:{kind}")),
        };
        let synthetic_decision = ApprovalDecision {
            action: DecisionAction::Prompt,
            risk: RiskLevel::High,
            reason: reason.to_string(),
            matched_rule: Some(format!("circuit_breaker:{kind}")),
            would_auto_approve: false,
        };
        if let Err(error) = self.audit.append(&synthetic_request, &synthetic_decision) {
            eprintln!("[circuit] audit log 追記に失敗しました: {error}");
        }
    }

    /// 現在のポリシーをディスクへ書き出す。書き込みに失敗してもメモリ上の状態は維持する。
    fn persist_policy(&self, policy: &PolicyConfig) -> Result<(), String> {
        policy.save_to(&self.policy_path)
    }
}

#[derive(Debug, serde::Serialize)]
struct GuardState {
    policy: PolicyConfig,
    platform: PlatformSnapshot,
    recent_audits: Vec<AuditEntry>,
    audit_log_path: String,
}

#[derive(Debug, serde::Serialize)]
struct ApprovalObservation {
    platform: PlatformSnapshot,
    observed: Option<ObservedApproval>,
    decision: Option<ApprovalDecision>,
    recorded: bool,
    details: String,
    diagnostics: ObserveDiagnostics,
}

#[tauri::command]
async fn get_guard_state(state: State<'_, AppState>) -> Result<GuardState, String> {
    let policy = state
        .policy
        .lock()
        .map_err(|_| "Policy ロックを取得できません".to_string())?
        .clone();
    let recent_audits = state.audit.list_recent(12)?;
    let audit_log_path = state.audit.path().to_string_lossy().to_string();
    let platform = async_runtime::spawn_blocking(platform::snapshot_active_window)
        .await
        .map_err(|error| format!("Platform snapshot を取得できません: {error}"))?;

    Ok(GuardState {
        policy,
        platform,
        recent_audits,
        audit_log_path,
    })
}

#[tauri::command]
async fn observe_approval_request(
    state: State<'_, AppState>,
) -> Result<ApprovalObservation, String> {
    // UIA observe の結果を回路ブレーカーへ報告する。spawn_blocking の panic /
    // 内部 UIA error の両方を「失敗」として扱う。成功（observed が None でも
    // platform 走査自体は完走）したら failure 連続カウンタはリセットされる。
    let join_result = async_runtime::spawn_blocking(platform::observe_approval_request).await;
    let observe_result = match join_result {
        Ok(inner) => inner,
        Err(join_error) => {
            if let Some(reason) = state.circuit.record_observe_result(false) {
                state.trigger_circuit_breaker("uia_failure", &reason);
            }
            return Err(format!(
                "UI Automation observation を実行できません: {join_error}"
            ));
        }
    };
    let (observed, diagnostics) = match observe_result {
        Ok(pair) => {
            if let Some(reason) = state.circuit.record_observe_result(true) {
                // 成功時は通常 None だが、リセット直後の特殊条件を将来追加する余地。
                state.trigger_circuit_breaker("uia_failure", &reason);
            }
            pair
        }
        Err(platform_error) => {
            if let Some(reason) = state.circuit.record_observe_result(false) {
                state.trigger_circuit_breaker("uia_failure", &reason);
            }
            return Err(platform_error);
        }
    };
    let platform = platform_snapshot_from_observation(&observed);
    let Some(observed_request) = observed else {
        return Ok(ApprovalObservation {
            platform,
            observed: None,
            decision: None,
            recorded: false,
            details: "Codex 承認ウィンドウは検出されませんでした。".to_string(),
            diagnostics,
        });
    };

    let policy = state
        .policy
        .lock()
        .map_err(|_| "Policy ロックを取得できません".to_string())?
        .clone();
    // observe 段階で返す decision は「現時点の policy に基づく見込み判定」であり、
    // UI のプレビュー専用。実際の自動操作は auto_approve_observed_request で
    // 再評価された決定で行うため、両者が異なる場合は後者が権威。これにより
    // 観察直後にユーザーが paused / allow_git_* を切り替えても、最新ポリシーが
    // クリック実行時に反映される。audit log には実行時の decision のみが残る。
    let decision = policy.evaluate(&observed_request.request);

    // 監査ログは実際にクリック動作を行った時点（auto_approve_observed_request）でのみ
    // 追記する。観察フェーズで append すると、Codex 主ウィンドウが開いている間
    // ポーリング毎に同じ内容が積み上がってしまうため。
    Ok(ApprovalObservation {
        platform,
        observed: Some(observed_request),
        decision: Some(decision),
        recorded: false,
        details: "Codex 承認ウィンドウを観察し、承認操作を実行可能です。".to_string(),
        diagnostics,
    })
}

fn platform_snapshot_from_observation(observed: &Option<ObservedApproval>) -> PlatformSnapshot {
    PlatformSnapshot {
        backend: "Windows UI Automation".to_string(),
        available: true,
        focused_window_title: observed
            .as_ref()
            .map(|observed| observed.request.window_title.clone()),
        details: if observed.is_some() {
            "UI Automation 監視がバックグラウンドで Codex 候補ウィンドウを検出しました。"
                .to_string()
        } else {
            "Codex 承認候補ウィンドウは検出されませんでした。".to_string()
        },
    }
}

#[derive(Debug, serde::Serialize)]
struct AutoApproveOutcome {
    decision: ApprovalDecision,
    click: ClickOutcome,
    audited: bool,
}

/// 自動操作を抑制するユーザー入力の最小アイドル時間（ミリ秒）。
/// この時間以内にキーボード/マウス入力があった場合は、マウス操作の競合を避けるため
/// 自動承認をスキップし、次回ポーリングでの再評価に委ねる。
const USER_ACTIVITY_GUARD_MS: u32 = 1500;

#[tauri::command]
async fn auto_approve_observed_request(
    request: ApprovalRequest,
    state: State<'_, AppState>,
) -> Result<AutoApproveOutcome, String> {
    let policy_snapshot = {
        let policy = state
            .policy
            .lock()
            .map_err(|_| "Policy ロックを取得できません".to_string())?;
        if policy.paused {
            return Err("ガードは一時停止中です。自動承認は実行できません。".to_string());
        }
        policy.clone()
    };

    // ユーザーが操作中（直近の入力から閾値未満）の場合は、マウスカーソルを奪わないよう
    // 自動操作をスキップする。次のポーリングサイクルで再評価される。
    let idle_ms = platform::user_idle_ms();
    if idle_ms < USER_ACTIVITY_GUARD_MS {
        return Err(format!(
            "ユーザー操作中のため自動承認をスキップしました（idle={}ms < {}ms）。",
            idle_ms, USER_ACTIVITY_GUARD_MS
        ));
    }

    let decision = policy_snapshot.evaluate(&request);
    let is_dismiss = decision.action == DecisionAction::Dismiss;
    let is_deny = decision.action == DecisionAction::Deny;
    if decision.action != DecisionAction::Approve && !is_dismiss && !is_deny {
        return Err(format!(
            "policy 判定が approve/dismiss/deny ではありません ({:?})。自動操作を中止しました。",
            decision.action
        ));
    }

    // 観測フェーズで git commit dialog と判定済みなら click 側で再走査しないよう hint を渡す。
    let is_git_commit_hint = request
        .requested_permission
        .as_deref()
        .map(|permission| permission == "git_commit_dismiss")
        .unwrap_or(false);

    let click = if is_deny {
        async_runtime::spawn_blocking(move || {
            platform::click_no_in_codex_approval(is_git_commit_hint)
        })
        .await
        .map_err(|error| format!("自動操作の実行に失敗しました: {error}"))??
    } else {
        async_runtime::spawn_blocking(move || {
            platform::click_yes_in_codex_approval(is_git_commit_hint)
        })
        .await
        .map_err(|error| format!("自動操作の実行に失敗しました: {error}"))??
    };

    let mut audit_decision = decision.clone();
    // 経路追跡のため method 識別子のみ短く付加する。詳細な notes は ClickOutcome 上に
    // メモリ保持されるが audit log には載せない（過去版で notes 全文を残した結果、
    // reason が肥大化したため）。
    let method_trace = click
        .method
        .as_deref()
        .map(|m| format!(", method={m}"))
        .unwrap_or_default();
    audit_decision.reason = if is_deny {
        format!(
            "{}（auto-denied: no={}, submit={}{}）",
            audit_decision.reason, click.yes_invoked, click.submit_invoked, method_trace
        )
    } else if is_dismiss {
        format!(
            "{}（auto-dismissed: close={}{}）",
            audit_decision.reason, click.yes_invoked, method_trace
        )
    } else {
        format!(
            "{}（auto-approved: yes={}, submit={}{}）",
            audit_decision.reason, click.yes_invoked, click.submit_invoked, method_trace
        )
    };
    state.audit.append(&request, &audit_decision)?;

    // 自動承認 burst 検出: 同一コマンドが短時間に連発で auto-approve されている
    // 場合に paused へ倒す。今回の click はすでに実行済みなので中断せず、
    // 次サイクル以降の polling を抑止するだけ。dismiss / deny も含めて記録する
    // （Codex 側の異常ループでは dismiss も連発しうるため）。
    let burst_key = request
        .command
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .unwrap_or(&request.window_title);
    if let Some(reason) = state.circuit.record_auto_approve(burst_key) {
        state.trigger_circuit_breaker("auto_approve_burst", &reason);
    }

    Ok(AutoApproveOutcome {
        decision: audit_decision,
        click,
        audited: true,
    })
}

#[tauri::command]
fn set_guard_paused(paused: bool, state: State<'_, AppState>) -> Result<PolicyConfig, String> {
    update_policy(&state, |policy| policy.paused = paused)
}

#[tauri::command]
fn set_allow_git_add(allow: bool, state: State<'_, AppState>) -> Result<PolicyConfig, String> {
    update_policy(&state, |policy| policy.allow_git_add = allow)
}

#[tauri::command]
fn set_allow_git_commit(allow: bool, state: State<'_, AppState>) -> Result<PolicyConfig, String> {
    update_policy(&state, |policy| policy.allow_git_commit = allow)
}

/// ポリシーを安全に更新し、ディスクへ永続化する共通ヘルパー。
/// 永続化に失敗した場合はエラーを返すが、メモリ上の状態は更新済み（次回保存時に再試行可能）。
fn update_policy<F>(state: &State<'_, AppState>, mutate: F) -> Result<PolicyConfig, String>
where
    F: FnOnce(&mut PolicyConfig),
{
    let updated = {
        let mut policy = state
            .policy
            .lock()
            .map_err(|_| "Policy ロックを取得できません".to_string())?;
        mutate(&mut policy);
        policy.clone()
    };
    state.persist_policy(&updated)?;
    Ok(updated)
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    // 簡易バリデーション: http(s):// 以外のスキームは拒否する。`cmd /c start` は
    // file:/// や任意のローカルパスも開けてしまうため、URL 経路で外部リソースを
    // 開く本コマンドでは http/https に絞る。
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("http/https 以外の URL は開けません。".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        // `cmd /c start <arg>` は最初のクオート付き引数を「ウィンドウタイトル」として
        // 解釈するため、URL を直接渡すとタイトル扱いになるか、URL 中の `&` / `|` /
        // `^` などが cmd によって解釈されてしまう。空タイトル `""` を挟み、URL は
        // 別引数として渡すことでタイトル衝突を回避する。URL 自体には Command::args
        // が自動でクオートを付けるため、cmd 側で trim される心配はないが、念のため
        // URL に含まれる " と % は拒否する（実用上のリンクには現れない）。
        if trimmed.contains('"') || trimmed.contains('%') {
            return Err("URL に解釈不能な文字が含まれています。".to_string());
        }
        std::process::Command::new("cmd")
            .args(["/c", "start", "", trimmed])
            .spawn()
            .map_err(|error| format!("URL を開けませんでした: {error}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(trimmed)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(trimmed)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn run() {
    let builder = tauri::Builder::default();

    // 単一インスタンス強制: 2 つ目の起動を検出すると、既存ウィンドウを前面に出して
    // 新規プロセスは即終了する。これにより以下を防ぐ:
    //   - policy.json の read-modify-write 競合（複数インスタンスが paused/
    //     allow_git_* を同時に書くと更新が消える）。
    //   - 複数インスタンスが同じ Codex ダイアログに対し UI Automation の
    //     click を重ねて投げ、誤動作・意図しないクリックが起きる事故。
    // Linux 向け plugin は本プロジェクトの対象外。
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }));

    builder
        .setup(|app| {
            let state = AppState::new(app.handle()).map_err(std::io::Error::other)?;
            app.manage(state);
            build_tray(app.handle()).map_err(std::io::Error::other)?;
            Ok(())
        })
        // ウィンドウ閉じる X ボタンを「タスクトレイへ最小化」へ振り替える。
        // バックグラウンド監視を継続するため、明示的な「退出」操作（トレイ
        // メニュー / Cmd+Q / app.exit）がない限りプロセスは生き続ける。
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_guard_state,
            observe_approval_request,
            auto_approve_observed_request,
            set_guard_paused,
            set_allow_git_add,
            set_allow_git_commit,
            update::check_for_app_update,
            open_url,
        ])
        .run(tauri::generate_context!())
        .expect("Codex Approval Guard の起動に失敗しました");
}

/// タスクトレイを組み立てる。
/// - 左クリック: メイン窓の表示/非表示トグル。
/// - メニュー「表示」: 強制的に前面化。
/// - メニュー「終了」: プロセス完全終了（バックグラウンド監視も停止）。
///
/// アイコンはアプリ既定の window icon を流用する（追加のアセット同梱不要）。
fn build_tray(app: &tauri::AppHandle) -> Result<(), String> {
    let show_item = MenuItem::with_id(app, "tray-show", "ウィンドウを表示", true, None::<&str>)
        .map_err(|error| format!("トレイメニューを構築できません: {error}"))?;
    let quit_item = MenuItem::with_id(app, "tray-quit", "終了", true, None::<&str>)
        .map_err(|error| format!("トレイメニューを構築できません: {error}"))?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])
        .map_err(|error| format!("トレイメニューを構築できません: {error}"))?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip("Codex Approval Guard")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray-show" => show_and_focus_main_window(app),
            "tray-quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Tauri 2 では Click イベントが Down / Up の両方で発火する。
            // Up（リリース時）だけ反応して、ダブルカウントとドラッグ誤発火を避ける。
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder
        .build(app)
        .map_err(|error| format!("トレイアイコンを作成できません: {error}"))?;
    Ok(())
}

fn show_and_focus_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    if visible {
        let _ = window.hide();
    } else {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
