# 次回リリースノート草稿

このドキュメントは `v0.1.3` の Codex Approval Guard リリース用の草稿です。

実装言語ではなくリリースノート言語で書きます。`CHANGELOG.md` の `## Unreleased` を起点に、ユーザーが理解できる粒度に書き直してここに記録してください。

## Suggested Release Title

`Codex Approval Guard v0.1.3`

## Short Summary

v0.1.3 は自動拒否の実行経路、直近自動操作の表示、Windows でのクリック安定性、AArch64 環境の更新アセット選択を改善します。

## Suggested GitHub Release Body

### Highlights

- `deny` 判定時に Codex の「3. 否」相当の選択肢を自動選択し、「提交」まで進められるようにしました。
- ステータスカードに直近の自動承認・自動拒否・自動閉鎖を一時表示し、対象操作と適用ルールをすぐ確認できます。
- Windows UI Automation の通常 invoke が使えない場合に、バックグラウンドクリックを追加の手段として試行します。
- Windows AArch64 環境で arm64 アセットがない場合でも、x64 / x86 / 汎用アセットへフォールバックして更新候補を選べるようにしました。

### Why This Release Matters

承認だけでなく拒否も自動化対象になると、危険または許可しない操作を同じルール体系で処理できます。今回の更新では `deny` 判定を実際の UI 操作に接続し、直近の自動操作を画面上で確認できるようにしました。

また、Windows の UIA invoke が使えないケースにバックグラウンドクリックで対応し、更新確認では AArch64 環境でも実用的な配布アセットを選びやすくしています。

### User-Facing Improvements

#### 自動承認 / 拒否 / 閉鎖

- `deny` 判定で「3. 否」相当の選択肢を自動選択。
- 自動操作後、ステータスカードへ直近操作の対象、判定、適用ルールを一時表示。
- ダークモード時の主要 UI 配色を調整。

#### プラットフォーム対応

- Windows UI Automation の通常 invoke が使えない要素に対して、バックグラウンドクリックを試行。
- Windows AArch64 環境で x64 / x86 / 汎用アセットを更新候補として選択可能に改善。

### Suggested "Upgrade Notes" Section

既存設定の移行は不要です。通常どおり新しいアプリ bundle をインストールしてください。

### Suggested "Who Should Update" Section

このリリースは特に次のユーザーに有用です:

- Codex Desktop で頻繁に承認ダイアログを操作している
- 拒否すべき操作をルールで自動処理したい
- Windows AArch64 環境で更新確認を使いたい
