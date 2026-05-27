# 次回リリースノート草稿

このドキュメントは `v0.1.10` の Codex Approval Guard リリース用の草稿です。

実装言語ではなくリリースノート言語で書きます。`CHANGELOG.md` の `## 0.1.10 - 2026-05-28` を起点に、ユーザーが理解できる粒度に書き直してここに記録してください。

## Suggested Release Title

`Codex Approval Guard v0.1.10`

## Short Summary

v0.1.10 は、Windows でサイドバーの pending 会話をアクティブ化する際に、Codex Desktop の「チャット名を変更 / Rename chat」ダイアログが意図せず開く問題を修正します。サイドバー会話の操作では `Invoke` より `Select` を優先し、万一 rename dialog が開いた場合も承認候補として扱わず Escape で閉じます。あわせて、v0.1.9 の回路ブレーカーは一度外し、今回判明した副作用へ直接対処する形に戻します。

## Suggested GitHub Release Body

### Highlights

- Windows UIA backend で、サイドバー pending 会話を選択する際に `SelectionItemPattern.select` を優先するようにしました。
- `InvokePattern.invoke` が Codex Desktop 側で rename 操作に割り当てられている場合でも、「チャット名を変更」ダイアログを開きにくくなります。
- 「チャット名を変更 / Rename chat / 重命名聊天」ダイアログを検出した場合は承認候補として扱わず、Escape broadcast で自動閉鎖します。
- v0.1.9 の auto-approve burst / UIA observe failure 回路ブレーカーはいったん削除しました。今回の問題は自動停止ではなく、rename dialog 副作用の直接抑止で対応します。

### Why This Release Matters

v0.1.8 以降、アクティブ会話の承認処理後に、サイドバーの別 pending 会話をアクティブ化する経路が入りました。Windows UI Automation ではサイドバー会話の ListItem に `Invoke` を投げられますが、Codex Desktop 側ではこの既定動作が「会話名を変更」に割り当てられている場合があります。

その結果、Guard が pending 会話を選ぼうとしただけで rename dialog を開き、次の観測が承認ダイアログではなく rename dialog に当たって詰まる可能性がありました。

v0.1.10 では、サイドバー会話のアクティブ化では `Select` を先に使い、rename dialog が見えた場合は Escape で閉じます。これにより、複数会話の承認待ちを処理する経路を維持しつつ、意図しない rename dialog の副作用を抑えます。

### User-Facing Improvements

#### Windows pending conversation

- サイドバーの「承認が必要」会話を選択する時に `SelectionItemPattern.select` を優先。
- `Invoke` は `Select` が使えない場合の fallback に限定。
- rename dialog が開いた場合は自動的に Escape を送信。

#### 誤認防止

- rename dialog は承認候補として扱わない。
- 日本語、中国語、英語の rename dialog 表現を判定対象に追加。
- rename dialog を検出した場合は diagnostics に記録。

#### 挙動変更

- v0.1.9 の回路ブレーカーは削除。
- 自動 paused への切り替えではなく、rename dialog の副作用を直接抑止する実装に変更。

### Suggested "Upgrade Notes" Section

既存設定の移行操作は不要です。

v0.1.9 で追加された `circuit_breaker:*` による自動 paused は、このリリースでは作動しません。長時間運用時の異常停止ガードよりも、今回判明した rename dialog 副作用の直接修正を優先しています。

### Suggested "Who Should Update" Section

このリリースは特に次のユーザーに有用です:

- Windows で複数会話 / 複数プロジェクトの承認待ちを扱う
- Codex Desktop の「チャット名を変更」ダイアログが意図せず開く
- pending conversation の自動アクティブ化が rename dialog で詰まる
- v0.1.9 の自動 paused より、rename dialog の直接修正を優先したい
