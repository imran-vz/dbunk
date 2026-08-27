// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { DiagnosisPanel } from "@/components/connection-form/diagnosis-panel";
import type { ConnectionDiagnosis } from "@/lib/store/types";

afterEach(cleanup);

const PASSED: ConnectionDiagnosis = {
  engine: "PostgreSQL",
  stages: [
    {
      stage: "tunnel",
      result: {
        status: "passed",
        elapsedMs: 41,
        detail: { kind: "tunnel", localEndpoint: "127.0.0.1:54912" },
      },
    },
    {
      stage: "dns",
      result: {
        status: "passed",
        elapsedMs: 3,
        detail: { kind: "dns", addresses: ["10.20.4.17"] },
      },
    },
    { stage: "tcp", result: { status: "passed", elapsedMs: 12 } },
    {
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
          clientCertificatePresented: true,
          poolHostnameVerification: "caOnly",
        },
      },
    },
    { stage: "authentication", result: { status: "passed", elapsedMs: 24 } },
    {
      stage: "database",
      result: {
        status: "passed",
        elapsedMs: 9,
        detail: { kind: "database", serverVersion: "PostgreSQL 16.4" },
      },
    },
  ],
  outcome: { kind: "reachable", latencyMs: 147 },
  warnings: ["poolHostnameVerificationCaOnly"],
};

const FAILED_TLS: ConnectionDiagnosis = {
  engine: "PostgreSQL",
  stages: [
    { stage: "tunnel", result: { status: "skipped", reason: "noTunnel" } },
    { stage: "dns", result: { status: "passed", elapsedMs: 4 } },
    { stage: "tcp", result: { status: "passed", elapsedMs: 18 } },
    {
      stage: "tls",
      result: {
        status: "failed",
        elapsedMs: 71,
        kind: "certificateUntrusted",
        message: "invalid peer certificate: UnknownIssuer",
      },
    },
    {
      stage: "authentication",
      result: { status: "skipped", reason: "blockedByEarlierFailure" },
    },
    {
      stage: "database",
      result: { status: "skipped", reason: "blockedByEarlierFailure" },
    },
  ],
  outcome: { kind: "failed", stage: "tls" },
  warnings: [],
};

const PLAINTEXT: ConnectionDiagnosis = {
  engine: "PostgreSQL",
  stages: [
    { stage: "tunnel", result: { status: "skipped", reason: "noTunnel" } },
    { stage: "dns", result: { status: "passed", elapsedMs: 3 } },
    { stage: "tcp", result: { status: "passed", elapsedMs: 11 } },
    {
      stage: "tls",
      result: {
        status: "passed",
        elapsedMs: 2,
        detail: {
          kind: "tls",
          encrypted: false,
          protocol: null,
          cipher: null,
          certificateVerified: false,
          hostnameVerified: false,
          clientCertificatePresented: false,
          poolHostnameVerification: "notApplicable",
        },
      },
    },
    { stage: "authentication", result: { status: "passed", elapsedMs: 31 } },
    { stage: "database", result: { status: "passed", elapsedMs: 8 } },
  ],
  outcome: { kind: "reachable", latencyMs: 55 },
  warnings: ["notEncrypted", "productionWithoutVerification"],
};

describe("DiagnosisPanel", () => {
  it("renders every stage in order with its detail and elapsed time", () => {
    render(<DiagnosisPanel report={PASSED} />);
    const rows = screen.getAllByRole("listitem").map((row) => row.textContent);
    expect(rows).toEqual([
      "SSH tunnellocal 127.0.0.1:5491241 ms",
      "DNS10.20.4.173 ms",
      "TCP12 ms",
      "TLSTLSv1.358 ms",
      "Authentication24 ms",
      "DatabasePostgreSQL 16.49 ms",
    ]);
    expect(screen.getByRole("status").textContent).toContain("Reachable147 ms");
    expect(
      screen.getByText(
        "Encrypted · TLSv1.3 · TLS_AES_256_GCM_SHA384 · chain verified · host name verified · client certificate presented",
      ),
    ).toBeTruthy();
    expect(screen.getByRole("note").textContent).toMatch(
      /pooled driver that verifies the certificate chain only/,
    );
  });

  it("names the failing stage with the kind headline and the backend message", () => {
    render(<DiagnosisPanel report={FAILED_TLS} />);
    expect(screen.getByRole("alert").textContent).toContain("Failed at TLS");
    expect(
      screen.getByText("TLS: the server certificate is not trusted"),
    ).toBeTruthy();
    expect(
      screen.getByText("invalid peer certificate: UnknownIssuer"),
    ).toBeTruthy();
    const rows = screen.getAllByRole("listitem").map((row) => row.textContent);
    expect(rows[0]).toBe("SSH tunnelno tunnel configured");
    expect(rows[4]).toBe("Authenticationnot reached");
    expect(rows[5]).toBe("Databasenot reached");
    expect(screen.queryByText(/^Encrypted/)).toBeNull();
  });

  it("renders the plaintext fallback as danger, never inside a success banner", () => {
    render(<DiagnosisPanel report={PLAINTEXT} />);
    const alerts = screen.getAllByRole("alert");
    expect(alerts.map((alert) => alert.textContent)).toEqual([
      "This connection is not encrypted. The server did not offer TLS and “prefer” fell back to plaintext.",
      "This is a production connection and the server certificate is not verified.",
    ]);
    expect(alerts[0]?.className).toContain("text-danger");
    expect(alerts[1]?.className).toContain("text-warning");
    // The outcome header is a status line, not a banner wrapping the warning.
    const status = screen.getByRole("status");
    expect(status.textContent).toContain("Reachable55 ms");
    expect(status.className).not.toMatch(/bg-(accent|success)/);
    expect(
      screen.getByText("plaintext — server offered no TLS").className,
    ).toContain("text-danger");
    expect(screen.queryByText(/^Encrypted/)).toBeNull();
  });
});
