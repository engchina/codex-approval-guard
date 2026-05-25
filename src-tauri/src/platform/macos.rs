use super::PlatformSnapshot;

pub fn snapshot_active_window() -> PlatformSnapshot {
    PlatformSnapshot {
        backend: "macOS Accessibility".to_string(),
        available: false,
        focused_window_title: None,
        details: "Accessibility adapter は未接続です。macOS では現在 Codex 承認ウィンドウの自動検出・自動承認を行いません。"
            .to_string(),
    }
}
