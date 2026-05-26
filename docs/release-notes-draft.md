# 次回リリースノート草稿

このドキュメントは `v0.1.6` の Codex Approval Guard リリース用の草稿です。

実装言語ではなくリリースノート言語で書きます。`CHANGELOG.md` の `## Unreleased` を起点に、ユーザーが理解できる粒度に書き直してここに記録してください。

## Suggested Release Title

`Codex Approval Guard v0.1.6`

## Short Summary

v0.1.6 は自動操作結果に成功経路を表す `method` ラベルを保持できるようにし、ask_user_question 系プロンプトの推奨選択肢と WebView 内蔵 dialog の閉鎖経路を扱いやすくします。

## Suggested GitHub Release Body

### Highlights

- 自動操作結果の `ClickOutcome` に `method` ラベルを追加しました。
- `method` には `wm-close`、`escape-multi[attach-focus+sendinput+broadcast]`、`uia-recommended-option` などの短い経路名を保持できるようにしています。
- 番号付きの「（推荐）/（推奨）/(Recommended)」選択肢を自動承認候補として扱えるようにしました。
- 日本語版 Codex Desktop の「これらの変更を行いますか?」形式の変更適用プロンプトを検出できるようにしました。
- WebView 内蔵 dialog の Escape 閉鎖を `AttachThreadInput`、`SendInput`、HWND broadcast の複数経路で試すように改善しました。
- serde default を付け、既存の結果 payload との互換性を維持しています。

### Why This Release Matters

自動操作の安定性を追うには、結果として成功したかだけでなく、どの経路で成功したかを短く持てることが重要です。

今回の更新では `ClickOutcome` に `method` を追加し、今後の監査ログや UI 表示で自動操作経路を扱いやすくしました。

また、Codex の確認 UI で推奨選択肢が明示されるケースと、WebView 内蔵 dialog が単純な `PostMessage` を取りこぼすケースに対して、より実運用に近い自動操作経路を追加しています。

日本語版 Codex Desktop の変更適用プロンプトにも対応し、言語差分で検出漏れが起きにくくなりました。

### User-Facing Improvements

#### 自動承認 / 拒否 / 閉鎖

- 自動操作結果に成功経路ラベルを保持可能に変更。
- ask_user_question 系プロンプトの推奨選択肢を自動承認候補として認識。
- 日本語版 Codex Desktop の変更適用プロンプトを認識。
- 「继续 / Continue / 継続」ボタンを送信ボタンとして認識。
- 既存 payload と互換性を保つため `method` は省略可能。

#### プラットフォーム対応

- Windows / macOS adapter で共有する `ClickOutcome` 型を拡張。
- 診断用の詳細 notes とは別に、短い method label を扱えるように整理。
- Windows では Escape 閉鎖を `AttachThreadInput`、`SendInput`、HWND broadcast の複数経路で試行。

### Suggested "Upgrade Notes" Section

既存設定の移行は不要です。通常どおり新しいアプリ bundle をインストールしてください。

### Suggested "Who Should Update" Section

このリリースは特に次のユーザーに有用です:

- Codex Desktop で頻繁に承認ダイアログを操作している
- 日本語 UI の Codex Desktop で変更適用プロンプトを扱っている
- 推奨選択肢付きの Codex プロンプトを自動承認したい
- 自動操作の成功経路を監査・診断で追いたい
- 今後の method 表示・記録拡張に備えたい
