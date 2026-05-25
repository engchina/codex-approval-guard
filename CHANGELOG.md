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
