//! プラットフォーム非依存の文字列マッチャー集。
//!
//! Codex Desktop の承認ダイアログに含まれる「1. はい」「3. いいえ」「提交」「閉じる」等の
//! UI ラベルや、サイドバーの「承認が必要」バッジ、本文中の承認キーワードを判定する
//! pure な関数群。Windows (UI Automation) / macOS (Accessibility) 両プラットフォームから
//! 同一ロジックで参照する。

use super::parser::is_pending_approval_badge;

pub fn starts_with_one_prefix(s: &str) -> bool {
    let trimmed = s.trim();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first == '①' {
        return true;
    }
    if first == '1' || first == '１' {
        let Some(second) = chars.next() else {
            return true; // Just "1"
        };
        if !second.is_ascii_digit() && !('０'..='９').contains(&second) {
            return true;
        }
    }
    false
}

pub fn starts_with_three_prefix(s: &str) -> bool {
    let trimmed = s.trim();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first == '③' {
        return true;
    }
    if first == '3' || first == '３' {
        let Some(second) = chars.next() else {
            return true; // Just "3"
        };
        if !second.is_ascii_digit() && !('０'..='９').contains(&second) {
            return true;
        }
    }
    false
}

pub fn starts_with_recommended_prefix(s: &str) -> bool {
    let trimmed = s.trim();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if ('①'..='⑨').contains(&first) {
        return true;
    }
    if (first.is_ascii_digit() && first != '0') || ('１'..='９').contains(&first) {
        let Some(second) = chars.next() else {
            return true; // Just a single digit
        };
        if !second.is_ascii_digit() && !('０'..='９').contains(&second) {
            return true;
        }
    }
    false
}

pub fn is_first_yes_option(name: &str) -> bool {
    let trimmed = name.trim();
    if is_standalone_primary_approval_label(trimmed) {
        return true;
    }
    starts_with_one_prefix(trimmed) && looks_like_primary_approval_option(trimmed)
}

pub fn is_standalone_primary_approval_label(label: &str) -> bool {
    let lower = label.to_lowercase();
    matches!(label, "是" | "承認" | "批准" | "はい")
        || lower == "yes"
        || lower == "approve"
        || lower == "allow"
}

pub fn looks_like_primary_approval_option(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.contains("是")
        || name.contains("承認")
        || name.contains("批准")
        || name.contains("確認")
        || name.contains("はい")
        || contains_ascii_word(&lower, "approve")
        || contains_ascii_word(&lower, "yes")
        || contains_ascii_word(&lower, "allow")
}

/// Codex の ask_user_question 系プロンプトでは「（推荐）」「（推奨）」「(Recommended)」
/// マーカー付きの選択肢が出る（番号は 1〜N のどれでも可）。Guard はそのマーカー付きの
/// 番号付き選択肢を最優先で選んでクリックする。
///
/// 偶発的なマッチ（例: 否系選択肢の説明文に「推荐」が含まれる）を避けるため、
/// マーカーは必ずカッコで囲まれていることを要件とする。さらに 1〜9（全角／半角／丸数字）の
/// 番号プレフィックスを必須とする。
pub fn is_recommended_option(name: &str) -> bool {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    let has_marker = trimmed.contains("（推荐）")
        || trimmed.contains("(推荐)")
        || trimmed.contains("（推奨）")
        || trimmed.contains("(推奨)")
        || trimmed.contains("（おすすめ）")
        || trimmed.contains("(おすすめ)")
        || lower.contains("(recommended)")
        || lower.contains("（recommended）");
    if !has_marker {
        return false;
    }
    starts_with_recommended_prefix(trimmed)
}

/// click 系で使う組み合わせ matcher。「1. 是」相当 もしくは「番号付き (推荐) 選択肢」
/// のいずれかにマッチ。両者が同時に存在する Codex プロンプトは観測されないため、候補が
/// 一意に絞られる前提。
pub fn is_first_yes_or_recommended_option(name: &str) -> bool {
    is_first_yes_option(name) || is_recommended_option(name)
}

pub fn is_first_no_option(name: &str) -> bool {
    let trimmed = name.trim();
    if is_standalone_primary_rejection_label(trimmed) {
        return true;
    }
    starts_with_three_prefix(trimmed) && looks_like_primary_rejection_option(trimmed)
}

pub fn is_standalone_primary_rejection_label(label: &str) -> bool {
    let lower = label.to_lowercase();
    matches!(label, "否" | "拒否" | "拒绝" | "いいえ")
        || lower == "no"
        || lower == "deny"
        || lower == "decline"
        || lower == "reject"
}

pub fn looks_like_primary_rejection_option(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.contains("否")
        || name.contains("拒否")
        || name.contains("拒绝")
        || name.contains("いいえ")
        || contains_ascii_word(&lower, "deny")
        || contains_ascii_word(&lower, "no")
        || contains_ascii_word(&lower, "decline")
        || contains_ascii_word(&lower, "reject")
}

/// `haystack` のうち、`needle` が単語境界（英数字とアンダースコア以外）で
/// 囲まれた位置に現れるかを判定する。`contains("no")` のような部分一致が
/// "now" / "note" / "know" / "another" などを誤って引っ掛けるのを防ぐ。
/// `needle` は ASCII 小文字前提で呼び出すこと（呼び出し側で `to_lowercase` 済み）。
fn contains_ascii_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    let mut idx = 0;
    while let Some(pos) = haystack[idx..].find(needle) {
        let start = idx + pos;
        let end = start + needle_bytes.len();
        let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_word_byte(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        // 1 byte 進めて次のマッチを探す。needle は ASCII 想定なので安全。
        idx = start + 1;
    }
    false
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub fn is_submit_button(name: &str) -> bool {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    trimmed.starts_with("提交")
        || trimmed.starts_with("送信")
        || trimmed.starts_with("確認")
        || trimmed.starts_with("继续")
        || trimmed.starts_with("続行")
        || trimmed.starts_with("継続")
        || lower.starts_with("submit")
        || lower.starts_with("continue")
        || lower == "ok"
        || lower.starts_with("ok ")
        || lower.starts_with("ok\t")
        || lower.starts_with("ok\n")
        || lower.starts_with("ok\r")
}

pub fn is_close_or_cancel_button(name: &str) -> bool {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    // 注意: "跳过 / Skip / スキップ" は Codex メインウィンドウのサイドバー等にも存在し、
    // commit dialog の関閉対象とは無関係なケースが観測されたため、ここから除外する。
    // close icon と「关闭/閉じる/取消/Cancel」系の明示的なラベルのみを許可する。
    trimmed == "关闭"
        || trimmed == "閉じる"
        || trimmed == "取消"
        || trimmed == "キャンセル"
        || trimmed == "X"
        || trimmed == "x"
        || trimmed == "✕"
        || trimmed == "✖"
        || trimmed == "×"
        || trimmed == "⨯"
        || trimmed == "閉じるボタン"
        || trimmed == "キャンセルボタン"
        || trimmed == "关闭按钮"
        || lower == "close"
        || lower == "cancel"
        || lower == "dismiss"
        || lower == "close button"
        || lower == "cancel button"
        || lower == "close dialog"
        || lower == "close modal"
        || lower == "关闭对话框"
}

pub fn looks_like_approval_keyword(line: &str) -> bool {
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
        // Codex の ask_user_question 系プロンプトでは「（推荐）/（推奨）/(Recommended)」
        // マーカー付きの選択肢が出る。これは承認候補と見なせるため keyword_hit のトリガに含める。
        || line.contains("（推荐）")
        || line.contains("(推荐)")
        || line.contains("（推奨）")
        || line.contains("(推奨)")
        || lower.contains("(recommended)")
        || line.contains("プラン")
        || line.contains("実施しますか")
        || line.contains("実行しますか")
        || line.contains("方案")
        || line.contains("是否执行")
        || lower.contains("plan")
        // サイドバーのバッジ（非アクティブ会話での承認待ち）も拾うため、
        // バッジ文字列単体を keyword_hit のトリガに含める。判定本体は
        // parser::is_pending_approval_badge と整合させる。
        || is_pending_approval_badge(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_numbered_and_accessibility_stripped_yes_option() {
        assert!(is_first_yes_option("1. 是"));
        assert!(is_first_yes_option("1。是"));
        assert!(is_first_yes_option("１. 是"));
        assert!(is_first_yes_option("① 是"));
        assert!(is_first_yes_option("是"));
        assert!(is_first_yes_option("Yes"));
        assert!(is_first_yes_option("Approve"));
        assert!(is_first_yes_option("1。 はい"));
        assert!(is_first_yes_option("1. はい"));
        assert!(is_first_yes_option("はい"));
    }

    #[test]
    fn matches_numbered_and_accessibility_stripped_no_option() {
        assert!(is_first_no_option("3. 否"));
        assert!(is_first_no_option("3。否"));
        assert!(is_first_no_option("３. 否"));
        assert!(is_first_no_option("③ 否"));
        assert!(is_first_no_option("否"));
        assert!(is_first_no_option("No"));
        assert!(is_first_no_option("Decline"));
        assert!(is_first_no_option("3. 否，请告知 Codex 如何調整"));
        assert!(is_first_no_option(
            "3。 いいえ、Codex に何をすべきかを別の方法で指示してください"
        ));
        assert!(is_first_no_option("3. いいえ"));
        assert!(is_first_no_option("いいえ"));
    }

    #[test]
    fn does_not_match_remembered_approval_or_denial_options() {
        assert!(!is_first_yes_option("是，且本次会话不再询问"));
        assert!(!is_first_yes_option("2. 是，且本次会话不再询问"));
        assert!(!is_first_yes_option("3. 否，请告知 Codex 如何调整"));
        assert!(!is_first_yes_option("提交"));
    }

    #[test]
    fn rejection_matcher_does_not_match_no_as_substring() {
        // 旧実装は contains("no") で "now" / "note" / "another" にも反応していた。
        // 新実装では単語境界を要求するため、これらの誤マッチは起こらない。
        assert!(!looks_like_primary_rejection_option(
            "3. Now switch to plan B"
        ));
        assert!(!looks_like_primary_rejection_option("3. Note this option"));
        assert!(!looks_like_primary_rejection_option("3. Another option"));
        // 本来の no/deny/reject はもちろん拾える。
        assert!(looks_like_primary_rejection_option("3. No, do not run"));
        assert!(looks_like_primary_rejection_option("3. Deny the request"));
        assert!(looks_like_primary_rejection_option("3. Reject and stop"));
    }

    #[test]
    fn approval_matcher_does_not_match_yes_as_substring() {
        // "eyes" / "keystone" / "yesterday" などへの誤マッチを防ぐ。
        assert!(!looks_like_primary_approval_option("1. Eyes on it"));
        assert!(!looks_like_primary_approval_option("1. Yesterday plan"));
        // 本来の yes/approve/allow は引き続き拾う。
        assert!(looks_like_primary_approval_option("1. Yes, proceed"));
        assert!(looks_like_primary_approval_option("1. Approve and run"));
        assert!(looks_like_primary_approval_option("1. Allow access"));
    }

    #[test]
    fn matches_close_or_cancel_button_variants() {
        for name in [
            "关闭",
            "閉じる",
            "取消",
            "キャンセル",
            "X",
            "x",
            "✕",
            "✖",
            "×",
            "⨯",
            "Close",
            "close",
            "Cancel",
            "Dismiss",
            "Close dialog",
            "Close Modal",
            "关闭对话框",
            "Close button",
            "Cancel button",
            "閉じるボタン",
            "キャンセルボタン",
            "关闭按钮",
        ] {
            assert!(is_close_or_cancel_button(name), "should match `{name}`");
        }
    }

    #[test]
    fn does_not_match_skip_or_unrelated_buttons_as_close() {
        // 「跳过 / Skip / スキップ」は Codex メインウィンドウの別箇所にも存在し、
        // commit dialog の関閉対象とは無関係なため close マッチャから除外する。
        for name in [
            "提交",
            "继续",
            "Submit",
            "Continue",
            "确认",
            "1. 是",
            "跳过",
            "スキップ",
            "Skip",
            "skip",
        ] {
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
        // ask_user_question 系の「继续 / Continue / 続行 / 継続」も submit と見なす。
        // 日本語版 Codex Desktop の実際の送信ボタン文字列は「続行」(zokkou)。
        assert!(is_submit_button("继续"));
        assert!(is_submit_button("继续 ⏎"));
        assert!(is_submit_button("Continue"));
        assert!(is_submit_button("続行"));
        assert!(is_submit_button("続行 ⏎"));
        assert!(is_submit_button("継続"));
        // 日本語版 apply-changes ダイアログでは「送信する / 確認する」の表記が出る
        // (screenshot 確認済み)。末尾の Enter マーカー付きも含めて submit と扱う。
        assert!(is_submit_button("送信する"));
        assert!(is_submit_button("送信する ⏎"));
        assert!(is_submit_button("確認する"));
        assert!(is_submit_button("確認する ⏎"));
    }

    #[test]
    fn matches_recommended_option_with_parens_marker() {
        // 番号付き + カッコ内マーカーの組み合わせは推奨選択肢として扱う。
        assert!(is_recommended_option("1. 先改 AWS RAG（推荐）"));
        assert!(is_recommended_option("2. オプション（推奨）"));
        assert!(is_recommended_option("3. オプション（おすすめ）"));
        assert!(is_recommended_option("1. Option A (Recommended)"));
        assert!(is_recommended_option("4. choice (recommended)"));
        assert!(is_recommended_option("１。先改 AWS RAG (推荐)"));
        assert!(is_recommended_option("① 先改 AWS RAG (推荐)"));
    }

    #[test]
    fn does_not_match_unmarked_or_freestyle_recommended_text() {
        // 番号プレフィックスなし → false（dialog 本文に偶発的に出る「推荐」を拾わない）。
        assert!(!is_recommended_option("（推荐）先改 AWS RAG"));
        assert!(!is_recommended_option("推荐: 先改 AWS RAG"));
        // カッコなし → false（rejection 選択肢の説明文に「推荐」を含むケース等を弾く）。
        assert!(!is_recommended_option("3. 否，请告知 Codex 推荐什么"));
        // マーカーなし → false。
        assert!(!is_recommended_option("1. 是"));
        assert!(!is_recommended_option("2. 全仓库本地化"));
    }

    #[test]
    fn combined_yes_or_recommended_matcher_covers_both_cases() {
        // 既存の「1. 是」も推奨選択肢も同じ matcher で拾えるようにする。
        assert!(is_first_yes_or_recommended_option("1. 是"));
        assert!(is_first_yes_or_recommended_option(
            "1. 先改 AWS RAG（推荐）"
        ));
        assert!(is_first_yes_or_recommended_option("2. オプション（推奨）"));
        assert!(is_first_yes_or_recommended_option(
            "１。先改 AWS RAG (推荐)"
        ));
        assert!(is_first_yes_or_recommended_option("① 先改 AWS RAG (推荐)"));
        // 否定系・無関係は引っ掛けない。
        assert!(!is_first_yes_or_recommended_option("3. 否"));
        assert!(!is_first_yes_or_recommended_option("2. 全仓库本地化"));
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
        assert!(looks_like_approval_keyword("承認が必要"));
        // 無関係なサイドバーラベルは引っ掛けない。
        assert!(!looks_like_approval_keyword("启动项目"));
        assert!(!looks_like_approval_keyword("main"));
    }
}
