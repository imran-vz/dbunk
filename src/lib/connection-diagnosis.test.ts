import { describe, expect, it } from "vitest";

import type { ConnectionDiagnosis, DiagnosisStage } from "@/lib/store/types";

import {
  diagnosisElapsedMs,
  diagnosisFailureDetail,
  diagnosisFailureHeadline,
  formatTlsFailure,
  diagnosisPassedDetail,
  diagnosisTlsSummary,
  syntheticReachableDiagnosis,
} from "./connection-diagnosis";

const tlsStage = (
  detail: Partial<
    Extract<
      NonNullable<
        Extract<DiagnosisStage["result"], { status: "passed" }>["detail"]
      >,
      { kind: "tls" }
    >
  >,
): DiagnosisStage => ({
  stage: "tls",
  result: {
    status: "passed",
    elapsedMs: 58,
    detail: {
      kind: "tls",
      encrypted: true,
      protocol: "TLSv1.3",
      cipher: "TLS_AES_256_GCM_SHA384",
      certificateVerified: true,
      hostnameVerified: true,
      clientCertificatePresented: false,
      poolHostnameVerification: "full",
      ...detail,
    },
  },
});

const report = (
  stages: DiagnosisStage[],
  outcome: ConnectionDiagnosis["outcome"] = { kind: "reachable", latencyMs: 1 },
): ConnectionDiagnosis => ({
  engine: "PostgreSQL",
  stages,
  outcome,
  warnings: [],
});

describe("diagnosisTlsSummary", () => {
  it("lists protocol, cipher, and every verification the report claims", () => {
    expect(
      diagnosisTlsSummary(
        report([tlsStage({ clientCertificatePresented: true })]),
      ),
    ).toBe(
      "Encrypted · TLSv1.3 · TLS_AES_256_GCM_SHA384 · chain verified · host name verified · client certificate presented",
    );
  });

  it("states what was not verified instead of omitting it", () => {
    expect(
      diagnosisTlsSummary(
        report([
          tlsStage({ certificateVerified: false, hostnameVerified: false }),
        ]),
      ),
    ).toContain("chain not verified · host name not verified");
  });

  it("returns null when the session is not encrypted or TLS did not run", () => {
    expect(
      diagnosisTlsSummary(
        report([tlsStage({ encrypted: false, protocol: null, cipher: null })]),
      ),
    ).toBeNull();
    expect(
      diagnosisTlsSummary(
        report([
          {
            stage: "tls",
            result: { status: "skipped", reason: "tlsDisabled" },
          },
        ]),
      ),
    ).toBeNull();
    expect(
      diagnosisTlsSummary(
        report([
          {
            stage: "tls",
            result: {
              status: "failed",
              elapsedMs: 3,
              kind: "handshakeFailed",
              message: "x",
            },
          },
        ]),
      ),
    ).toBeNull();
  });
});

describe("diagnosisPassedDetail", () => {
  it("renders the prefer plaintext fallback as a danger-toned detail", () => {
    expect(
      diagnosisPassedDetail(
        tlsStage({ encrypted: false, protocol: null, cipher: null }),
      ),
    ).toEqual({ text: "plaintext — server offered no TLS", tone: "danger" });
    expect(diagnosisPassedDetail(tlsStage({}))).toEqual({
      text: "TLSv1.3",
      tone: "muted",
    });
  });

  it("surfaces tunnel endpoint, resolved addresses, and server version", () => {
    expect(
      diagnosisPassedDetail({
        stage: "tunnel",
        result: {
          status: "passed",
          elapsedMs: 1,
          detail: { kind: "tunnel", localEndpoint: "127.0.0.1:54912" },
        },
      }),
    ).toEqual({ text: "local 127.0.0.1:54912", tone: "muted" });
    expect(
      diagnosisPassedDetail({
        stage: "dns",
        result: {
          status: "passed",
          elapsedMs: 1,
          detail: { kind: "dns", addresses: ["10.0.0.1", "10.0.0.2"] },
        },
      }),
    ).toEqual({ text: "10.0.0.1, 10.0.0.2", tone: "muted" });
    expect(
      diagnosisPassedDetail({
        stage: "tcp",
        result: { status: "passed", elapsedMs: 1 },
      }),
    ).toBeNull();
  });
});

describe("diagnosisFailureHeadline", () => {
  it("names TLS failures as TLS and every other kind by its stage", () => {
    expect(diagnosisFailureHeadline("certificateUntrusted")).toBe(
      "TLS: the server certificate is not trusted",
    );
    expect(diagnosisFailureHeadline("hostnameMismatch")).toMatch(/^TLS:/);
    expect(diagnosisFailureHeadline("authenticationFailed")).toBe(
      "Authentication failed",
    );
    expect(diagnosisFailureHeadline("databaseMissing")).toBe(
      "The database does not exist",
    );
  });
});

describe("diagnosisElapsedMs", () => {
  it("sums only the stages that ran", () => {
    expect(
      diagnosisElapsedMs(
        report(
          [
            {
              stage: "tunnel",
              result: { status: "skipped", reason: "noTunnel" },
            },
            { stage: "dns", result: { status: "passed", elapsedMs: 4 } },
            {
              stage: "tls",
              result: {
                status: "failed",
                elapsedMs: 71,
                kind: "certificateUntrusted",
                message: "x",
              },
            },
          ],
          { kind: "failed", stage: "tls" },
        ),
      ),
    ).toBe(75);
  });
});

describe("syntheticReachableDiagnosis", () => {
  it("never claims encryption the dev stub did not observe", () => {
    const synthetic = syntheticReachableDiagnosis({
      id: "pg",
      name: "pg",
      engine: "PostgreSQL",
      host: "localhost",
      database: "postgres",
      port: 5432,
      user: "postgres",
      password: "",
      role: "read/write",
      ssl: true,
    });
    expect(synthetic.outcome).toEqual({ kind: "reachable", latencyMs: 0 });
    expect(synthetic.stages.map((stage) => stage.stage)).toEqual([
      "tunnel",
      "dns",
      "tcp",
      "tls",
      "authentication",
      "database",
    ]);
    expect(
      synthetic.stages.find((stage) => stage.stage === "tls")?.result,
    ).toEqual({ status: "skipped", reason: "notApplicable" });
    expect(diagnosisTlsSummary(synthetic)).toBeNull();
  });
});

describe("formatTlsFailure", () => {
  it("shows the backend's headline once, then its detail", () => {
    expect(
      formatTlsFailure(
        "certificateUntrusted",
        "The server certificate is not trusted: invalid peer certificate: UnknownIssuer",
      ),
    ).toBe(
      "TLS: the server certificate is not trusted — invalid peer certificate: UnknownIssuer",
    );
  });

  it("keeps a message the headline does not already cover", () => {
    expect(
      formatTlsFailure(
        "invalidLocalMaterial",
        "client key /c.key is passphrase-protected",
      ),
    ).toBe(
      "TLS: a local certificate or key could not be used — client key /c.key is passphrase-protected",
    );
  });

  it("falls back to the headline alone when the message adds nothing", () => {
    expect(
      formatTlsFailure("handshakeFailed", "The TLS handshake failed"),
    ).toBe("TLS: the handshake failed");
    expect(formatTlsFailure("serverRefusedTls", "")).toBe(
      "TLS: the server does not support TLS on this port",
    );
  });

  it("strips only TLS headlines from diagnosis details", () => {
    expect(
      diagnosisFailureDetail(
        "hostnameMismatch",
        "The server certificate does not match the expected host name: NotValidForName",
      ),
    ).toBe("NotValidForName");
    expect(
      diagnosisFailureDetail(
        "authenticationFailed",
        "password authentication failed",
      ),
    ).toBe("password authentication failed");
  });
});
