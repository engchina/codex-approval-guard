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

## 0.1.10 - 2026-05-28

### Added

- Codex Desktop の「チャット名を変更 / Rename chat / 重命名聊天」ダイアログを検出し、承認候補として扱わず Escape broadcast で自動閉鎖する保護を追加。

### Changed

- サイドバーの pending 会話をアクティブ化する Windows UIA 経路で、`InvokePattern.invoke` より `SelectionItemPattern.select` を優先するように変更。Codex 側で `Invoke` が rename 操作に割り当てられているケースの副作用を避ける。
- v0.1.9 で追加した auto-approve burst / UIA observe failure の回路ブレーカーをいったん削除。自動 paused への切り替えではなく、今回判明した rename dialog 副作用を直接抑止する実装に戻した。

### Fixed

- サイドバーの「承認が必要」会話を選択する際、Codex の「チャット名を変更」ダイアログが意図せず開く問題を修正。
- rename dialog が開いた状態を承認ダイアログとして誤認し、以後の自動承認観測が詰まる可能性を修正。

### Internal

- rename chat dialog の多言語判定テストを追加。

## 0.1.9 - 2026-05-27

### Added

- 同一コマンドの自動承認が短時間に連発した場合、または UI Automation の観測が連続失敗した場合に、ガードを自動的に paused へ倒す回路ブレーカーを追加。作動理由は audit log に `circuit_breaker:*` として残す。
- タスクトレイ常駐を追加。ウィンドウの X ボタンではプロセスを終了せず非表示にし、トレイの左クリックまたは「ウィンドウを表示」から復帰できる。「終了」メニューでは監視も含めて完全終了する。
- Windows / macOS で単一インスタンス起動を強制し、複数プロセスが同じ `policy.json` や Codex 承認ダイアログを同時に扱う競合を防ぐ。

### Changed

- バックグラウンド polling を固定 600ms から適応型に変更。承認ダイアログ検出直後は 300ms、アイドル時は 1200ms で回し、取りこぼしを抑えつつ UIA IPC 負荷を下げる。
- 設定と audit log の保存先解決で `current_dir()` への fallback をやめ、`app_data_dir` から `app_config_dir` の順に明示的なアプリ用ディレクトリだけを使うように変更。
- audit log は自動操作ごとの記録をそのまま残すように変更。起動時刻ベースの擬似 dedup を廃止し、短時間に同じ操作が繰り返された事実を追跡できるようにした。
- WebView CSP を有効化し、`default-src 'self'` ベースで IPC / dev server / data image など必要な経路だけを許可するように変更。
- 外部 URL を開く backend コマンドを http/https のみに制限し、Windows では `cmd /c start` の空タイトル指定で URL を安全に渡すように変更。

### Fixed

- 承認/拒否ボタン判定の英語部分一致を単語境界ベースに変更し、`no` が `now` / `note` / `another` に、`yes` が `eyes` / `yesterday` に誤マッチする問題を修正。
- command parser で `to_lowercase()` により UTF-8 バイト長が変わる文字を含む場合、結合コマンド分割が文字境界外 slice で panic する可能性を修正。
- audit log の脱敏対象を `window_title` / `cwd` / `target_paths` / `requested_permission` まで広げ、空白や改行を維持したまま token / secret / password / credential を含む token を置換するように修正。
- Windows UIA のプロセス名取得で全プロセス列挙を繰り返す処理をやめ、プロセスイメージパスの basename を使って polling 遅延を抑えるように修正。
- Escape broadcast 経路で親 HWND だけでなく列挙できた子孫 HWND にも送るようにし、列挙上限に達した場合は notes に理由を残すように修正。

### Internal

- `GuardCircuit` の auto-approve burst / UIA failure counter と、audit / policy redaction / matcher / parser の回帰テストを追加。
- `tauri-plugin-single-instance` を Windows / macOS 依存に追加し、不要になった Windows ToolHelp 依存を削除。
- GitHub Releases 更新確認テストのネットワークエラー許容条件を、日本語 error prefix に合わせて更新。

## 0.1.8 - 2026-05-26

### Added

- macOS 対応の本実装。`accessibility-sys` / `core-foundation` / `objc2-app-kit` を用いて Codex Desktop プロセスを `NSWorkspace.runningApplications` から列挙し、AX ツリーを再帰走査して承認ダイアログ・サイドバーの「承認が必要」バッジを検出する。`AXPress` で「1. はい / 3. いいえ / 閉じる / 送信」ボタンを押下、`AXRaise` でサイドバー会話をアクティブ化する。`CGEventSourceSecondsSinceLastEventType` で user_idle を取得して Windows 版と同じ 1500ms ガードを適用する。Windows 版と同等の餓死対策（メイン操作成功後のサイドバー pending 自動アクティブ化）も搭載済み。アクセシビリティ権限が必要。

### Fixed

- アクティブ会話が短い間隔で承認を生成し続ける状況（例: `git status` を秒間隔で実行）で、他プロジェクトのサイドバー「承認が必要」会話が永続的に処理されない餓死（starvation）を修正。アクティブ会話のメイン操作（承認/拒否どちらも）成功後に、追加でサイドバーの pending 会話を 1 件アクティブ化し、次回ポーリングで処理されるようにした。`click_yes` / `click_no` 両系統で対称に動作する。
- macOS AX バックエンドで、非文字列の AX 属性値を CFString として扱う可能性と、可視ラベルが `AXStaticText` 子要素に分離されているボタンを直接押せない可能性を修正。AX 属性の type id を確認し、文字列候補は最近傍のクリック可能な祖先要素へ引き上げて `AXPress` / `AXRaise` する。
- macOS の git commit dialog 閉鎖で、観測フェーズの hint だけを根拠に通常の Codex メインウィンドウを閉じる可能性を修正。クリック対象ウィンドウ自体の title / AX 本文が commit dialog と判定できる場合のみ閉鎖操作を行い、内蔵 modal 判定時はネイティブウィンドウの close button 候補を除外する。

### Internal

- プラットフォーム非依存の文字列マッチャー（`is_first_yes_option` / `is_first_no_option` / `is_recommended_option` / `is_submit_button` / `is_close_or_cancel_button` / `looks_like_approval_keyword` 等）を `platform/matchers.rs` に集約。Windows / macOS 両バックエンドから同一実装を参照することで挙動の乖離を防ぐ。

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
