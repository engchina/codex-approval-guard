export type DecisionAction =
  | "observe_only"
  | "approve"
  | "dismiss"
  | "deny"
  | "prompt"
  | "ignore";

export type RiskLevel = "low" | "medium" | "high";

export interface ApprovalRequest {
  id?: string | null;
  source_app: string;
  window_title: string;
  prompt_text: string;
  command?: string | null;
  cwd?: string | null;
  target_paths: string[];
  requested_permission?: string | null;
}

export interface ApprovalDecision {
  action: DecisionAction;
  risk: RiskLevel;
  reason: string;
  matched_rule?: string | null;
  would_auto_approve: boolean;
}

export interface PolicyConfig {
  paused: boolean;
  allow_git_add: boolean;
  allow_git_commit: boolean;
}

export interface PlatformSnapshot {
  backend: string;
  available: boolean;
  focused_window_title?: string | null;
  details: string;
}

export interface ObservedApproval {
  request: ApprovalRequest;
  raw_text: string[];
  detected_by: string;
}

export interface ObserveDiagnostics {
  windows_scanned: number;
  notes: string[];
}

export interface ApprovalObservation {
  platform: PlatformSnapshot;
  observed?: ObservedApproval | null;
  decision?: ApprovalDecision | null;
  recorded: boolean;
  details: string;
  diagnostics: ObserveDiagnostics;
}

export interface ClickOutcome {
  target_window: string;
  yes_invoked: boolean;
  submit_invoked: boolean;
  notes: string[];
}

export interface AutoApproveOutcome {
  decision: ApprovalDecision;
  click: ClickOutcome;
  audited: boolean;
}

export interface AuditEntry {
  id: string;
  created_at: string;
  request: ApprovalRequest;
  decision: ApprovalDecision;
}

export interface GuardState {
  policy: PolicyConfig;
  platform: PlatformSnapshot;
  recent_audits: AuditEntry[];
  audit_log_path: string;
}

export interface UpdateCheckResult {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  releaseUrl: string;
  downloadUrl?: string | null;
  assetName?: string | null;
  releaseNotes: string;
  publishedAt?: string | null;
}
