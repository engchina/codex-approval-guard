mod audit;
mod platform;
mod policy;
mod update;

use std::sync::Mutex;

use audit::{AuditEntry, AuditStore};
use platform::{ClickOutcome, ObserveDiagnostics, ObservedApproval, PlatformSnapshot};
use policy::{ApprovalDecision, ApprovalRequest, DecisionAction, PolicyConfig};
use tauri::{async_runtime, Manager, State};

struct AppState {
    policy: Mutex<PolicyConfig>,
    policy_path: std::path::PathBuf,
    audit: AuditStore,
}

impl AppState {
    fn new(app: &tauri::AppHandle) -> Result<Self, String> {
        let app_data_dir = match app.path().app_data_dir() {
            Ok(path) => path,
            Err(_) => std::env::current_dir()
                .map_err(|error| format!("アプリデータディレクトリを取得できません: {error}"))?,
        };

        std::fs::create_dir_all(&app_data_dir)
            .map_err(|error| format!("アプリデータディレクトリを作成できません: {error}"))?;

        let policy_path = app_data_dir.join("policy.json");
        // 起動時にディスクから設定を復元する（存在しない場合はデフォルト）。
        let policy = PolicyConfig::load_or_default(&policy_path);

        Ok(Self {
            policy: Mutex::new(policy),
            policy_path,
            audit: AuditStore::new(app_data_dir.join("audit.jsonl")),
        })
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
    let (observed, diagnostics) = async_runtime::spawn_blocking(platform::observe_approval_request)
        .await
        .map_err(|error| format!("UI Automation observation を実行できません: {error}"))??;
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
            "UI Automation 監視がバックグラウンドで Codex 候補ウィンドウを検出しました。".to_string()
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
    if decision.action != DecisionAction::Approve && !is_dismiss {
        return Err(format!(
            "policy 判定が approve/dismiss ではありません ({:?})。自動操作を中止しました。",
            decision.action
        ));
    }

    let click = async_runtime::spawn_blocking(platform::click_yes_in_codex_approval)
        .await
        .map_err(|error| format!("自動操作の実行に失敗しました: {error}"))??;

    let mut audit_decision = decision.clone();
    audit_decision.reason = if is_dismiss {
        format!(
            "{}（auto-dismissed: close={}）",
            audit_decision.reason, click.yes_invoked
        )
    } else {
        format!(
            "{}（auto-approved: yes={}, submit={}）",
            audit_decision.reason, click.yes_invoked, click.submit_invoked
        )
    };
    state.audit.append(&request, &audit_decision)?;

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
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", &url])
            .spawn()
            .map_err(|error| format!("URL を開けませんでした: {error}"))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "macos")]
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|error| error.to_string())?;
        #[cfg(target_os = "linux")]
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn run() {
    let _ = audit::get_app_start_time();

    tauri::Builder::default()
        .setup(|app| {
            let state = AppState::new(app.handle()).map_err(std::io::Error::other)?;
            app.manage(state);
            Ok(())
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
