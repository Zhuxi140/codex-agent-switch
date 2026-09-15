import { OrchestrationJobTrackingView } from "./App";
import type { OrchestrationJobPageResponse } from "./api";

const parentThreadId = "01a0a280-c97b-76b0-9e7c-7829a8397170";
const workerThreadId = "01a0a280-c9f7-7e13-8ef3-ec06eacab1d6";

const page: OrchestrationJobPageResponse = {
  page: 0,
  pageSize: 20,
  totalCount: 2,
  jobs: [
    {
      jobId: "cas-managed-second",
      idempotencyKey: "cas-managed-second",
      state: "COMPLETED",
      agentId: "managed-preview-agent",
      parentThreadId,
      workspaceScopeKey: "c:/workspace/codex-agent-switch",
      taskScopeKey: "cas-managed-worker-proof",
      lastErrorCode: null,
      createdAt: "2026-09-15T00:35:15Z",
      updatedAt: "2026-09-15T00:35:17Z",
      terminalAt: "2026-09-15T00:35:17Z",
      attempts: [{
        attemptId: "preview-managed-attempt-second",
        attemptNo: 1,
        state: "SUCCEEDED",
        routeAction: "REUSE",
        plannedExecutionKind: "MANAGED_WORKER",
        executionKind: "MANAGED_WORKER",
        codexThreadId: workerThreadId,
        codexTurnId: "01a0a280-eab8-7e42-b26a-07de7a55c8f8",
        leaseState: "RELEASED",
        receiptStage: "PARENT_ACKNOWLEDGED",
        reviewDecision: "APPROVE",
        totalTokens: 31_058,
        updatedAt: "2026-09-15T00:35:17Z",
      }],
    },
    {
      jobId: "cas-managed-first",
      idempotencyKey: "cas-managed-first",
      state: "COMPLETED",
      agentId: "managed-preview-agent",
      parentThreadId,
      workspaceScopeKey: "c:/workspace/codex-agent-switch",
      taskScopeKey: "cas-managed-worker-proof",
      lastErrorCode: null,
      createdAt: "2026-09-15T00:35:05Z",
      updatedAt: "2026-09-15T00:35:14Z",
      terminalAt: "2026-09-15T00:35:14Z",
      attempts: [{
        attemptId: "preview-managed-attempt-first",
        attemptNo: 1,
        state: "SUCCEEDED",
        routeAction: "SPAWN",
        plannedExecutionKind: "MANAGED_WORKER",
        executionKind: "MANAGED_WORKER",
        codexThreadId: workerThreadId,
        codexTurnId: "01a0a280-ca0b-7a91-b3b6-efd732ea6022",
        leaseState: "RELEASED",
        receiptStage: "PARENT_ACKNOWLEDGED",
        reviewDecision: "APPROVE",
        totalTokens: 31_058,
        updatedAt: "2026-09-15T00:35:14Z",
      }],
    },
  ],
};

export function OrchestrationTrackingPreview() {
  return (
    <main className="settings-shell" style={{ padding: 24 }}>
      <OrchestrationJobTrackingView error={null} onRefresh={() => undefined} page={page} />
    </main>
  );
}
