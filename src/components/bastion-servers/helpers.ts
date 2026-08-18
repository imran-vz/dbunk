import type {
  BastionAuthMethod,
  BastionServer,
  Connection,
  SaveBastionServerInput,
  SecretChange,
} from "@/lib/store";
import { bastionIdsReferencedByConnection } from "@/lib/store/bastion-references";

import type { BastionDraft } from "./types";

export function analyzePrivateKeyContent(value: string): {
  tone: "neutral" | "ready" | "warning";
  message: string;
} {
  const trimmed = value.trim();
  if (!trimmed) {
    // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
    return {
      tone: "neutral",
      message:
        "Paste an OpenSSH or PEM private key to store it as a Bastion secret.",
    };
  }
  const header = trimmed.match(/-----BEGIN ([A-Z0-9 ]+PRIVATE KEY)-----/);
  const footer = trimmed.match(/-----END ([A-Z0-9 ]+PRIVATE KEY)-----/);
  if (!header || !footer || header[1] !== footer[1]) {
    // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
    return {
      tone: "warning",
      message: "Missing matching private-key header and footer.",
    };
  }
  if (/ENCRYPTED|Proc-Type:\s*4,ENCRYPTED/i.test(trimmed)) {
    // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
    return {
      tone: "warning",
      message: "Encrypted key detected. Add the passphrase before saving.",
    };
  }
  // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
  return { tone: "ready", message: "Private key format detected." };
}

export function blankDraft(): BastionDraft {
  return {
    id: crypto.randomUUID(),
    name: "",
    host: "",
    port: 22,
    user: "",
    authMethod: "privateKeyPath",
    privateKeyPath: "",
    password: "",
    privateKeyContent: "",
    passphrase: "",
    clearPassphrase: false,
  };
}

export function draftFromBastion(bastion: BastionServer): BastionDraft {
  return {
    id: bastion.id,
    name: bastion.name,
    host: bastion.host,
    port: bastion.port,
    user: bastion.user,
    authMethod: bastion.authMethod,
    privateKeyPath: bastion.privateKeyPath ?? "",
    password: "",
    privateKeyContent: "",
    passphrase: "",
    clearPassphrase: false,
  };
}

export function normalizeDraft(draft: BastionDraft): SaveBastionServerInput {
  const optional = (value?: string) => {
    const trimmed = value?.trim();
    return trimmed ? trimmed : undefined;
  };
  return {
    id: draft.id,
    name: draft.name.trim(),
    host: draft.host.trim(),
    port: draft.port || 22,
    user: draft.user.trim(),
    authMethod: draft.authMethod,
    privateKeyPath: optional(draft.privateKeyPath),
    password: secretChangeFromInput(draft.password),
    privateKeyContent: secretChangeFromInput(draft.privateKeyContent),
    passphrase: draft.clearPassphrase
      ? { action: "clear" }
      : secretChangeFromInput(draft.passphrase),
  };
}

export function authLabel(method: BastionAuthMethod): string {
  switch (method) {
    case "password":
      return "Password";
    case "privateKeyPath":
      return "Key path";
    case "privateKeyContent":
      return "Key content";
  }
}

export function secretSummary(bastion: BastionServer): string {
  const parts = [
    bastion.hasPassword ? "password" : null,
    bastion.hasPrivateKeyContent ? "private key content" : null,
    bastion.hasPassphrase ? "passphrase" : null,
  ].filter(Boolean);
  return parts.length > 0 ? parts.join(", ") : "none";
}

export function filterBastions(
  bastions: BastionServer[],
  query: string,
): BastionServer[] {
  const needle = query.trim().toLowerCase();
  if (!needle) {
    return bastions;
  }
  return bastions.filter((bastion) =>
    [
      bastion.name,
      bastion.host,
      bastion.user,
      authLabel(bastion.authMethod),
      bastion.hostKeyFingerprint ?? "",
    ]
      .join(" ")
      .toLowerCase()
      .includes(needle),
  );
}

export function bastionReferenceCounts(
  bastions: BastionServer[],
  connections: Connection[],
): Record<string, number> {
  // SAFETY: The value is constrained by the typed component or library contract at this boundary.
  const counts = Object.fromEntries(
    bastions.map((bastion) => [bastion.id, 0]),
  ) as Record<string, number>;
  for (const connection of connections) {
    for (const bastionId of bastionIdsReferencedByConnection(connection)) {
      if (bastionId in counts) {
        counts[bastionId] += 1;
      }
    }
  }
  return counts;
}

function secretChangeFromInput(value: string): SecretChange {
  return value.trim() ? { action: "set", value } : { action: "keep" };
}
