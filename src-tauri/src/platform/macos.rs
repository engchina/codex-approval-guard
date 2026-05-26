//! macOS Accessibility (AX) backend.
//!
//! Windows 版 (`windows.rs`) と同等の挙動を、macOS の Accessibility API を介して提供する。
//! 主な責務:
//! - Codex Desktop プロセス（bundle identifier ベース）を `NSWorkspace.runningApplications` で
//!   列挙する。
//! - AX ツリーを再帰走査して title / description / value を文字列として収集し、
//!   `parser::parse_observed_approval_with_context` に渡して承認候補を抽出する。
//! - `kAXPressAction` で「1. はい」「3. いいえ」「閉じる」「送信」ボタンを押下し、
//!   `kAXRaiseAction` でサイドバーの承認待ち会話をアクティブ化する。
//! - `CGEventSourceSecondsSinceLastEventType` で直近の入力からの idle 時間を取得する。
//!
//! Accessibility 権限（システム設定 → プライバシーとセキュリティ → アクセシビリティ）が必要。
//! 権限未許可の場合は `snapshot_active_window` がその旨を `details` に載せて返す。
//!
//! 設計判断: AX 呼び出しはどのスレッドからでも安全に行えるため Windows 版のような UIA 専用
//! スレッドは設けない。Tauri 側の `spawn_blocking` が既にワーカースレッドを用意してくれる。

#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use super::matchers::{
    is_close_or_cancel_button, is_first_no_option, is_first_yes_or_recommended_option,
    is_recommended_option, is_submit_button, looks_like_approval_keyword,
};
use super::parser::{
    is_pending_approval_badge, looks_like_git_commit_window, parse_observed_approval_with_context,
    title_matches_git_commit,
};
use super::{ClickOutcome, ObserveDiagnostics, ObservedApproval, PlatformSnapshot};
use crate::policy::ApprovalRequest;

use accessibility_sys::{
    kAXChildrenAttribute, kAXDescriptionAttribute, kAXFocusedApplicationAttribute,
    kAXFocusedWindowAttribute, kAXHelpAttribute, kAXPressAction, kAXRaiseAction, kAXRoleAttribute,
    kAXSubroleAttribute, kAXTitleAttribute, kAXTrustedCheckOptionPrompt, kAXValueAttribute,
    kAXWindowsAttribute, AXError, AXIsProcessTrustedWithOptions, AXUIElementCopyAttributeValue,
    AXUIElementCreateApplication, AXUIElementCreateSystemWide, AXUIElementGetTypeID,
    AXUIElementPerformAction, AXUIElementRef,
};
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFRelease, CFRetain, CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use core_foundation_sys::base::{CFGetTypeID, CFTypeID};
use objc2::rc::Retained;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSString;

use std::ffi::c_void;

const MAX_TOP_LEVEL_WINDOWS: usize = 120;
const MAX_TEXT_LINES: usize = 1200;
const MAX_TREE_DEPTH: usize = 20;
const MAX_CLICK_TREE_DEPTH: usize = 60;
const MAX_DUMP_ENTRIES: usize = 80;

/// `kCGAnyInputEventType` 相当の sentinel。Apple の CGEventTypes.h で `(~0)` と定義されており、
/// `CGEventSourceSecondsSinceLastEventType` に渡すと「全イベント種別の中で最も最近の入力」を
/// 返す。core-graphics crate の `CGEventType` enum には対応するバリアントが無いため、ここで
/// 直接 u32 として宣言する。
const K_CG_ANY_INPUT_EVENT_TYPE: u32 = u32::MAX;
/// `kCGEventSourceStateHIDSystemState = 1`。core-graphics の `CGEventSourceStateID` enum の
/// HIDSystemState バリアントと同値。FFI でも i32 として扱う。
const K_CG_EVENT_SOURCE_HID_SYSTEM_STATE: i32 = 1;

#[cfg_attr(target_os = "macos", link(name = "CoreGraphics", kind = "framework"))]
extern "C" {
    /// 最後に当該イベント種別の入力があってからの経過秒数を返す。
    /// `eventType` に `K_CG_ANY_INPUT_EVENT_TYPE` を渡せば「任意の入力イベント」が対象となる。
    fn CGEventSourceSecondsSinceLastEventType(source: i32, event_type: u32) -> f64;
}

/// Codex Desktop / ChatGPT デスクトップアプリの bundle identifier 候補。
/// 配布形態の変更に追随できるよう既知のものを列挙し、`contains` 判定で寛容にマッチさせる。
const CODEX_BUNDLE_HINTS: &[&str] = &[
    "com.openai.codex",
    "com.openai.chat",
    "com.openai.chatgpt",
    "ai.openai.codex",
    "ai.openai.chat",
    "ai.openai.chatgpt",
    "openai.codex",
    "openai.chat",
    "openai.chatgpt",
];
const CODEX_NAME_HINTS: &[&str] = &["codex", "chatgpt"];

// =============================================================================
// 公開 API
// =============================================================================

pub fn snapshot_active_window() -> PlatformSnapshot {
    if !is_accessibility_trusted() {
        return PlatformSnapshot {
            backend: "macOS Accessibility".to_string(),
            available: false,
            focused_window_title: None,
            details: "アクセシビリティ権限が許可されていません。システム設定 → プライバシーとセキュリティ → アクセシビリティ で Codex 承認ガード を許可してください。".to_string(),
        };
    }

    let title = focused_window_title();
    let details = match &title {
        Some(_) => "AX 経由でフォアグラウンドウィンドウを取得できました。".to_string(),
        None => "フォアグラウンドウィンドウを特定できませんでした。".to_string(),
    };
    PlatformSnapshot {
        backend: "macOS Accessibility".to_string(),
        available: true,
        focused_window_title: title,
        details,
    }
}

pub fn observe_approval_request() -> Result<(Option<ObservedApproval>, ObserveDiagnostics), String>
{
    if !is_accessibility_trusted() {
        return Err(
            "アクセシビリティ権限が許可されていません。システム設定で許可してください。"
                .to_string(),
        );
    }

    let mut diagnostics = ObserveDiagnostics::default();
    let apps = codex_running_apps();
    if apps.is_empty() {
        diagnostics
            .notes
            .push("Codex Desktop プロセスが見つかりません。".to_string());
        return Ok((None, diagnostics));
    }

    for app in apps.iter() {
        let pid = app.pid;
        let app_element = unsafe { AxElement::from_owned(AXUIElementCreateApplication(pid)) };
        let windows = match copy_attribute_array(&app_element, kAXWindowsAttribute) {
            Some(arr) => arr,
            None => {
                diagnostics.notes.push(format!(
                    "pid={} ({}) からウィンドウ配列を取得できません。",
                    pid,
                    short(&app.name, 40)
                ));
                continue;
            }
        };

        for window in windows.into_iter().take(MAX_TOP_LEVEL_WINDOWS) {
            diagnostics.windows_scanned += 1;
            let title = element_string_attribute(&window, kAXTitleAttribute).unwrap_or_default();
            diagnostics
                .notes
                .push(format!("候補: pid={} title=\"{}\"", pid, short(&title, 60)));
            if let Some(observed) = parse_window(&window, &title, &mut diagnostics) {
                return Ok((Some(observed), diagnostics));
            }
        }
    }

    Ok((None, diagnostics))
}

pub fn click_yes_in_codex_approval(is_git_commit_hint: bool) -> Result<ClickOutcome, String> {
    if !is_accessibility_trusted() {
        return Err(
            "アクセシビリティ権限が許可されていません。システム設定で許可してください。"
                .to_string(),
        );
    }

    let apps = codex_running_apps();
    if apps.is_empty() {
        return Err("Codex Desktop プロセスが見つかりません。".to_string());
    }

    let mut outcome = ClickOutcome::default();

    for app in apps.iter() {
        let pid = app.pid;
        let app_element = unsafe { AxElement::from_owned(AXUIElementCreateApplication(pid)) };
        let windows = match copy_attribute_array(&app_element, kAXWindowsAttribute) {
            Some(arr) => arr,
            None => continue,
        };

        for window in windows.into_iter().take(MAX_TOP_LEVEL_WINDOWS) {
            let title = element_string_attribute(&window, kAXTitleAttribute).unwrap_or_default();
            let title_says_git_commit = title_matches_git_commit(&title);

            let is_git_commit = if title_says_git_commit {
                true
            } else {
                let mut raw_text = Vec::new();
                let mut keyword_hit = false;
                collect_text(&window, 0, &mut raw_text, &mut keyword_hit);
                looks_like_git_commit_window(&title, &raw_text)
            };

            if is_git_commit_hint && !is_git_commit {
                outcome.notes.push(format!(
                    "Git commit hint はありますが、このウィンドウは commit dialog ではないためスキップします: pid={} title=\"{}\"",
                    pid,
                    short(&title, 60),
                ));
                continue;
            }

            outcome.target_window = title.clone();
            outcome.notes.push(format!(
                "ターゲット: pid={} title=\"{}\" git_commit={}",
                pid,
                short(&title, 60),
                is_git_commit
            ));

            // git commit dialog: AX 経由で close 系ボタンを直接押す。Windows 版の WM_CLOSE /
            // Escape 経路は macOS では成立しないため、ここでは UIA close-button 相当のみを試す。
            let (matcher, label): (fn(&str) -> bool, &str) = if is_git_commit {
                (is_close_or_cancel_button, "「閉じる/キャンセル」")
            } else {
                (
                    is_first_yes_or_recommended_option,
                    "「1. はい / N. (推奨)」",
                )
            };

            let mut candidates = collect_candidates(&window, matcher);
            if is_git_commit && !title_says_git_commit {
                candidates.retain(|candidate| !is_native_window_close_candidate(candidate));
            }
            log_candidates(&mut outcome, label, &candidates);

            let yes_target = match pick_best(&candidates) {
                Some(target) => target,
                None => {
                    if !is_git_commit {
                        let sidebar_targets = collect_pending_approval_sidebar_targets(&window);
                        log_candidates(&mut outcome, "「承認待ち」バッジ", &sidebar_targets);
                        if let Some(sidebar_target) = pick_best(&sidebar_targets) {
                            invoke_candidate(sidebar_target).map_err(|error| {
                                format!(
                                    "サイドバーの承認待ち会話の自動アクティブ化に失敗しました: {error}"
                                )
                            })?;
                            outcome.notes.push(format!(
                                "サイドバーの承認待ち会話をアクティブ化しました: role=\"{}\" name=\"{}\"。次回ポーリングで承認操作を試行します。",
                                sidebar_target.role,
                                short(&sidebar_target.name, 60),
                            ));
                            return Ok(outcome);
                        }
                    }
                    let dump = dump_approval_tree(&window);
                    let dump_text = if dump.is_empty() {
                        "（承認関連要素なし）".to_string()
                    } else {
                        dump.join("\n")
                    };
                    return Err(format!(
                        "{}相当の要素が見つかりませんでした。Codex のダイアログが現在表示されていない可能性があります。\n\nAX dump (承認関連 / interactive 要素):\n{}",
                        label, dump_text
                    ));
                }
            };
            invoke_candidate(yes_target)
                .map_err(|error| format!("{}の AXPress に失敗しました: {}", label, error))?;
            outcome.notes.push(format!(
                "{}AXPress: role=\"{}\" name=\"{}\"",
                label,
                yes_target.role,
                short(&yes_target.name, 60),
            ));
            outcome.yes_invoked = true;
            outcome.method = Some(
                if is_git_commit {
                    "ax-close-button"
                } else if is_recommended_option(&yes_target.name) {
                    "ax-recommended-option"
                } else {
                    "ax-yes-button"
                }
                .to_string(),
            );

            if is_git_commit {
                outcome
                    .notes
                    .push("Git コミットダイアログのため「送信」処理をスキップします。".to_string());
                return Ok(outcome);
            }

            let mut submit_candidates = Vec::new();
            for _ in 0..3 {
                std::thread::sleep(std::time::Duration::from_millis(150));
                submit_candidates = collect_candidates(&window, is_submit_button);
                if !submit_candidates.is_empty() {
                    break;
                }
            }
            log_candidates(&mut outcome, "「送信」", &submit_candidates);
            match pick_best(&submit_candidates) {
                Some(submit_target) => {
                    invoke_candidate(submit_target).map_err(|error| {
                        format!(
                            "「1. はい」は選択しましたが、「送信」の AXPress に失敗しました: {error}。手動で送信してください。"
                        )
                    })?;
                    outcome.notes.push(format!(
                        "「提交」AXPress: role=\"{}\" name=\"{}\"",
                        submit_target.role,
                        short(&submit_target.name, 60),
                    ));
                    outcome.submit_invoked = true;
                }
                None => {
                    outcome.notes.push(
                        "「提交」ボタンが見つかりませんでした（既に送信された可能性）。"
                            .to_string(),
                    );
                }
            }

            try_activate_pending_sidebar_after_main(&window, &mut outcome);
            return Ok(outcome);
        }
    }

    Err("Codex プロセスのウィンドウが見つかりませんでした。".to_string())
}

pub fn click_no_in_codex_approval(is_git_commit_hint: bool) -> Result<ClickOutcome, String> {
    if !is_accessibility_trusted() {
        return Err(
            "アクセシビリティ権限が許可されていません。システム設定で許可してください。"
                .to_string(),
        );
    }

    let apps = codex_running_apps();
    if apps.is_empty() {
        return Err("Codex Desktop プロセスが見つかりません。".to_string());
    }

    let mut outcome = ClickOutcome::default();

    for app in apps.iter() {
        let pid = app.pid;
        let app_element = unsafe { AxElement::from_owned(AXUIElementCreateApplication(pid)) };
        let windows = match copy_attribute_array(&app_element, kAXWindowsAttribute) {
            Some(arr) => arr,
            None => continue,
        };

        for window in windows.into_iter().take(MAX_TOP_LEVEL_WINDOWS) {
            let title = element_string_attribute(&window, kAXTitleAttribute).unwrap_or_default();
            let title_says_git_commit = title_matches_git_commit(&title);
            let is_git_commit = if title_says_git_commit {
                true
            } else {
                let mut raw_text = Vec::new();
                let mut keyword_hit = false;
                collect_text(&window, 0, &mut raw_text, &mut keyword_hit);
                looks_like_git_commit_window(&title, &raw_text)
            };

            if is_git_commit_hint && !is_git_commit {
                outcome.notes.push(format!(
                    "Git commit hint はありますが、このウィンドウは commit dialog ではないためスキップします: pid={} title=\"{}\"",
                    pid,
                    short(&title, 60),
                ));
                continue;
            }

            outcome.target_window = title.clone();
            outcome.notes.push(format!(
                "ターゲット: pid={} title=\"{}\" git_commit={}",
                pid,
                short(&title, 60),
                is_git_commit
            ));

            let (matcher, label): (fn(&str) -> bool, &str) = if is_git_commit {
                (is_close_or_cancel_button, "「閉じる/キャンセル」")
            } else {
                (is_first_no_option, "「3. いいえ」")
            };

            let mut candidates = collect_candidates(&window, matcher);
            if is_git_commit && !title_says_git_commit {
                candidates.retain(|candidate| !is_native_window_close_candidate(candidate));
            }
            log_candidates(&mut outcome, label, &candidates);
            let no_target = match pick_best(&candidates) {
                Some(target) => target,
                None => {
                    let dump = dump_approval_tree(&window);
                    let dump_text = if dump.is_empty() {
                        "（承認関連要素なし）".to_string()
                    } else {
                        dump.join("\n")
                    };
                    return Err(format!(
                        "{}相当の要素が見つかりませんでした。Codex のダイアログが現在表示されていない可能性があります。\n\nAX dump (承認関連 / interactive 要素):\n{}",
                        label, dump_text
                    ));
                }
            };
            invoke_candidate(no_target)
                .map_err(|error| format!("{}の AXPress に失敗しました: {}", label, error))?;
            outcome.notes.push(format!(
                "{}AXPress: role=\"{}\" name=\"{}\"",
                label,
                no_target.role,
                short(&no_target.name, 60),
            ));
            outcome.yes_invoked = true;
            outcome.method = Some(
                if is_git_commit {
                    "ax-close-button"
                } else {
                    "ax-no-button"
                }
                .to_string(),
            );

            if is_git_commit {
                outcome
                    .notes
                    .push("Git コミットダイアログのため「送信」処理をスキップします。".to_string());
                return Ok(outcome);
            }

            let mut submit_candidates = Vec::new();
            for _ in 0..3 {
                std::thread::sleep(std::time::Duration::from_millis(150));
                submit_candidates = collect_candidates(&window, is_submit_button);
                if !submit_candidates.is_empty() {
                    break;
                }
            }
            log_candidates(&mut outcome, "「送信」", &submit_candidates);
            match pick_best(&submit_candidates) {
                Some(submit_target) => {
                    invoke_candidate(submit_target).map_err(|error| {
                        format!(
                            "「3. いいえ」は選択しましたが、「送信」の AXPress に失敗しました: {error}。手動で送信してください。"
                        )
                    })?;
                    outcome.notes.push(format!(
                        "「提交」AXPress: role=\"{}\" name=\"{}\"",
                        submit_target.role,
                        short(&submit_target.name, 60),
                    ));
                    outcome.submit_invoked = true;
                }
                None => {
                    outcome.notes.push(
                        "「提交」ボタンが見つかりませんでした（既に送信された可能性）。"
                            .to_string(),
                    );
                }
            }

            try_activate_pending_sidebar_after_main(&window, &mut outcome);
            return Ok(outcome);
        }
    }

    Err("Codex プロセスのウィンドウが見つかりませんでした。".to_string())
}

pub fn user_idle_ms() -> u32 {
    let seconds = unsafe {
        CGEventSourceSecondsSinceLastEventType(
            K_CG_EVENT_SOURCE_HID_SYSTEM_STATE,
            K_CG_ANY_INPUT_EVENT_TYPE,
        )
    };
    if !seconds.is_finite() || seconds < 0.0 {
        return 0;
    }
    let ms = seconds * 1000.0;
    if ms >= u32::MAX as f64 {
        u32::MAX
    } else {
        ms as u32
    }
}

// =============================================================================
// 内部実装
// =============================================================================

fn is_accessibility_trusted() -> bool {
    // prompt=false で「権限あるか確認だけ」する。ユーザーへの確認ダイアログは出さない。
    // 権限がない場合は snapshot_active_window 側で details に説明を載せる。
    // `kAXTrustedCheckOptionPrompt` は accessibility-sys では珍しく `CFStringRef` の静的変数
    // としてエクスポートされているため、そのまま CFDictionary のキーとして利用できる。
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let value = CFBoolean::false_value();
        let dict = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
        AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef())
    }
}

fn focused_window_title() -> Option<String> {
    let system_wide = unsafe { AxElement::from_owned(AXUIElementCreateSystemWide()) };
    let focused_app = element_child_attribute(&system_wide, kAXFocusedApplicationAttribute)?;
    let focused_window = element_child_attribute(&focused_app, kAXFocusedWindowAttribute)?;
    element_string_attribute(&focused_window, kAXTitleAttribute)
        .filter(|title| !title.trim().is_empty())
}

#[derive(Debug, Clone)]
struct CodexApp {
    pid: i32,
    name: String,
    #[allow(dead_code)]
    bundle_id: String,
}

fn codex_running_apps() -> Vec<CodexApp> {
    let mut found = Vec::new();
    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let apps = workspace.runningApplications();
        for app in apps.iter() {
            let bundle_id = ns_optional_string(app.bundleIdentifier()).unwrap_or_default();
            let name = ns_optional_string(app.localizedName()).unwrap_or_default();
            let bundle_id_lower = bundle_id.to_lowercase();
            let name_lower = name.to_lowercase();
            let is_codex_bundle = CODEX_BUNDLE_HINTS
                .iter()
                .any(|hint| bundle_id_lower.contains(hint));
            let is_codex_name = CODEX_NAME_HINTS
                .iter()
                .any(|hint| name_lower.contains(hint));
            // codex-approval-guard 自分自身は除外する。
            let is_guard_self = bundle_id_lower.contains("codex-approval-guard")
                || bundle_id_lower.contains("codex_approval_guard")
                || name_lower.contains("codex approval guard");
            if is_guard_self {
                continue;
            }
            if !is_codex_bundle && !is_codex_name {
                continue;
            }
            let pid = app.processIdentifier();
            if pid <= 0 {
                continue;
            }
            found.push(CodexApp {
                pid,
                name,
                bundle_id,
            });
        }
    }
    found
}

fn ns_optional_string(s: Option<Retained<NSString>>) -> Option<String> {
    s.map(|value| value.to_string())
}

// ----- AX element wrapper -----

/// AXUIElementRef を保持する RAII ラッパー。`Drop` で CFRelease する。
/// `unsafe` を局所化し、所有権の二重解放を防ぐ。
struct AxElement {
    raw: AXUIElementRef,
}

impl AxElement {
    /// 既に retain 済みの値を消費する。AX API の戻り値はほとんど retained ownership なので
    /// この形が標準。
    unsafe fn from_owned(raw: AXUIElementRef) -> Self {
        Self { raw }
    }

    fn as_raw(&self) -> AXUIElementRef {
        self.raw
    }
}

impl Drop for AxElement {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                CFRelease(self.raw as CFTypeRef);
            }
        }
    }
}

impl Clone for AxElement {
    fn clone(&self) -> Self {
        if !self.raw.is_null() {
            unsafe {
                CFRetain(self.raw as CFTypeRef);
            }
        }
        Self { raw: self.raw }
    }
}

// AXUIElementRef は CFType 由来のスレッドセーフな不透明ポインタ。spawn_blocking で別スレッドへ
// 渡すために Send/Sync を主張する。AX API は内部でロックを取るため、複数スレッドからの読み取りも
// 安全（Apple documentation 準拠）。
unsafe impl Send for AxElement {}
unsafe impl Sync for AxElement {}

// ----- 属性取得ヘルパー -----
//
// accessibility-sys 0.1.x では `kAX*Attribute` 系定数が `&str` で定義されている。
// AX API は CFStringRef を要求するため、各呼び出しで CFString::new(&str) を作って渡す必要が
// ある。CFString::new は内部で CFStringCreateWithBytes を呼ぶため許容できるオーバーヘッド。

fn ax_ui_element_type_id() -> CFTypeID {
    unsafe { AXUIElementGetTypeID() }
}

fn copy_attribute_raw(element: &AxElement, attribute_name: &str) -> Option<CFType> {
    if element.as_raw().is_null() {
        return None;
    }
    let attr_cfs = CFString::new(attribute_name);
    let mut value: CFTypeRef = std::ptr::null();
    let err: AXError = unsafe {
        AXUIElementCopyAttributeValue(element.as_raw(), attr_cfs.as_concrete_TypeRef(), &mut value)
    };
    if err != 0 || value.is_null() {
        return None;
    }
    Some(unsafe { CFType::wrap_under_create_rule(value) })
}

fn element_string_attribute(element: &AxElement, attribute_name: &str) -> Option<String> {
    let value = copy_attribute_raw(element, attribute_name)?;
    if !value.instance_of::<CFString>() {
        return None;
    }
    let string_ref: CFStringRef = value.as_CFTypeRef() as CFStringRef;
    if string_ref.is_null() {
        return None;
    }
    let s = unsafe { CFString::wrap_under_get_rule(string_ref) };
    Some(s.to_string())
}

fn element_child_attribute(element: &AxElement, attribute_name: &str) -> Option<AxElement> {
    let value = copy_attribute_raw(element, attribute_name)?;
    if value.type_of() != ax_ui_element_type_id() {
        return None;
    }
    let raw = value.as_CFTypeRef() as AXUIElementRef;
    if raw.is_null() {
        return None;
    }
    // CFType の Drop で release されないよう、ここで retain して所有権を移譲する。
    unsafe {
        CFRetain(raw as CFTypeRef);
        Some(AxElement::from_owned(raw))
    }
}

fn copy_attribute_array(element: &AxElement, attribute_name: &str) -> Option<Vec<AxElement>> {
    let value = copy_attribute_raw(element, attribute_name)?;
    if !value.instance_of::<CFArray<CFTypeRef>>() {
        return None;
    }
    let array_ref: CFArrayRef = value.as_CFTypeRef() as CFArrayRef;
    if array_ref.is_null() {
        return None;
    }
    let array = unsafe { CFArray::<CFTypeRef>::wrap_under_get_rule(array_ref) };
    let raw_values: Vec<*const c_void> = array.get_all_values();
    let mut out = Vec::with_capacity(raw_values.len());
    for item in raw_values {
        if item.is_null() {
            continue;
        }
        let item_ref = item as CFTypeRef;
        if unsafe { CFGetTypeID(item_ref) } != ax_ui_element_type_id() {
            continue;
        }
        let raw = item as AXUIElementRef;
        if raw.is_null() {
            continue;
        }
        unsafe {
            CFRetain(raw as CFTypeRef);
            out.push(AxElement::from_owned(raw));
        }
    }
    Some(out)
}

fn element_children(element: &AxElement) -> Vec<AxElement> {
    copy_attribute_array(element, kAXChildrenAttribute).unwrap_or_default()
}

/// 要素から「視認用の文字列」を一つ生成する。AX には可視ラベルになり得る属性が複数あるため
/// (title, description, value, help) 優先順にチェックし、空でない最初のものを採用する。
fn element_display_name(element: &AxElement) -> String {
    for attr in [
        kAXTitleAttribute,
        kAXDescriptionAttribute,
        kAXValueAttribute,
        kAXHelpAttribute,
    ] {
        if let Some(value) = element_string_attribute(element, attr) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    String::new()
}

fn element_role(element: &AxElement) -> String {
    element_string_attribute(element, kAXRoleAttribute).unwrap_or_default()
}

fn element_subrole(element: &AxElement) -> String {
    element_string_attribute(element, kAXSubroleAttribute).unwrap_or_default()
}

// ----- 観測フェーズ -----

fn parse_window(
    window: &AxElement,
    title: &str,
    diagnostics: &mut ObserveDiagnostics,
) -> Option<ObservedApproval> {
    if title_matches_git_commit(title) {
        diagnostics.notes.push(format!(
            "  parse: title=\"{}\" git commit short-circuit",
            short(title, 40)
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
            detected_by: "macOS Accessibility title-only observer (git commit)".to_string(),
        });
    }

    let mut raw_text = Vec::new();
    let mut keyword_hit = false;
    collect_text(window, 0, &mut raw_text, &mut keyword_hit);
    diagnostics.notes.push(format!(
        "  parse: title=\"{}\" lines={} keyword={}",
        short(title, 40),
        raw_text.len(),
        keyword_hit
    ));

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
            detected_by: "macOS Accessibility content observer (git commit)".to_string(),
        });
    }

    if !keyword_hit {
        return None;
    }
    parse_observed_approval_with_context(
        title,
        raw_text,
        "macOS Accessibility content observer",
        true,
    )
}

fn collect_text(
    element: &AxElement,
    depth: usize,
    output: &mut Vec<String>,
    keyword_hit: &mut bool,
) {
    if depth > MAX_TREE_DEPTH || output.len() >= MAX_TEXT_LINES {
        return;
    }
    let name = element_display_name(element);
    if !name.is_empty() {
        for line in push_clean(output, name) {
            if !*keyword_hit && looks_like_approval_keyword(&line) {
                *keyword_hit = true;
            }
        }
    }
    for child in element_children(element) {
        collect_text(&child, depth + 1, output, keyword_hit);
    }
}

fn push_clean(output: &mut Vec<String>, value: String) -> Vec<String> {
    let mut pushed = Vec::new();
    for line in value.split(|c: char| c == '\n' || c == '\r') {
        let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.len() >= 2 && !output.iter().any(|existing| existing == &normalized) {
            output.push(normalized.clone());
            pushed.push(normalized);
        }
    }
    pushed
}

// ----- クリック候補収集 -----

struct Candidate {
    element: AxElement,
    name: String,
    role: String,
    subrole: String,
}

impl Candidate {
    fn is_interactive_role(&self) -> bool {
        is_clickable_role(&self.role)
    }

    fn score(&self) -> i32 {
        let mut score = 0;
        if self.is_interactive_role() {
            score += 10;
        }
        // AXButton は最優先。
        if self.role == "AXButton" {
            score += 5;
        }
        // 行アイテム系（サイドバー会話）は次点。
        if matches!(self.role.as_str(), "AXRow" | "AXOutlineRow" | "AXCell") {
            score += 3;
        }
        // close-button subrole が立っているものは確度が高い。
        if self.subrole == "AXCloseButton" || self.subrole == "AXCancelButton" {
            score += 4;
        }
        score
    }
}

fn collect_candidates(root: &AxElement, matcher: fn(&str) -> bool) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut ancestor_stack = Vec::new();
    collect_candidates_recursive(root, 0, matcher, &mut ancestor_stack, &mut out);
    out
}

fn collect_candidates_recursive(
    element: &AxElement,
    depth: usize,
    matcher: fn(&str) -> bool,
    ancestor_stack: &mut Vec<AxElement>,
    out: &mut Vec<Candidate>,
) {
    if depth > MAX_CLICK_TREE_DEPTH || out.len() >= 30 {
        return;
    }
    let name = element_display_name(element);
    if matcher(&name) {
        let own_role = element_role(element);
        let target_element = if is_clickable_role(&own_role) {
            element.clone()
        } else {
            pick_clickable_ancestor(ancestor_stack).unwrap_or_else(|| element.clone())
        };
        let role = element_role(&target_element);
        let subrole = element_subrole(&target_element);
        out.push(Candidate {
            element: target_element,
            name,
            role,
            subrole,
        });
    }
    ancestor_stack.push(element.clone());
    for child in element_children(element) {
        collect_candidates_recursive(&child, depth + 1, matcher, ancestor_stack, out);
    }
    ancestor_stack.pop();
}

fn pick_best(candidates: &[Candidate]) -> Option<&Candidate> {
    candidates.iter().max_by_key(|c| c.score())
}

fn log_candidates(outcome: &mut ClickOutcome, label: &str, candidates: &[Candidate]) {
    outcome
        .notes
        .push(format!("{label} 候補: {} 件", candidates.len()));
    for (i, c) in candidates.iter().take(5).enumerate() {
        outcome.notes.push(format!(
            "  [{}] name=\"{}\" role={} subrole={} score={}",
            i,
            short(&c.name, 60),
            c.role,
            c.subrole,
            c.score()
        ));
    }
}

fn invoke_candidate(candidate: &Candidate) -> Result<(), String> {
    let press_action = CFString::new(kAXPressAction);
    let err = unsafe {
        AXUIElementPerformAction(
            candidate.element.as_raw(),
            press_action.as_concrete_TypeRef(),
        )
    };
    if err == 0 {
        return Ok(());
    }
    // AXRow / AXCell など Press を実装しない要素はサイドバーで多い。AXRaiseAction を後続で
    // 試して focus/activation を行う。
    let raise_action = CFString::new(kAXRaiseAction);
    let err2 = unsafe {
        AXUIElementPerformAction(
            candidate.element.as_raw(),
            raise_action.as_concrete_TypeRef(),
        )
    };
    if err2 == 0 {
        return Ok(());
    }
    Err(format!(
        "AXPress / AXRaise いずれも失敗しました (press_err={}, raise_err={})",
        err, err2
    ))
}

fn is_native_window_close_candidate(candidate: &Candidate) -> bool {
    let lower = candidate.name.to_lowercase();
    candidate.subrole == "AXCloseButton"
        || lower == "close button"
        || candidate.name == "閉じるボタン"
        || candidate.name == "关闭按钮"
}

// ----- サイドバー pending badge ハンドリング -----

fn collect_pending_approval_sidebar_targets(window: &AxElement) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut ancestor_stack: Vec<AxElement> = Vec::new();
    collect_pending_approval_recursive(window, 0, &mut ancestor_stack, &mut out);
    out
}

fn collect_pending_approval_recursive(
    element: &AxElement,
    depth: usize,
    ancestor_stack: &mut Vec<AxElement>,
    out: &mut Vec<Candidate>,
) {
    if depth > MAX_CLICK_TREE_DEPTH || out.len() >= 10 {
        return;
    }

    let name = element_display_name(element);
    if is_pending_approval_badge(&name) {
        let target_element = if is_clickable_role(&element_role(element)) {
            element.clone()
        } else {
            pick_clickable_ancestor(ancestor_stack).unwrap_or_else(|| element.clone())
        };
        let role = element_role(&target_element);
        let subrole = element_subrole(&target_element);
        let display = element_display_name(&target_element);
        out.push(Candidate {
            element: target_element,
            name: if display.is_empty() { name } else { display },
            role,
            subrole,
        });
    }

    ancestor_stack.push(element.clone());
    for child in element_children(element) {
        collect_pending_approval_recursive(&child, depth + 1, ancestor_stack, out);
    }
    ancestor_stack.pop();
}

fn pick_clickable_ancestor(stack: &[AxElement]) -> Option<AxElement> {
    for ancestor in stack.iter().rev() {
        let role = element_role(ancestor);
        if is_primary_clickable_role(&role) {
            return Some(ancestor.clone());
        }
    }
    for ancestor in stack.iter().rev() {
        let role = element_role(ancestor);
        if is_fallback_clickable_role(&role) {
            return Some(ancestor.clone());
        }
    }
    None
}

fn is_clickable_role(role: &str) -> bool {
    is_primary_clickable_role(role) || is_fallback_clickable_role(role)
}

fn is_primary_clickable_role(role: &str) -> bool {
    matches!(
        role,
        "AXButton"
            | "AXMenuItem"
            | "AXRadioButton"
            | "AXCheckBox"
            | "AXLink"
            | "AXCell"
            | "AXRow"
            | "AXOutlineRow"
            | "AXTab"
    )
}

fn is_fallback_clickable_role(role: &str) -> bool {
    matches!(role, "AXGroup" | "AXTabGroup")
}

/// `click_yes_inner` / `click_no_inner` のメイン操作成功後に追加でサイドバー pending 会話を
/// 1 件アクティブ化することで、アクティブ会話の承認ループに阻まれて他プロジェクトの承認待ちが
/// 永続的にスキップされる餓死を回避する。Windows 版と同じロジック。
fn try_activate_pending_sidebar_after_main(window: &AxElement, outcome: &mut ClickOutcome) {
    let sidebar_targets = collect_pending_approval_sidebar_targets(window);
    if sidebar_targets.is_empty() {
        return;
    }
    log_candidates(outcome, "後続「承認待ち」サイドバー", &sidebar_targets);
    let Some(target) = pick_best(&sidebar_targets) else {
        return;
    };
    match invoke_candidate(target) {
        Ok(()) => {
            outcome.notes.push(format!(
                "他会話の承認待ちサイドバーをアクティブ化（次サイクル用）: role=\"{}\" name=\"{}\"",
                target.role,
                short(&target.name, 60),
            ));
        }
        Err(error) => {
            outcome.notes.push(format!(
                "後続サイドバーアクティブ化に失敗（メイン操作は成功済）: name=\"{}\" error={}",
                short(&target.name, 60),
                error,
            ));
        }
    }
}

// ----- デバッグダンプ -----

fn dump_approval_tree(window: &AxElement) -> Vec<String> {
    let mut out = Vec::new();
    dump_approval_tree_recursive(window, 0, &mut out);
    out
}

fn dump_approval_tree_recursive(element: &AxElement, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_CLICK_TREE_DEPTH || out.len() >= MAX_DUMP_ENTRIES {
        return;
    }
    let name = element_display_name(element);
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    let role = element_role(element);
    let is_interactive_role = is_clickable_role(&role);
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
    let interactive_with_name = is_interactive_role && !trimmed.is_empty();
    if has_approval_keyword || interactive_with_name {
        out.push(format!(
            "  d={:02} name=\"{}\" role={}",
            depth,
            short(trimmed, 80),
            role,
        ));
    }
    for child in element_children(element) {
        dump_approval_tree_recursive(&child, depth + 1, out);
    }
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
