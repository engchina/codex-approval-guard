# Architecture

`codex-approval-guard` は、Codex の承認疲れを減らすための Windows / macOS 向けデスクトップ補助ツールです。Codex 承認ウィンドウをバックグラウンドで監視し、検出された承認要求を自動でクリック（承認）します。緊急停止（pause）と監査ログによって、利用者がいつでも介入・追跡できる状態を保ちます。

## 構成

| Layer | 技術 | 役割 |
| --- | --- | --- |
| Desktop shell | Tauri v2 | ウィンドウ、配布、ネイティブ bridge |
| UI | React / TypeScript | 監視状態、観察結果、監査ログの操作画面 |
| Core | Rust | 承認リクエスト判定、監査、redaction |
| Parser | Rust fixtures | UI Automation raw text から command / cwd / target path を抽出 |
| Windows adapter | UI Automation | Codex 承認ウィンドウの検出と「是 / 提交」のクリック実行 |
| macOS adapter | Accessibility API | 後続フェーズで検出を担当（現在は未接続） |

## 判定フロー

```mermaid
flowchart TD
  A["Platform adapter (background poll)"] --> B["Codex context check"]
  B -->|Codex 文脈なし| Z["何もしない"]
  B -->|Codex 文脈あり| C["Policy engine"]
  C --> D{"paused?"}
  D -->|yes| P["prompt + audit（自動承認しない）"]
  D -->|no| G["approve + auto-click + audit"]
```

## 境界

- 対象 platform は Windows と macOS のみです。
- Codex 以外のウィンドウは承認対象にしません。
- Parser は `Codex` を含まない approval / yes-no dialog を無視します。
- ボタン文言だけでは承認しません（UI Automation の承認文脈検出を必須とします）。
- Windows observer は UI Automation で承認文脈を検出し、paused でなければ Invoke / click を呼び出して「是 / 提交」を送信します。
- 緊急停止（paused = true）の間は自動クリックを行わず、policy 判定は `prompt` を返します。
- macOS Accessibility adapter はまだ未接続です。
- 監査ログは JSONL で保存し、機微情報に見える語句は記録前に redaction します。

## Parser fixture

`src-tauri/fixtures/ui_text` に UI Automation raw text の positive / negative fixture を置きます。positive は Codex 承認文脈から command、cwd、target path を抽出できることを確認します。negative は UAC、ブラウザ権限、VPN などの一般的な Yes / Allow dialog を誤検出しないことを確認します。

監査ログは observation と auto-approve 実行を JSONL に追記し、保存時には token、secret、password、credential、id_rsa に見える語句を redaction します。
