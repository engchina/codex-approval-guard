# Changelog

`Codex Approval Guard` のユーザーに見える変更点を記録します。形式は [Keep a Changelog](https://keepachangelog.com/ja/1.1.0/) に準拠します。

## Unreleased

### Added

<!-- 新しいガード対象、自動承認/閉鎖ルール、UI 改善など、ユーザーが気付く新機能。 -->

### Changed

<!-- 既存挙動・既定値・ワークフローの変更で、ユーザーが日常的に気付くもの。 -->

### Fixed

<!-- 自動承認/閉鎖の精度、誤検出、UI、安定性などの不具合修正。 -->

### Internal

<!-- リファクタ、ツール、テスト、リリースプロセスなど、プロジェクト履歴に残したい保守作業。 -->

## 0.1.7 - 2026-05-26

### Added

- ask_user_question 系プロンプトの推奨選択肢 marker として「（おすすめ）」および全角の「（recommended）」を検出できるように追加。
- 日本語・中国語・英語の plan confirmation prompt（例: 「このプランを実施しますか」「是否执行此方案」「execute this plan」）を検出できるように追加。

### Changed

- 承認・拒否・推奨選択肢の番号判定を、半角数字だけでなく全角数字と丸数字にも対応するように改善。
- 送信ボタン判定を、ショートカット表記や補助文が続くラベルでも拾いやすく改善。
- plan confirmation prompt と apply changes prompt では、チャット履歴中の shell command を実行対象として誤抽出しないように改善。

### Fixed

- 日本語 UI の推奨選択肢や全角番号付き選択肢を自動操作候補として拾えない場合がある問題を修正。
- `npm run ...` の後ろに自然言語の確認メモが続く場合に、メモ部分まで command として扱う可能性を修正。

### Internal

- 番号プレフィックス判定を helper に分離し、関連テストを拡充。
- plan confirmation prompt と自然言語メモ付き command 解析のテストを追加。

## 0.1.6 - 2026-05-26

### Added

- 自動操作結果に成功経路を表す `method` ラベルを保持できるように `ClickOutcome` を拡張。
- Codex の ask_user_question 系プロンプトで、番号付きの「（推荐）/（推奨）/(Recommended)」選択肢を自動承認候補として扱うように追加。
- 日本語版 Codex Desktop の「これらの変更を行いますか?」形式の変更適用プロンプトを検出できるように追加。

### Changed

- 監査ログの reason には冗長な notes 全文ではなく、成功した自動操作経路を表す短い `method` のみを記録するように変更。
- WebView 内蔵 dialog の Escape 閉鎖経路を `AttachThreadInput`、`SendInput`、HWND broadcast の複数経路で試し、成功した経路を `method` に記録するように改善。
- ask_user_question 系プロンプトの送信ボタンとして「继续 / Continue / 継続」を扱うように改善。

### Internal

- `ClickOutcome.method` に serde default を付与し、既存の結果 payload との互換性を維持。
- 推奨選択肢 matcher、submit matcher、日本語変更適用 fixture のテスト期待値を追加。

## 0.1.5 - 2026-05-26

### Added

- WebView 内蔵の git commit dialog を `VK_ESCAPE` で閉じる高速経路を追加。

### Changed

- git commit dialog の自動閉鎖では `WM_CLOSE` が使えない場合に Escape 経路を優先し、UIA の閉じるボタン探索へ進む前に安全な候補を試すように改善。
- Escape 経路では Codex の親 HWND と子 HWND にキーイベントを送信し、WebView2 内の modal に届きやすく改善。

### Fixed

- `跳过` / `Skip` / `スキップ` を git commit dialog の閉鎖候補から除外し、Codex メインウィンドウ内の無関係なボタンを誤クリックするリスクを低減。

### Internal

- Escape 経路と close/cancel matcher のテスト期待値を更新。

## 0.1.4 - 2026-05-26

### Added

- git commit dialog をタイトルだけで即時検出し、独立 HWND の場合は `WM_CLOSE` で高速に閉じる経路を追加。
- 自動操作後の監査ログに、対象ウィンドウとクリック経路の trace notes を記録するように追加。

### Changed

- 承認リクエストのバックグラウンドポーリング間隔と自動操作 cooldown を短縮し、git commit dialog などの応答を速く改善。
- git commit dialog と判定済みのリクエストではクリック時の UI ツリー再走査を抑え、観測結果の hint を優先するように改善。
- `关闭/取消` 候補探索時に control type を絞り、WebView2 の巨大 UI ツリーで不要な pattern 取得を減らすように改善。

### Fixed

- git commit dialog の自動閉鎖が UI ツリー走査待ちで遅くなる問題を軽減。

### Internal

- git commit dialog のタイトル判定を `title_matches_git_commit` に分離。
- Windows click 経路に `is_git_commit_hint` を渡せるように platform API を更新。

## 0.1.3 - 2026-05-25

### Added

- `deny` 判定で Codex の「3. 否」相当の選択肢を自動選択し、「提交」まで進める自動拒否処理を追加。
- 直近の自動操作をステータスカードに一時表示し、自動承認・自動拒否・自動閉鎖の対象と適用ルールを確認できるように追加。

### Changed

- Windows UI Automation の invoke が使えない要素に対して、バックグラウンドクリックを安全側の追加手段として試行するように改善。
- Windows AArch64 環境で arm64 配布アセットがない場合、x64 / x86 / 汎用アセットへ順にフォールバックするように更新確認の選択ロジックを改善。
- ダークモード時の主要カード、履歴、コピー操作の配色を調整。

### Fixed

- `deny` 判定が自動操作対象にならず、ユーザーの手動拒否に残っていた問題を修正。

### Internal

- Windows の自動拒否候補検出と AArch64 更新アセット選択のテストを追加。
- バックグラウンドクリック用に `windows-sys` の GDI API feature を追加。

## 0.1.2 - 2026-05-25

### Added

- GitHub Releases の最新公開版を確認し、更新がある場合に UI へ通知するアップデート確認機能を追加。
- 非アクティブな会話のサイドバーに表示される「承認待ち」バッジを検出し、対象会話を自動でアクティブ化する処理を追加。

### Changed

- 更新通知からリリースページまたは適切な配布アセットを開けるように UI を更新。
- Windows UI Automation の承認候補探索を拡張し、承認ダイアログ本体がまだ UI ツリーに出ていないケースでも次回ポーリングへつなげるように改善。

### Fixed

- 別会話で発生した承認待ちが、現在の会話を選択するまで自動承認対象として見つからない問題を軽減。

### Internal

- アップデート確認用に `reqwest` と `semver` を追加。
- 承認待ちバッジ検出と release asset 選択のテストを追加。

## 0.1.1 - 2026-05-25

### Added

- 履歴エリアから監査ログファイルのパスをコピーできるボタンを追加。

### Changed

- 直近履歴の表示を判定種別ごとの左罫線で見分けやすく改善。
- 履歴時刻をローカル時刻として整形し、読みやすさを改善。
- Tauri bundle が Windows / macOS 向けの複数サイズ PNG と ICNS icon を参照するように更新。

### Internal

- ローカルの Cargo target 検証ディレクトリを Git 管理対象外として扱うように整理。

## 0.1.0 - 2026-05-25

### Added

- Codex Desktop の承認ダイアログを Windows UI Automation で検出し、`1. 是` などの第一承認オプションを自動でクリックする初版ガード機能。
- Codex の git 提交ダイアログ（`提交更改` / `Commit Changes` / `変更をコミット`）を検出して関閉ボタン（`关闭` / `跳过` / `✕` などの異字体含む）でダイアログを自動で閉じる `auto_dismiss_git_commit` ルール。
- `git commit` シェルコマンドの承認要求は自動承認せず、ユーザーの手動確認を求める `manual_git_commit` ルール。
- 直近の自動操作を表示する履歴 UI と、JSONL 形式の監査ログ（センシティブ値の `[REDACTED]` 化付き）。
- ガードを停止/再開できる UI トグル。
- macOS 向けのプレースホルダ実装（snapshot のみ。自動承認は未対応）。
