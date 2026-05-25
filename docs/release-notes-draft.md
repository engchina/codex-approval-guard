# 次回リリースノート草稿

このドキュメントは `v0.1.1` の Codex Approval Guard リリース用の草稿です。

実装言語ではなくリリースノート言語で書きます。`CHANGELOG.md` の `## Unreleased` を起点に、ユーザーが理解できる粒度に書き直してここに記録してください。

## Suggested Release Title

`Codex Approval Guard v0.1.1`

## Short Summary

v0.1.1 は履歴 UI と bundle icon 設定を整え、監査ログへの到達性とリリース成果物の見え方を改善します。

## Suggested GitHub Release Body

### Highlights

- 監査ログファイルのパスを UI から直接コピーできます。
- 直近履歴に判定種別ごとの左罫線を追加し、自動承認・閉鎖・確認待ちを見分けやすくしました。
- 履歴時刻をローカル時刻として表示し、確認しやすくしました。
- Windows / macOS bundle 用 icon 参照を複数サイズの PNG / ICNS / ICO 構成に更新しました。

### Why This Release Matters

監査ログは自動操作の説明責任に直結するため、利用者がログパスをすぐ共有・確認できることが重要です。今回の更新では、履歴画面からログパスをコピーできるようにし、履歴一覧も判定種別ごとの視認性を上げました。

また、bundle icon 設定を整理することで、GitHub Actions で生成される Windows / macOS 成果物が各プラットフォームで適切な icon asset を参照できるようにしました。

### User-Facing Improvements

#### 自動承認 / 閉鎖

- 履歴エリアに監査ログパスのコピー操作を追加。
- 履歴項目に判定種別ごとの左罫線を追加。
- 履歴時刻の表示をローカル時刻ベースに改善。

#### プラットフォーム対応

- Tauri bundle icon 設定を複数サイズ PNG、ICNS、ICO の構成に更新。

### Suggested "Upgrade Notes" Section

既存設定の移行は不要です。通常どおり新しいアプリ bundle をインストールしてください。

### Suggested "Who Should Update" Section

このリリースは特に次のユーザーに有用です:

- Codex Desktop で頻繁に承認ダイアログを操作している
- git 関連ダイアログをガード対象にしたい
- 監査ログを使って自動操作の履歴を確認したい
