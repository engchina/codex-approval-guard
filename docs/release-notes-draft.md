# 次回リリースノート草稿

このドキュメントは `v0.1.5` の Codex Approval Guard リリース用の草稿です。

実装言語ではなくリリースノート言語で書きます。`CHANGELOG.md` の `## Unreleased` を起点に、ユーザーが理解できる粒度に書き直してここに記録してください。

## Suggested Release Title

`Codex Approval Guard v0.1.5`

## Short Summary

v0.1.5 は WebView 内蔵の git commit dialog を Escape 経路で閉じられるようにし、`Skip` 系ボタンの誤クリックリスクを下げます。

## Suggested GitHub Release Body

### Highlights

- WebView 内蔵の git commit dialog を `VK_ESCAPE` で閉じる高速経路を追加しました。
- Escape 経路では Codex の親 HWND と子 HWND にキーイベントを送信し、WebView2 内の modal に届きやすくしました。
- `跳过` / `Skip` / `スキップ` を閉鎖候補から除外し、Codex メインウィンドウ内の無関係なボタンを誤クリックしにくくしました。

### Why This Release Matters

git commit dialog は独立 HWND として開く場合だけでなく、Codex の WebView 内蔵 modal として表示される場合があります。独立 HWND では `WM_CLOSE` が有効ですが、WebView 内蔵 modal では UIA 上の明確な「閉じる」ボタンが見つからないことがあります。

今回の更新では、WebView 内蔵 modal に対して Escape キー経路を先に試し、無関係な `Skip` 系ボタンを close/cancel matcher から外すことで、速さと安全性の両方を改善しました。

### User-Facing Improvements

#### 自動承認 / 拒否 / 閉鎖

- WebView 内蔵 modal の git commit dialog に Escape キー経路を利用。
- `Skip` 系ボタンを close/cancel matcher から除外。

#### プラットフォーム対応

- Codex 親 HWND と最大 64 件の子 HWND に Escape keydown / keyup を送信。
- Escape 経路が失敗した場合は既存の UIA 経路へフォールバック。

### Suggested "Upgrade Notes" Section

既存設定の移行は不要です。通常どおり新しいアプリ bundle をインストールしてください。

### Suggested "Who Should Update" Section

このリリースは特に次のユーザーに有用です:

- Codex Desktop で頻繁に承認ダイアログを操作している
- WebView 内蔵の git commit dialog を自動閉鎖したい
- `Skip` 系ボタンの誤クリックを避けたい
