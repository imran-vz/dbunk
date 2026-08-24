/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters, anti-slop/no-unsafe-dictionary-type -- Transport errors arrive as unstructured invoke rejections and must be decoded at this boundary. */
import { isTlsFailureKind } from "@/lib/store/types";
import type { TlsFailureKind } from "@/lib/store/types";

export const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object";

export const isNullableString = (value: unknown): value is string | null =>
  value === null || typeof value === "string";

export const isNullableNumber = (value: unknown): value is number | null =>
  value === null || typeof value === "number";

export type DecodedTlsFailed = {
  kind: "tlsFailed";
  tlsKind: TlsFailureKind;
  message: string;
};

export type DecodedTimeout = { kind: "timeout"; operation: string };

export type DecodedDatabaseError = {
  kind: "database";
  code: string | null;
  message: string;
  severity: string | null;
  position: number | null;
};

export function decodeTlsFailed(
  error: Record<string, unknown>,
): DecodedTlsFailed | null {
  return typeof error.tlsKind === "string" &&
    isTlsFailureKind(error.tlsKind) &&
    typeof error.message === "string"
    ? { kind: "tlsFailed", tlsKind: error.tlsKind, message: error.message }
    : null;
}

export function decodeTimeout(
  error: Record<string, unknown>,
): DecodedTimeout | null {
  return typeof error.operation === "string"
    ? { kind: "timeout", operation: error.operation }
    : null;
}

export function decodeDatabaseError(
  error: Record<string, unknown>,
): DecodedDatabaseError | null {
  return isNullableString(error.code) &&
    typeof error.message === "string" &&
    isNullableString(error.severity) &&
    isNullableNumber(error.position)
    ? {
        kind: "database",
        code: error.code,
        message: error.message,
        severity: error.severity,
        position: error.position,
      }
    : null;
}
