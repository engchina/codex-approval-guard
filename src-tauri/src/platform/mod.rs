use crate::policy::ApprovalRequest;

mod parser;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlatformSnapshot {
    pub backend: String,
    pub available: bool,
    pub focused_window_title: Option<String>,
    pub details: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ObservedApproval {
    pub request: ApprovalRequest,
    pub raw_text: Vec<String>,
    pub detected_by: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ObserveDiagnostics {
    pub windows_scanned: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ClickOutcome {
    pub target_window: String,
    pub yes_invoked: bool,
    pub submit_invoked: bool,
    /// 実際に成功した自動操作の経路ラベル（例: "wm-close", "escape-attach",
    /// "escape-broadcast", "uia-close-button", "uia-yes-button", "uia-no-button"）。
    /// audit log の reason 末尾に短い形で付加され、原因追跡を容易にする。
    /// `notes` は冗長な診断ログのため audit には載せず、メモリ上のみで保持する。
    #[serde(default)]
    pub method: Option<String>,
    pub notes: Vec<String>,
}

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
pub fn snapshot_active_window() -> PlatformSnapshot {
    windows::snapshot_active_window()
}

#[cfg(target_os = "windows")]
pub fn observe_approval_request() -> Result<(Option<ObservedApproval>, ObserveDiagnostics), String>
{
    windows::observe_approval_request()
}

#[cfg(target_os = "windows")]
pub fn click_yes_in_codex_approval(is_git_commit_hint: bool) -> Result<ClickOutcome, String> {
    windows::click_yes_in_codex_approval(is_git_commit_hint)
}

#[cfg(target_os = "windows")]
pub fn click_no_in_codex_approval(is_git_commit_hint: bool) -> Result<ClickOutcome, String> {
    windows::click_no_in_codex_approval(is_git_commit_hint)
}

#[cfg(target_os = "windows")]
pub fn user_idle_ms() -> u32 {
    windows::user_idle_ms()
}

#[cfg(target_os = "macos")]
pub fn snapshot_active_window() -> PlatformSnapshot {
    macos::snapshot_active_window()
}

#[cfg(target_os = "macos")]
pub fn observe_approval_request() -> Result<(Option<ObservedApproval>, ObserveDiagnostics), String>
{
    Ok((None, ObserveDiagnostics::default()))
}

#[cfg(target_os = "macos")]
pub fn click_yes_in_codex_approval(_is_git_commit_hint: bool) -> Result<ClickOutcome, String> {
    Err("macOS では自動承認は未対応です。".to_string())
}

#[cfg(target_os = "macos")]
pub fn click_no_in_codex_approval(_is_git_commit_hint: bool) -> Result<ClickOutcome, String> {
    Err("macOS では自動拒否は未対応です。".to_string())
}

#[cfg(target_os = "macos")]
pub fn user_idle_ms() -> u32 {
    u32::MAX
}
