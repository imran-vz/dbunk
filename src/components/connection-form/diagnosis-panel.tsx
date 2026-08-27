/**
 * Test Connection result as a checklist (Plan 012, mock A): one row per
 * stage in the backend's fixed order, the failing stage expanded in
 * place, skipped stages muted with their reason, then the TLS summary
 * and the report's warnings. Everything shown is derived from the
 * report — a `reachable` outcome is a header line, never a green banner
 * that a `notEncrypted` warning would have to sit inside.
 */

import { IconCheck, IconMinus, IconX } from "@tabler/icons-react";

import {
  DIAGNOSIS_SKIP_REASON,
  DIAGNOSIS_STAGE_LABEL,
  DIAGNOSIS_WARNING_COPY,
  diagnosisElapsedMs,
  diagnosisFailureDetail,
  diagnosisFailureHeadline,
  diagnosisPassedDetail,
  diagnosisTlsSummary,
} from "@/lib/connection-diagnosis";
import type {
  ConnectionDiagnosis,
  DiagnosisStage,
  DiagnosisWarning,
} from "@/lib/store/types";
import { cn } from "@/lib/utils";

export function DiagnosisPanel({ report }: { report: ConnectionDiagnosis }) {
  const summary = diagnosisTlsSummary(report);
  const failed = report.outcome.kind === "failed";
  return (
    <div
      role={failed ? "alert" : "status"}
      className="grid gap-2 rounded-md border border-border-subtle bg-surface-panel px-3 py-2.5"
    >
      <div className="flex items-center justify-between gap-2 text-xs font-medium">
        {report.outcome.kind === "reachable" ? (
          <>
            <span>Reachable</span>
            <span className="font-mono text-2xs font-normal text-text-muted">
              {report.outcome.latencyMs} ms
            </span>
          </>
        ) : (
          <>
            <span className="text-danger">
              Failed at {DIAGNOSIS_STAGE_LABEL[report.outcome.stage]}
            </span>
            <span className="font-mono text-2xs font-normal text-text-muted">
              {diagnosisElapsedMs(report)} ms
            </span>
          </>
        )}
      </div>
      <ol className="m-0 grid list-none gap-0.5 p-0">
        {report.stages.map((stage) => (
          <StageRow key={stage.stage} stage={stage} />
        ))}
      </ol>
      {summary ? (
        <p className="m-0 text-2xs text-text-secondary">{summary}</p>
      ) : null}
      {report.warnings.map((warning) => (
        <WarningLine key={warning} warning={warning} />
      ))}
    </div>
  );
}

function StageRow({ stage }: { stage: DiagnosisStage }) {
  const label = DIAGNOSIS_STAGE_LABEL[stage.stage];
  const { result } = stage;
  if (result.status === "skipped") {
    return (
      <li className="flex items-baseline gap-2 py-0.5 text-2xs text-text-disabled">
        <IconMinus className="size-3 shrink-0 self-center" />
        <span className="w-24 shrink-0">{label}</span>
        <span>{DIAGNOSIS_SKIP_REASON[result.reason]}</span>
      </li>
    );
  }
  if (result.status === "failed") {
    return (
      <li className="grid gap-1 py-0.5 text-2xs">
        <div className="flex items-baseline gap-2 text-danger">
          <IconX className="size-3 shrink-0 self-center" />
          <span className="w-24 shrink-0 font-medium">{label}</span>
          <span className="min-w-0 flex-1">
            {diagnosisFailureHeadline(result.kind)}
          </span>
          <span className="font-mono text-text-muted">
            {result.elapsedMs} ms
          </span>
        </div>
        <pre className="m-0 ml-5 whitespace-pre-wrap break-words rounded-sm bg-danger/10 px-2 py-1 font-mono text-2xs text-danger">
          {diagnosisFailureDetail(result.kind, result.message)}
        </pre>
      </li>
    );
  }
  const detail = diagnosisPassedDetail(stage);
  return (
    <li className="flex items-baseline gap-2 py-0.5 text-2xs">
      <IconCheck className="size-3 shrink-0 self-center text-success" />
      <span className="w-24 shrink-0 font-medium text-foreground">{label}</span>
      <span
        className={cn(
          "min-w-0 flex-1 truncate",
          detail?.tone === "danger" ? "text-danger" : "text-text-muted",
        )}
      >
        {detail?.text ?? ""}
      </span>
      <span className="font-mono text-text-muted">{result.elapsedMs} ms</span>
    </li>
  );
}

const WARNING_TONE = {
  danger: "border-danger/30 bg-danger/10 text-danger",
  warning: "border-warning/30 bg-warning/10 text-warning",
  info: "border-info/30 bg-info/10 text-info",
};

function WarningLine({ warning }: { warning: DiagnosisWarning }) {
  const copy = DIAGNOSIS_WARNING_COPY[warning];
  return (
    <p
      role={copy.tone === "info" ? "note" : "alert"}
      className={cn(
        "m-0 rounded-md border px-2.5 py-1.5 text-2xs",
        WARNING_TONE[copy.tone],
      )}
    >
      {copy.text}
    </p>
  );
}
