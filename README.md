# codex-approval-guard

Codex の承認疲れを減らすための Windows / macOS 向け desktop assistant です。Codex ウィンドウだけを対象に承認要求を検出し、緊急停止と監査ログを組み合わせて、利用者がいつでも介入・追跡できる状態を保ちながら反復承認を自動化します。

## 現在の状態

- Tauri v2 + Rust + React / TypeScript の初期構成
- Rust policy engine（緊急停止対応）
- JSONL 監査ログ
- Windows / macOS platform adapter の境界
- Windows UI Automation 監視（バックグラウンド自動チェックと手動観察）
- 検出時に policy 判定に応じて自動操作（承認・拒否・閉鎖）を実行

## 開発

```powershell
npm install
npm run tauri:dev
```

## 検証

```powershell
npm run build
npm run cargo:test
npm run tauri:build
```

Windows の配布パッケージは既定で NSIS installer を生成します。MSI は WiX / Windows Installer Service の ICE 検証に依存するため、管理された Windows 環境では失敗することがあります。macOS では `npm run tauri:build:mac` で app / dmg bundle を生成します。Linux は対象外です。

`npm run tauri:build` は `src-tauri/target/package` を使います。release exe を手元で起動したままでも、packaging build が既存の `target/release` 成果物を上書きしないためです。

## 設計メモ

詳しくは [docs/architecture.md](docs/architecture.md) を参照してください。
