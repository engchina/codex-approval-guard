# 次回リリースノート草稿

このドキュメントは `v0.1.9` の Codex Approval Guard リリース用の草稿です。

実装言語ではなくリリースノート言語で書きます。`CHANGELOG.md` の `## 0.1.9 - 2026-05-27` を起点に、ユーザーが理解できる粒度に書き直してここに記録してください。

## Suggested Release Title

`Codex Approval Guard v0.1.9`

## Short Summary

v0.1.9 は、自動承認の異常連発や UI Automation の連続失敗を検出してガードを自動停止する回路ブレーカーを追加します。あわせて、タスクトレイ常駐、単一インスタンス起動、適応 polling、audit log の記録改善、CSP 有効化、承認/拒否ラベルの誤検出修正を行い、長時間常駐させたときの安全性と安定性を高めます。

## Suggested GitHub Release Body

### Highlights

- 同じコマンドが短時間に自動承認され続けた場合、ガードを paused に切り替える回路ブレーカーを追加しました。
- UI Automation の観測が連続失敗した場合も paused に切り替え、理由を audit log に残します。
- ウィンドウの X ボタンで終了せず、タスクトレイに常駐するようになりました。
- Windows / macOS で単一インスタンス起動を強制し、複数プロセスが同じ承認ダイアログを操作する競合を防ぎます。
- 背景 polling を固定間隔から適応型に変更し、検出直後は速く、アイドル時は軽く動くようにしました。
- audit log の脱敏対象と記録粒度を改善し、短時間の繰り返し操作も追跡しやすくしました。
- WebView CSP を有効化し、外部 URL を開く backend コマンドを http/https のみに制限しました。

### Why This Release Matters

Codex 側がループ状態になったり、UI Automation が継続的に失敗したりすると、自動操作ツールは「止まる条件」を持っていない限り同じ操作を繰り返す可能性があります。

v0.1.9 では、こうした異常を検出した時点で `policy.paused = true` に倒し、作動理由を `circuit_breaker:*` として audit log に残します。ユーザーは原因を確認してから手動で再開できます。

また、常駐アプリとしての扱いやすさも改善しました。ウィンドウを閉じても監視は継続し、トレイから表示・終了できます。単一インスタンス化により、複数起動による policy 書き込み競合や二重クリックも避けられます。

### User-Facing Improvements

#### 安全停止

- auto-approve burst guard を追加。
- UIA observe failure guard を追加。
- 回路ブレーカー作動時は paused に切り替え、audit log に作動理由を記録。

#### 常駐動作

- タスクトレイアイコンを追加。
- X ボタンは終了ではなく非表示に変更。
- トレイメニューから「ウィンドウを表示」と「終了」を実行可能。
- Windows / macOS で単一インスタンス起動を強制。

#### 精度と安定性

- `yes` / `no` / `approve` / `deny` などの英語 matcher を単語境界ベースに変更し、`now` や `eyes` などへの誤マッチを防止。
- command parser で Unicode の lowercase 変換によりバイト長が変わる入力でも panic しないように修正。
- Windows UIA のプロセス名取得で全プロセス列挙を繰り返さず、polling の負荷を軽減。
- Escape broadcast 経路で子孫 HWND も対象にし、列挙上限に達した場合は notes に残すように改善。

#### 監査とセキュリティ

- audit log の擬似 dedup を廃止し、自動操作ごとの記録を保持。
- `window_title` / `cwd` / `target_paths` / `requested_permission` も脱敏対象に追加。
- WebView CSP を有効化。
- 外部 URL を開く backend コマンドを http/https のみに制限。

### Suggested "Upgrade Notes" Section

このリリースでは既存設定の移行操作は不要です。

ウィンドウの X ボタンはアプリ終了ではなくトレイへの非表示になります。監視を完全に止めたい場合は、トレイメニューの「終了」を使ってください。

回路ブレーカーにより paused になった場合は、audit log の `circuit_breaker:*` エントリで理由を確認してから手動で再開してください。

### Suggested "Who Should Update" Section

このリリースは特に次のユーザーに有用です:

- Codex Approval Guard を長時間常駐させて使う
- Codex 側の承認ループや UI Automation の不調時に自動操作を止めたい
- Windows / macOS で複数インスタンス起動による競合を避けたい
- audit log の脱敏範囲と記録粒度を改善したい
- 承認/拒否ラベルの誤検出を減らしたい
