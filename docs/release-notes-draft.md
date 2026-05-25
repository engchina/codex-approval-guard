# 次回リリースノート草稿

このドキュメントは `v0.1.4` の Codex Approval Guard リリース用の草稿です。

実装言語ではなくリリースノート言語で書きます。`CHANGELOG.md` の `## Unreleased` を起点に、ユーザーが理解できる粒度に書き直してここに記録してください。

## Suggested Release Title

`Codex Approval Guard v0.1.4`

## Short Summary

v0.1.4 は git commit dialog の検出と自動閉鎖を高速化し、自動操作の監査 trace とクリック候補探索の安定性を改善します。

## Suggested GitHub Release Body

### Highlights

- git commit dialog をタイトルだけで即時検出し、独立 HWND の場合は `WM_CLOSE` で高速に閉じます。
- 自動操作後の監査ログに、対象ウィンドウとクリック経路の trace notes を記録します。
- 承認リクエストのポーリング間隔と自動操作 cooldown を短縮し、応答までの待ち時間を減らしました。
- `关闭/取消` 候補探索時に control type を絞り、WebView2 の巨大 UI ツリーでも不要な pattern 取得を減らします。

### Why This Release Matters

git commit dialog は通常の承認 UI と違い、独立した dialog window として表示されることがあります。今回の更新ではタイトルだけでその dialog を検出し、可能な場合は UI ツリー全体の再走査を待たずに `WM_CLOSE` で閉じます。

また、自動操作の監査ログに trace notes を残すことで、どのウィンドウに対してどの経路で操作したかを後から追いやすくしました。

### User-Facing Improvements

#### 自動承認 / 拒否 / 閉鎖

- git commit dialog のタイトル一致時に高速閉鎖経路を利用。
- 自動操作の対象ウィンドウとクリック trace を監査ログに記録。
- バックグラウンドポーリング間隔と自動操作 cooldown を短縮。

#### プラットフォーム対応

- Windows UI Automation のクリック候補探索で control type filter を利用。
- git commit dialog 判定済みリクエストではクリック時の再走査を抑制。

### Suggested "Upgrade Notes" Section

既存設定の移行は不要です。通常どおり新しいアプリ bundle をインストールしてください。

### Suggested "Who Should Update" Section

このリリースは特に次のユーザーに有用です:

- Codex Desktop で頻繁に承認ダイアログを操作している
- git commit dialog の自動閉鎖を速くしたい
- 監査ログで自動操作の経路を確認したい
