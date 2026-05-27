//! 回路ブレーカー: Codex 側の異常時に自動操作を無限ループさせない保護層。
//!
//! 2 つの独立した検出ロジックを束ねる:
//!
//! 1. **Auto-approve burst guard** — 同じ command 文字列が短時間に連続して
//!    auto-approve されるパターンを検出する。Codex がプランループに陥り
//!    同じ承認を吐き続けるケース、もしくは本ガード側のマッチャ誤検出で
//!    意図しないボタンを叩き続けるケースで作動する。
//!
//! 2. **UIA observe failure guard** — `observe_approval_request` が連続して
//!    失敗（スレッド起動失敗 / UIA timeout / Win32 エラー）した場合に作動する。
//!    Codex が応答停止している、UIA サービスが死んでいる、本プロセスの
//!    権限が剥奪された、などのケースで作動する。
//!
//! どちらが作動しても呼び出し側で `policy.paused = true` を強制する想定。
//! 作動メッセージは **しきい値到達の瞬間** だけ返し、それ以降は同じ状態に
//! 留まっても `None` を返す（次に成功が観測されるまで一度きり）。これで
//! audit log の重複を防ぐ。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 同 command が auto-approve される頻度を見る窓。
pub const AUTO_APPROVE_WINDOW: Duration = Duration::from_secs(60);
/// 窓内で同 command の auto-approve がこの回数を超えると作動。
pub const MAX_AUTO_APPROVES_IN_WINDOW: usize = 5;
/// UIA observe が連続でこの回数失敗すると作動。
pub const MAX_CONSECUTIVE_UIA_FAILURES: u32 = 5;

#[derive(Debug, Default)]
pub struct GuardCircuit {
    inner: Mutex<GuardCircuitInner>,
}

#[derive(Debug, Default)]
struct GuardCircuitInner {
    /// command key → 直近 auto-approve タイムスタンプの並び。
    recent_auto_approvals: HashMap<String, VecDeque<Instant>>,
    /// command key を直近で auto-approve burst の理由として記録済みか。
    /// 重複 audit 抑止用。次に non-trigger な auto-approve が来れば解除。
    auto_approve_armed_keys: HashMap<String, bool>,
    /// UIA observe の連続失敗回数。成功が来たら 0 に戻す。
    uia_consecutive_failures: u32,
    /// UIA 失敗 guard がすでに作動済みか。次に成功が来たら解除。
    uia_failure_armed: bool,
}

impl GuardCircuit {
    pub fn new() -> Self {
        Self::default()
    }

    /// auto-approve の実行を記録する。`command_key` は dedup の単位
    /// （通常は `request.command` を使い、空のときは window_title 等で代用）。
    /// しきい値に到達した瞬間にだけ `Some(reason)` を返す。
    pub fn record_auto_approve(&self, command_key: &str) -> Option<String> {
        self.record_auto_approve_at(command_key, Instant::now())
    }

    /// テスト用に時刻を注入できる版。
    fn record_auto_approve_at(&self, command_key: &str, now: Instant) -> Option<String> {
        let mut guard = self.inner.lock().ok()?;
        let entry = guard
            .recent_auto_approvals
            .entry(command_key.to_string())
            .or_default();
        entry.push_back(now);
        // 古い記録を捨てる。
        while let Some(front) = entry.front().copied() {
            if now.duration_since(front) > AUTO_APPROVE_WINDOW {
                entry.pop_front();
            } else {
                break;
            }
        }
        let count = entry.len();
        if count <= MAX_AUTO_APPROVES_IN_WINDOW {
            // しきい値を超えていない: armed 状態を解除（次の超過で再び発火させる）。
            guard
                .auto_approve_armed_keys
                .insert(command_key.to_string(), false);
            return None;
        }
        let already_armed = guard
            .auto_approve_armed_keys
            .get(command_key)
            .copied()
            .unwrap_or(false);
        if already_armed {
            return None;
        }
        guard
            .auto_approve_armed_keys
            .insert(command_key.to_string(), true);
        Some(format!(
            "同一コマンド「{}」が直近 {} 秒間に {} 回連続で自動承認されました。Codex 側のループまたは誤検出の可能性があるため、ガードを一時停止します。",
            truncate_for_message(command_key, 80),
            AUTO_APPROVE_WINDOW.as_secs(),
            count,
        ))
    }

    /// UIA observe の結果を記録する。`ok = false` が連続して
    /// `MAX_CONSECUTIVE_UIA_FAILURES` 回に達した瞬間にだけ `Some(reason)` を返す。
    pub fn record_observe_result(&self, ok: bool) -> Option<String> {
        let mut guard = self.inner.lock().ok()?;
        if ok {
            guard.uia_consecutive_failures = 0;
            guard.uia_failure_armed = false;
            return None;
        }
        guard.uia_consecutive_failures = guard.uia_consecutive_failures.saturating_add(1);
        if guard.uia_consecutive_failures < MAX_CONSECUTIVE_UIA_FAILURES {
            return None;
        }
        if guard.uia_failure_armed {
            return None;
        }
        guard.uia_failure_armed = true;
        Some(format!(
            "UI Automation の観測が {} 回連続で失敗しました。Codex が応答停止しているか UIA service が利用できない可能性があるため、ガードを一時停止します。",
            guard.uia_consecutive_failures
        ))
    }
}

fn truncate_for_message(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_chars {
        value.to_string()
    } else {
        let mut out: String = chars.iter().take(max_chars).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_approve_burst_triggers_at_threshold_only_once() {
        let circuit = GuardCircuit::new();
        let key = "git status";
        let base = Instant::now();
        // 1..=MAX 回はトリガしない。
        for i in 0..MAX_AUTO_APPROVES_IN_WINDOW {
            let result =
                circuit.record_auto_approve_at(key, base + Duration::from_millis(i as u64 * 10));
            assert!(result.is_none(), "iteration {i} should not trigger");
        }
        // MAX+1 回目で発火。
        let trigger = circuit.record_auto_approve_at(key, base + Duration::from_millis(1000));
        assert!(trigger.is_some(), "should trigger at threshold+1");
        // さらに同じ key を打っても再発火しない。
        let again = circuit.record_auto_approve_at(key, base + Duration::from_millis(1100));
        assert!(again.is_none(), "should not refire while armed");
    }

    #[test]
    fn auto_approve_old_entries_age_out_of_window() {
        let circuit = GuardCircuit::new();
        let key = "git status";
        let base = Instant::now();
        // 4 回打って armed をリセット可能な状態にしておく。
        for i in 0..4 {
            assert!(circuit
                .record_auto_approve_at(key, base + Duration::from_millis(i * 10))
                .is_none());
        }
        // 60s 後に同じコマンドを 1 回打つ → 古い 4 件は窓外に出るのでカウントは 1。
        let result = circuit.record_auto_approve_at(key, base + Duration::from_secs(120));
        assert!(result.is_none(), "old entries should age out");
    }

    #[test]
    fn auto_approve_different_keys_do_not_share_counter() {
        let circuit = GuardCircuit::new();
        let base = Instant::now();
        for i in 0..MAX_AUTO_APPROVES_IN_WINDOW {
            assert!(circuit
                .record_auto_approve_at("git status", base + Duration::from_millis(i as u64))
                .is_none());
            assert!(circuit
                .record_auto_approve_at("npm test", base + Duration::from_millis(i as u64))
                .is_none());
        }
        // どちらも MAX 回ずつだが、key が違うのでまだトリガしない。
    }

    #[test]
    fn uia_failure_triggers_after_threshold_only_once() {
        let circuit = GuardCircuit::new();
        for i in 0..MAX_CONSECUTIVE_UIA_FAILURES - 1 {
            assert!(
                circuit.record_observe_result(false).is_none(),
                "iteration {i} should not trigger"
            );
        }
        // しきい値ちょうどで発火。
        let trigger = circuit.record_observe_result(false);
        assert!(trigger.is_some(), "should trigger at threshold");
        // 連続でさらに失敗しても再発火しない。
        let again = circuit.record_observe_result(false);
        assert!(again.is_none(), "should not refire while armed");
    }

    #[test]
    fn uia_failure_counter_resets_on_success() {
        let circuit = GuardCircuit::new();
        for _ in 0..(MAX_CONSECUTIVE_UIA_FAILURES - 1) {
            assert!(circuit.record_observe_result(false).is_none());
        }
        // 成功が来たら 0 に戻る。
        assert!(circuit.record_observe_result(true).is_none());
        // 再びしきい値直前まで失敗を積む。さっきの失敗が混ざっていれば発火するはず。
        for _ in 0..(MAX_CONSECUTIVE_UIA_FAILURES - 1) {
            assert!(circuit.record_observe_result(false).is_none());
        }
        // ここまでで再発火していないことを確認 (=連続失敗の数え直しが効いている)。
    }
}
