import {
  AlertTriangle,
  Pause,
  Play,
  ShieldCheck,
  Activity,
  FileClock,
  GitCommit,
  Settings,
  Copy,
  Check,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { callBackend } from "./backend";
import type {
  ApprovalObservation,
  AuditEntry,
  AutoApproveOutcome,
  GuardState,
  PolicyConfig,
  UpdateCheckResult,
} from "./types";

const actionLabels: Record<string, string> = {
  observe_only: "記録のみ",
  approve: "承認",
  dismiss: "閉鎖",
  deny: "拒否",
  prompt: "確認",
  ignore: "無視",
};

const riskLabels: Record<string, string> = {
  low: "低",
  medium: "中",
  high: "高",
};

const AUDIT_DISPLAY_LIMIT = 3;
// 適応 polling: 直近で承認ダイアログを観測したサイクルでは短い間隔で次を回し、
// 連続承認の取りこぼしを抑える。dialog が見つからないアイドル時は長い間隔に
// 落として Codex プロセスへの UIA IPC 圧を下げる。
const ACTIVE_POLL_MS = 300;
const IDLE_POLL_MS = 1200;
const AUTO_APPROVE_COOLDOWN_MS = 1500;

function App() {
  const [state, setState] = useState<GuardState | null>(null);
  const [observation, setObservation] = useState<ApprovalObservation | null>(null);
  const [autoApprove, setAutoApprove] = useState<AutoApproveOutcome | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(null);

  interface LastAction {
    action: string;
    risk: string;
    matched_rule?: string | null;
    target: string;
    time: number;
  }
  const [lastAction, setLastAction] = useState<LastAction | null>(null);
  const [lastActionShowUntil, setLastActionShowUntil] = useState<number>(0);

  const checkUpdate = useCallback(async () => {
    try {
      const result = await callBackend<UpdateCheckResult>("check_for_app_update");
      setUpdateResult(result);
    } catch (err) {
      console.error("Update check failed:", err);
    }
  }, []);

  useEffect(() => {
    void checkUpdate();
  }, [checkUpdate]);

  useEffect(() => {
    if (lastActionShowUntil === 0) return;
    const delay = lastActionShowUntil - Date.now();
    if (delay <= 0) {
      setLastActionShowUntil(0);
      return;
    }
    const id = window.setTimeout(() => {
      setLastActionShowUntil(0);
    }, delay);
    return () => window.clearTimeout(id);
  }, [lastActionShowUntil]);

  const handleUpdateClick = async (url: string) => {
    try {
      await callBackend("open_url", { url });
    } catch (err) {
      setError(String(err));
    }
  };


  const copyLogPath = async () => {
    if (!state?.audit_log_path) return;
    try {
      await navigator.clipboard.writeText(state.audit_log_path);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      // ignore
    }
  };

  const pollingRef = useRef(false);
  const pausedRef = useRef(false);
  const lastApprovalAtRef = useRef(0);
  // 直近の observation サイクルで dialog を検出したか。自適応 polling
  // のサイクル間隔決定にだけ使う。
  const lastSawDialogRef = useRef(false);

  const loadState = useCallback(async () => {
    const nextState = await callBackend<GuardState>("get_guard_state");
    setState(nextState);
  }, []);

  useEffect(() => {
    loadState().catch((err) => setError(String(err)));
  }, [loadState]);

  const policy = state?.policy;
  const recentAudits = state?.recent_audits ?? [];
  const displayedAudits = useMemo(
    () => recentAudits.slice(0, AUDIT_DISPLAY_LIMIT),
    [recentAudits],
  );

  useEffect(() => {
    pausedRef.current = Boolean(policy?.paused);
  }, [policy?.paused]);

  const setPaused = async (paused: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await callBackend<PolicyConfig>("set_guard_paused", { paused });
      await loadState();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const setAllowGitAdd = async (allow: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await callBackend<PolicyConfig>("set_allow_git_add", { allow });
      await loadState();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const setAllowGitCommit = async (allow: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await callBackend<PolicyConfig>("set_allow_git_commit", { allow });
      await loadState();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const runObservation = useCallback(
    async ({ manual }: { manual: boolean }) => {
      if (pollingRef.current) return;
      pollingRef.current = true;
      try {
        const result = await callBackend<ApprovalObservation>("observe_approval_request");
        setObservation(result);
        lastSawDialogRef.current = Boolean(result.observed);
        const inCooldown =
          !manual && Date.now() - lastApprovalAtRef.current < AUTO_APPROVE_COOLDOWN_MS;
        const autoAction = result.decision?.action;
        const shouldAutoAct = autoAction === "approve" || autoAction === "dismiss" || autoAction === "deny";
        if (shouldAutoAct && result.observed && !inCooldown) {
          try {
            const auto = await callBackend<AutoApproveOutcome>("auto_approve_observed_request", {
              request: result.observed.request,
            });
            setAutoApprove(auto);
            lastApprovalAtRef.current = Date.now();
            setLastAction({
              action: auto.decision.action,
              risk: auto.decision.risk,
              matched_rule: auto.decision.matched_rule,
              target: result.observed.request.command || result.observed.request.window_title,
              time: Date.now(),
            });
            setLastActionShowUntil(Date.now() + 5000);
          } catch (autoErr) {
            // Background errors don't block
          }
        }
        await loadState();
      } catch (err) {
        // Ignore background polling errors to keep UI stable
        // 失敗時は次サイクルを「アイドル」扱いで長めの間隔に落とす。
        // 連続失敗の本格的な保護は backend 側の circuit breaker が担う。
        lastSawDialogRef.current = false;
      } finally {
        pollingRef.current = false;
      }
    },
    [loadState],
  );

  useEffect(() => {
    // 「完了 → 次の遅延」の連鎖型 setTimeout。間隔は直近サイクルの結果に応じて
    // 動的に決める: dialog 検出時は ACTIVE_POLL_MS、それ以外は IDLE_POLL_MS。
    // setInterval を使わない理由は、UIA 走査が遅延した時に呼び出しがキューに
    // 溜まらないようにするため。
    let cancelled = false;
    let timer: number | null = null;

    const schedule = () => {
      if (cancelled) return;
      const delay = lastSawDialogRef.current ? ACTIVE_POLL_MS : IDLE_POLL_MS;
      timer = window.setTimeout(async () => {
        timer = null;
        if (!cancelled && !pausedRef.current) {
          try {
            await runObservation({ manual: false });
          } catch {
            // runObservation は内部で例外を握り潰すが、念のためここでも握って次サイクルを継続する。
          }
        }
        if (!cancelled) schedule();
      }, delay);
    };

    schedule();
    return () => {
      cancelled = true;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [runObservation]);

  const formatTime = (isoString: string) => {
    const date = new Date(isoString);
    if (Number.isNaN(date.getTime())) {
      return isoString;
    }
    return date.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    });
  };
  const showPersistentAction = lastAction && Date.now() < lastActionShowUntil;

  const getCardClass = () => {
    if (policy?.paused) return "paused";
    if (showPersistentAction) {
      return lastAction.action === "deny" ? "deny" : "alert";
    }
    return observation?.observed ? "alert" : "monitoring";
  };

  return (
    <main className="app-shell">
      <div className="workspace">
        <header className="topbar">
          <div className="brand">
            <ShieldCheck size={22} aria-hidden className="brand-icon" />
            <h1>Codex 承認ガード</h1>
          </div>
          <button
            type="button"
            className={`status-toggle-btn ${policy?.paused ? "paused" : "active"}`}
            onClick={() => setPaused(!policy?.paused)}
            disabled={busy || !policy}
            aria-label={policy?.paused ? "ガードを再開" : "ガードを停止"}
          >
            <span className="status-dot"></span>
            {policy?.paused ? "停止中" : "監視中"}
          </button>
        </header>

        {updateResult?.hasUpdate && (
          <div className="update-banner" role="status">
            <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
              <Activity size={14} className="pulse-icon" />
              <span>新しいバージョン (v{updateResult.latestVersion}) が利用可能です。</span>
            </div>
            <button
              type="button"
              className="update-btn"
              onClick={() => handleUpdateClick(updateResult.downloadUrl || updateResult.releaseUrl)}
            >
              アップデート
            </button>
          </div>
        )}

        {/* Exceptions / git allow settings */}
        <section className="exceptions-section" aria-label="自動承認の例外設定">
          <div className="exceptions-header">
            <Settings size={14} aria-hidden />
            <h2>自動承認の例外</h2>
          </div>
          <div className="exceptions-list">
            <SwitchRow
              icon={<GitCommit size={14} aria-hidden />}
              label="git add"
              description="ステージング操作を自動承認"
              checked={Boolean(policy?.allow_git_add)}
              disabled={busy || !policy}
              onChange={(next) => setAllowGitAdd(next)}
            />
            <SwitchRow
              icon={<GitCommit size={14} aria-hidden />}
              label="git commit"
              description="コミット操作を自動承認（commit ダイアログは別途常に自動閉鎖）"
              checked={Boolean(policy?.allow_git_commit)}
              disabled={busy || !policy}
              onChange={(next) => setAllowGitCommit(next)}
            />
          </div>
        </section>

        {error ? (
          <div className="error-line" role="alert">
            <AlertTriangle size={15} aria-hidden />
            <span>{error}</span>
          </div>
        ) : null}

        {/* Dynamic Status Card */}
        <section className={`status-card ${getCardClass()}`}>
          {policy?.paused ? (
            <div className="card-content paused">
              <AlertTriangle size={32} className="status-icon" />
              <div>
                <h2>自動承認は停止しています</h2>
                <p>Codex からの承認リクエストは自動承認されません。</p>
              </div>
            </div>
          ) : showPersistentAction && lastAction ? (
            <div className="card-content active-event">
              <div className="event-header">
                {lastAction.action === "deny" ? (
                  <span className="event-badge deny">自動拒否済</span>
                ) : lastAction.action === "dismiss" ? (
                  <span className="event-badge dismiss">自動閉鎖済</span>
                ) : (
                  <span className="event-badge approve">自動承認済</span>
                )}
                <span className="event-time">直近の自動操作</span>
              </div>
              <div className="event-details">
                <div className="event-field">
                  <span className="label">対象操作</span>
                  <strong className="value" title={lastAction.target}>
                    {lastAction.target}
                  </strong>
                </div>
                <div className="event-field-row">
                  <div>
                    <span className="label">判定</span>
                    <span className="value-decision" style={{ color: lastAction.action === "deny" ? "var(--color-danger)" : "var(--color-success)" }}>
                      {actionLabels[lastAction.action]} ({riskLabels[lastAction.risk]})
                    </span>
                  </div>
                  {lastAction.matched_rule && (
                    <div>
                      <span className="label">適用ルール</span>
                      <span className="value-rule">{lastAction.matched_rule}</span>
                    </div>
                  )}
                </div>
              </div>
            </div>
          ) : observation?.observed ? (
            <div className="card-content active-event">
              <div className="event-header">
                {observation.decision?.action === "deny" ? (
                  <span className="event-badge deny">自動拒否済</span>
                ) : observation.decision?.action === "dismiss" ? (
                  <span className="event-badge dismiss">自動閉鎖済</span>
                ) : (
                  <span className="event-badge approve">自動承認済</span>
                )}
                <span className="event-time">{observation.decision ? "最新の検出" : "検出中"}</span>
              </div>
              <div className="event-details">
                <div className="event-field">
                  <span className="label">対象操作</span>
                  <strong className="value" title={observation.observed.request.command || observation.observed.request.window_title}>
                    {observation.observed.request.command || observation.observed.request.window_title}
                  </strong>
                </div>
                <div className="event-field-row">
                  <div>
                    <span className="label">判定</span>
                    <span className="value-decision" style={{ color: observation.decision?.action === "deny" ? "var(--color-danger)" : "var(--color-success)" }}>
                      {observation.decision ? actionLabels[observation.decision.action] : "未判定"}
                      {observation.decision && ` (${riskLabels[observation.decision.risk]})`}
                    </span>
                  </div>
                  {observation.decision?.matched_rule && (
                    <div>
                      <span className="label">適用ルール</span>
                      <span className="value-rule">{observation.decision.matched_rule}</span>
                    </div>
                  )}
                </div>
              </div>
            </div>
          ) : (
            <div className="card-content monitoring">
              <div className="pulse-container">
                <div className="pulse-ring"></div>
                <Activity size={24} className="pulse-icon" />
              </div>
              <div>
                <h2>承認リクエストを監視中...</h2>
                <p>Codex 承認ウィンドウの検出を待機しています。</p>
              </div>
            </div>
          )}
        </section>

        {/* Audit Log list */}
        <section className="audit-section">
          <div className="audit-header">
            <div className="audit-title">
              <FileClock size={16} />
              <h2>直近の履歴</h2>
            </div>
            {state?.audit_log_path && (
              <button
                type="button"
                className="copy-path-btn"
                onClick={copyLogPath}
                title="ログファイルのパスをコピー"
              >
                {copied ? <Check size={12} className="success-icon" /> : <Copy size={12} />}
                <span>パスをコピー</span>
              </button>
            )}
          </div>

          <div className="audit-list">
            {displayedAudits.length > 0 ? (
              displayedAudits.map((entry: AuditEntry) => (
                <article className={`audit-item ${entry.decision.action}`} key={entry.id}>
                  <div className="item-meta">
                    <span className="item-time">{formatTime(entry.created_at)}</span>
                    <span className={`item-action-badge ${entry.decision.action}`}>
                      {actionLabels[entry.decision.action]}
                    </span>
                  </div>
                  <div className="item-content">
                    <span className="item-target" title={entry.request.command || entry.request.window_title}>
                      {entry.request.command || entry.request.window_title}
                    </span>
                    <span className="item-reason" title={entry.decision.reason}>
                      {entry.decision.reason.split("（")[0].split("(")[0].trim()}
                    </span>
                  </div>
                </article>
              ))
            ) : (
              <div className="empty-state">履歴はありません。</div>
            )}
          </div>
        </section>
      </div>
    </main>
  );
}

interface SwitchRowProps {
  icon: ReactNode;
  label: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onChange: (next: boolean) => void;
}

function SwitchRow({ icon, label, description, checked, disabled, onChange }: SwitchRowProps) {
  return (
    <label className={`switch-row ${checked ? "on" : "off"} ${disabled ? "disabled" : ""}`}>
      <span className="switch-leading">
        <span className="switch-icon" aria-hidden>{icon}</span>
        <span className="switch-text">
          <span className="switch-label">{label}</span>
          <span className="switch-description">{description}</span>
        </span>
      </span>
      <span className="switch-trailing">
        <span className={`switch-state-label ${checked ? "on" : "off"}`}>
          {checked ? "許可" : "拒否"}
        </span>
        <span className="switch-control" role="presentation">
          <input
            type="checkbox"
            checked={checked}
            disabled={disabled}
            onChange={(event) => onChange(event.currentTarget.checked)}
            aria-label={`${label} を ${checked ? "拒否" : "許可"} に切り替え`}
          />
          <span className="switch-track" aria-hidden>
            <span className="switch-thumb" />
          </span>
        </span>
      </span>
    </label>
  );
}

export default App;
