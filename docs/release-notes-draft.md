# 次回リリースノート草稿

このドキュメントは `v0.1.8` の Codex Approval Guard リリース用の草稿です。

実装言語ではなくリリースノート言語で書きます。`CHANGELOG.md` の `## Unreleased` を起点に、ユーザーが理解できる粒度に書き直してここに記録してください。

## Suggested Release Title

`Codex Approval Guard v0.1.8`

## Short Summary

v0.1.8 は macOS Accessibility backend を実装し、macOS でも Codex Desktop の承認ダイアログ検出、自動承認/拒否、git commit dialog の閉鎖、サイドバーの承認待ち会話アクティブ化を扱えるようにします。あわせて Windows / macOS で共通利用する文字列 matcher を整理し、複数会話の承認待ちが詰まる問題を改善します。

## Suggested GitHub Release Body

### Highlights

- macOS Accessibility API を使う本実装を追加しました。
- Codex Desktop プロセスを `NSWorkspace.runningApplications` から列挙し、AX ツリーを再帰走査して承認ダイアログとサイドバーの「承認が必要」バッジを検出します。
- macOS でも `AXPress` による「1. はい / 3. いいえ / 閉じる / 送信」操作と、`AXRaise` によるサイドバー会話のアクティブ化に対応しました。
- `CGEventSourceSecondsSinceLastEventType` で user idle を取得し、Windows 版と同じ 1500ms ガードを適用します。
- アクティブ会話が短い間隔で承認を生成し続ける場合でも、他プロジェクトのサイドバー承認待ちが処理されやすくなりました。
- プラットフォーム非依存の文字列 matcher を `platform/matchers.rs` に集約し、Windows / macOS の判定差分を減らしました。

### Why This Release Matters

これまで macOS backend は placeholder に近く、承認ダイアログの観測や自動操作は Windows 版が中心でした。

今回の更新で macOS Accessibility backend が実際に AX ツリーを読み、承認・拒否・閉鎖・サイドバー pending conversation のアクティブ化まで扱えるようになります。macOS で利用する場合は、システム設定のアクセシビリティ権限を Codex Approval Guard に許可する必要があります。

また、アクティブ会話が承認を連続生成するケースでは、他会話の pending badge が後回しになり続けることがありました。v0.1.8 ではメイン操作成功後にサイドバーの pending 会話を 1 件アクティブ化し、次回ポーリングで処理されるようにしています。

### User-Facing Improvements

#### macOS 対応

- macOS Accessibility API による承認ダイアログ観測を追加。
- macOS での approve / deny / dismiss 操作に対応。
- macOS での user idle 判定に対応。
- アクセシビリティ権限が未許可の場合は snapshot details に理由を表示。

#### 自動承認 / 拒否 / 閉鎖

- アクティブ会話の承認処理後、サイドバーの別 pending 会話を次回処理しやすいようにアクティブ化。
- macOS で非文字列の AX 属性値を誤って CFString として扱わないように改善。
- macOS で可視ラベルが `AXStaticText` 子要素に分離されているボタンも、クリック可能な祖先要素へ引き上げて操作。
- macOS の git commit dialog 閉鎖で、通常の Codex メインウィンドウを誤って閉じるリスクを低減。

#### 保守性

- Windows / macOS 共通の文字列 matcher を `platform/matchers.rs` に集約。
- `is_first_yes_option`、`is_first_no_option`、`is_recommended_option`、`is_submit_button`、`is_close_or_cancel_button`、`looks_like_approval_keyword` を両 backend から共有。

### Suggested "Upgrade Notes" Section

macOS で利用する場合は、システム設定 → プライバシーとセキュリティ → アクセシビリティ で Codex Approval Guard を許可してください。

Windows では既存設定の移行は不要です。通常どおり新しいアプリ bundle をインストールしてください。

### Suggested "Who Should Update" Section

このリリースは特に次のユーザーに有用です:

- macOS で Codex Approval Guard を使いたい
- 複数プロジェクト / 複数会話で承認待ちが同時に発生する
- アクティブ会話の承認が多く、別会話の pending badge が処理されにくい
- Windows / macOS の承認ラベル判定を揃えたい
