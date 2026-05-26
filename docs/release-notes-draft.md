# 次回リリースノート草稿

このドキュメントは `v0.1.7` の Codex Approval Guard リリース用の草稿です。

実装言語ではなくリリースノート言語で書きます。`CHANGELOG.md` の `## Unreleased` を起点に、ユーザーが理解できる粒度に書き直してここに記録してください。

## Suggested Release Title

`Codex Approval Guard v0.1.7`

## Short Summary

v0.1.7 は Codex Desktop の多言語プロンプト検出を改善し、日本語 UI の推奨選択肢、全角番号、丸数字、plan confirmation、ショートカット付き送信ボタンをより安定して扱えるようにします。

## Suggested GitHub Release Body

### Highlights

- ask_user_question 系プロンプトの推奨選択肢 marker として「（おすすめ）」および全角の「（recommended）」を検出できるようにしました。
- 日本語・中国語・英語の plan confirmation prompt（例: 「このプランを実施しますか」「是否执行此方案」「execute this plan」）を検出できるようにしました。
- 承認・拒否・推奨選択肢の番号判定を、半角数字だけでなく全角数字と丸数字にも対応しました。
- 送信ボタン判定を、ショートカット表記や補助文が続くラベルでも拾いやすくしました。
- 日本語 UI の推奨選択肢や全角番号付き選択肢を自動操作候補として拾えない場合がある問題を修正しました。
- `npm run ...` の後ろに自然言語の確認メモが続く場合に、メモ部分まで command として扱う可能性を修正しました。

### Why This Release Matters

Codex Desktop の承認プロンプトは、表示言語や UIA のアクセシビリティ表現によって番号やラベルの形が変わることがあります。

今回の更新では、全角数字や丸数字を含む番号付き選択肢、追加の日本語推奨 marker、plan confirmation prompt、ショートカット付きの送信ボタンを扱えるようにし、多言語 UI での検出漏れを減らしました。

また、plan confirmation や apply changes prompt ではチャット履歴中の shell command を誤って実行対象として拾わないようにし、`npm run ...` の後ろに続く自然言語メモも command から切り離します。

### User-Facing Improvements

#### 自動承認 / 拒否 / 閉鎖

- 「（おすすめ）」や全角の「（recommended）」を推奨選択肢 marker として認識。
- `１. ...` や `① ...` のような番号付き選択肢を承認・拒否・推奨候補として認識。
- 「このプランを実施しますか」「是否执行此方案」「execute this plan」形式の plan confirmation を認識。
- ショートカットや補助文が続く送信ボタンを認識しやすく改善。

#### 安定性

- 日本語 UI やアクセシビリティ表現差分による自動操作候補の検出漏れを低減。
- チャット履歴中の shell command や自然言語メモを command として誤抽出するリスクを低減。
- 番号プレフィックス、plan confirmation、自然言語メモ付き command の判定をテストで補強。

### Suggested "Upgrade Notes" Section

既存設定の移行は不要です。通常どおり新しいアプリ bundle をインストールしてください。

### Suggested "Who Should Update" Section

このリリースは特に次のユーザーに有用です:

- 日本語 UI の Codex Desktop で承認プロンプトを扱っている
- 推奨選択肢付きの Codex プロンプトを自動承認したい
- plan confirmation prompt を扱う場面が多い
- 全角番号や丸数字が表示される環境で自動操作の検出漏れを減らしたい
