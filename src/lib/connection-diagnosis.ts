/**
 * Copy and derivations for the staged connection diagnosis (ADR-0025).
 * Pure so the panel renders exactly what these return, and so the typed
 * transport-error formatters (query session, table browse, result
 * mutation) share the TLS headlines — a failed connect reads the same
 * in the form and in a session tab.
 */

import type {
  ConnectionDiagnosis,
  DiagnosisFailureKind,
  DiagnosisSkipReason,
  DiagnosisStage,
  DiagnosisStageKind,
  DiagnosisWarning,
  StoredConnection,
  TlsFailureKind,
} from "@/lib/store/types";

export const DIAGNOSIS_STAGE_LABEL = {
  tunnel: "SSH tunnel",
  dns: "DNS",
  tcp: "TCP",
  tls: "TLS",
  authentication: "Authentication",
  database: "Database",
} satisfies Record<DiagnosisStageKind, string>;

/**
 * `headline` is what the UI leads with; `backendPrefix` mirrors the
 * wording `postgres/connect_error.rs::tls_failure_message` puts in front
 * of its detail, so a message can be shown once, not twice.
 */
const TLS_FAILURE_COPY = {
  serverRefusedTls: {
    headline: "TLS: the server does not support TLS on this port",
    backendPrefix: "The server does not support TLS on this port",
  },
  certificateUntrusted: {
    headline: "TLS: the server certificate is not trusted",
    backendPrefix: "The server certificate is not trusted",
  },
  hostnameMismatch: {
    headline: "TLS: the certificate does not match the expected host name",
    backendPrefix:
      "The server certificate does not match the expected host name",
  },
  clientCertificateRejected: {
    headline: "TLS: the server rejected the client certificate",
    backendPrefix: "The server rejected the client certificate",
  },
  invalidLocalMaterial: {
    headline: "TLS: a local certificate or key could not be used",
    backendPrefix: "Local certificate material is invalid",
  },
  handshakeFailed: {
    headline: "TLS: the handshake failed",
    backendPrefix: "The TLS handshake failed",
  },
} satisfies Record<TlsFailureKind, { headline: string; backendPrefix: string }>;

export const TLS_FAILURE_HEADLINE = {
  serverRefusedTls: TLS_FAILURE_COPY.serverRefusedTls.headline,
  certificateUntrusted: TLS_FAILURE_COPY.certificateUntrusted.headline,
  hostnameMismatch: TLS_FAILURE_COPY.hostnameMismatch.headline,
  clientCertificateRejected:
    TLS_FAILURE_COPY.clientCertificateRejected.headline,
  invalidLocalMaterial: TLS_FAILURE_COPY.invalidLocalMaterial.headline,
  handshakeFailed: TLS_FAILURE_COPY.handshakeFailed.headline,
} satisfies Record<TlsFailureKind, string>;

export function isTlsFailureKind(
  kind: DiagnosisFailureKind,
): kind is TlsFailureKind {
  return kind in TLS_FAILURE_COPY;
}

/**
 * The part of a TLS failure message that the headline does not already
 * say. Foreign messages (the material loader's own text, or a future
 * backend wording) come back untouched.
 */
export function tlsFailureDetail(
  kind: TlsFailureKind,
  message: string,
): string {
  const { backendPrefix } = TLS_FAILURE_COPY[kind];
  if (message === backendPrefix) return "";
  const prefix = `${backendPrefix}: `;
  return message.startsWith(prefix) ? message.slice(prefix.length) : message;
}

/** One line for a typed `tlsFailed` transport error: headline — detail. */
export function formatTlsFailure(
  kind: TlsFailureKind,
  message: string,
): string {
  const detail = tlsFailureDetail(kind, message);
  const { headline } = TLS_FAILURE_COPY[kind];
  return detail ? `${headline} — ${detail}` : headline;
}

/** The message body under a failed diagnosis stage, minus any headline. */
export function diagnosisFailureDetail(
  kind: DiagnosisFailureKind,
  message: string,
): string {
  return isTlsFailureKind(kind) ? tlsFailureDetail(kind, message) : message;
}

const OTHER_FAILURE_HEADLINE = {
  tunnelFailed: "The SSH tunnel could not be established",
  dnsUnresolvable: "The host name could not be resolved",
  connectionRefused: "The server refused the TCP connection",
  timedOut: "Timed out",
  unreachable: "The host is unreachable",
  authenticationFailed: "Authentication failed",
  databaseMissing: "The database does not exist",
  other: "Failed",
} satisfies Record<Exclude<DiagnosisFailureKind, TlsFailureKind>, string>;

const FAILURE_HEADLINE = {
  ...TLS_FAILURE_HEADLINE,
  ...OTHER_FAILURE_HEADLINE,
} satisfies Record<DiagnosisFailureKind, string>;

export function diagnosisFailureHeadline(kind: DiagnosisFailureKind): string {
  return FAILURE_HEADLINE[kind];
}

export const DIAGNOSIS_SKIP_REASON = {
  noTunnel: "no tunnel configured",
  tlsDisabled: "TLS disabled",
  blockedByEarlierFailure: "not reached",
  notApplicable: "not applicable",
} satisfies Record<DiagnosisSkipReason, string>;

export const DIAGNOSIS_WARNING_COPY = {
  notEncrypted: {
    tone: "danger",
    text: "This connection is not encrypted. The server did not offer TLS and “prefer” fell back to plaintext.",
  },
  poolHostnameVerificationCaOnly: {
    tone: "info",
    text: "Metadata and admin queries use a pooled driver that verifies the certificate chain only — not the host name — when the connection is tunnelled.",
  },
  productionWithoutVerification: {
    tone: "warning",
    text: "This is a production connection and the server certificate is not verified.",
  },
} satisfies Record<
  DiagnosisWarning,
  { tone: "danger" | "warning" | "info"; text: string }
>;

export type DiagnosisDetailLine = {
  text: string;
  /** `danger` marks a passed stage whose detail is still bad news. */
  tone: "muted" | "danger";
};

/**
 * The one-line detail beside a passed stage, so a green tick is never
 * information-free. A passed `tls` stage without encryption is the
 * `prefer` plaintext fallback — passed, but rendered as the danger it is.
 */
export function diagnosisPassedDetail(
  stage: DiagnosisStage,
): DiagnosisDetailLine | null {
  const { result } = stage;
  if (result.status !== "passed" || !result.detail) return null;
  const { detail } = result;
  switch (detail.kind) {
    case "tunnel":
      return { text: `local ${detail.localEndpoint}`, tone: "muted" };
    case "dns":
      return { text: detail.addresses.join(", "), tone: "muted" };
    case "database":
      return { text: detail.serverVersion, tone: "muted" };
    case "tls":
      return detail.encrypted
        ? { text: detail.protocol ?? "encrypted", tone: "muted" }
        : { text: "plaintext — server offered no TLS", tone: "danger" };
  }
}

/**
 * "Encrypted · TLSv1.3 · … · chain verified · host name verified" from
 * the report alone — never from the mode the user selected. Null when
 * the session is not encrypted; the `notEncrypted` warning covers that.
 */
export function diagnosisTlsSummary(
  report: ConnectionDiagnosis,
): string | null {
  const tls = report.stages.find((stage) => stage.stage === "tls");
  const detail =
    tls?.result.status === "passed" && tls.result.detail?.kind === "tls"
      ? tls.result.detail
      : null;
  if (!detail?.encrypted) return null;
  const parts = [
    "Encrypted",
    detail.protocol,
    detail.cipher,
    detail.certificateVerified ? "chain verified" : "chain not verified",
    detail.hostnameVerified ? "host name verified" : "host name not verified",
    detail.clientCertificatePresented ? "client certificate presented" : null,
  ];
  return parts.filter((part) => part !== null).join(" · ");
}

/** Wall-clock across every stage that ran, for the failed-outcome header. */
export function diagnosisElapsedMs(report: ConnectionDiagnosis): number {
  return report.stages.reduce(
    (total, stage) =>
      stage.result.status === "skipped"
        ? total
        : total + stage.result.elapsedMs,
    0,
  );
}

/**
 * The report the browser dev build returns in place of a real probe. It
 * never claims encryption it did not observe: `tls` is skipped, not
 * passed, so the summary line and warnings stay honest in dev too.
 */
export function syntheticReachableDiagnosis(
  connection: StoredConnection,
): ConnectionDiagnosis {
  const tunnelled =
    connection.engine !== "SQLite" && connection.sshTunnel?.enabled === true;
  return {
    engine: connection.engine,
    stages: [
      {
        stage: "tunnel",
        result: tunnelled
          ? {
              status: "passed",
              elapsedMs: 0,
              detail: { kind: "tunnel", localEndpoint: "127.0.0.1:0" },
            }
          : { status: "skipped", reason: "noTunnel" },
      },
      { stage: "dns", result: { status: "passed", elapsedMs: 0 } },
      { stage: "tcp", result: { status: "passed", elapsedMs: 0 } },
      { stage: "tls", result: { status: "skipped", reason: "notApplicable" } },
      { stage: "authentication", result: { status: "passed", elapsedMs: 0 } },
      { stage: "database", result: { status: "passed", elapsedMs: 0 } },
    ],
    outcome: { kind: "reachable", latencyMs: 0 },
    warnings: [],
  };
}
