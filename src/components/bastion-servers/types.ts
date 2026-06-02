import type { SaveBastionServerInput } from "@/lib/store";

export type TestState =
  | { state: "idle" }
  | { state: "running"; id: string }
  | { state: "success"; id: string; latencyMs: number }
  | { state: "error"; id: string; error: string };

export type HostKeyResetState = { id: string; confirmHost: string } | null;

export type BastionDraft = Omit<
  SaveBastionServerInput,
  "password" | "privateKeyContent" | "passphrase"
> & {
  password: string;
  privateKeyContent: string;
  passphrase: string;
  clearPassphrase: boolean;
};
