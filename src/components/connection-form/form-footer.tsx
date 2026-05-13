/**
 * Sticky footer: test-status banners + Test Connection button (new
 * mode only) + the primary submit + Cancel + credential-storage hint.
 * Split so the parent's JSX is just structure, not chrome.
 */

import { IconShieldLock } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import type { CredentialStorageMode } from "@/lib/store";

import type { Mode } from "./form-utils";
import type { ConnectionFormApi, TestStatus } from "./use-connection-form";

type CredentialMode = CredentialStorageMode | null | undefined;

interface FormFooterProps {
  form: ConnectionFormApi;
  formMode: Mode;
  testStatus: TestStatus;
  credentialMode: CredentialMode;
  onTest: () => void | Promise<void>;
  onCancel?: () => void;
  handleCancel: () => void;
}

export function FormFooter({
  form,
  formMode,
  testStatus,
  credentialMode,
  onTest,
  onCancel,
  handleCancel,
}: FormFooterProps) {
  return (
    <div className="flex flex-col gap-2 border-t border-border-subtle bg-surface-window p-4">
      {testStatus.state === "success" ? (
        <div className="rounded-md border border-accent-green/30 bg-accent-green/10 px-3 py-2 text-[0.6875rem] text-accent-green-hover">
          Connected in {testStatus.latencyMs} ms.
        </div>
      ) : null}
      {testStatus.state === "error" ? (
        <div
          role="alert"
          className="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 font-mono text-[0.6875rem] text-danger"
        >
          {testStatus.error}
        </div>
      ) : null}
      <div className="flex gap-2">
        {formMode === "new" ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="flex-1"
            disabled={testStatus.state === "running"}
            onClick={() => {
              void onTest();
            }}
          >
            {testStatus.state === "running" ? "Testing…" : "Test Connection"}
          </Button>
        ) : null}
        <Button
          type="submit"
          size="sm"
          className="flex-1"
          disabled={form.state.isSubmitting || !form.state.isValid}
        >
          {submitLabel(form.state.isSubmitting, formMode)}
        </Button>
      </div>
      {formMode === "new" ? (
        <CredentialHint credentialMode={credentialMode} />
      ) : null}
      {onCancel ? (
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="self-start text-text-muted"
          onClick={handleCancel}
        >
          Cancel
        </Button>
      ) : null}
    </div>
  );
}

function submitLabel(isSubmitting: boolean, formMode: Mode): string {
  if (isSubmitting) return "Saving…";
  return formMode === "edit" ? "Save changes" : "Create Connection";
}

function CredentialHint({
  credentialMode,
}: {
  credentialMode: CredentialMode;
}) {
  return (
    <div className="flex items-center gap-1.5 text-[0.6875rem] text-text-muted">
      <IconShieldLock className="size-3 text-accent-green" />
      {credentialHintText(credentialMode)}
    </div>
  );
}

function credentialHintText(credentialMode: CredentialMode): string {
  if (credentialMode === "plain-sqlite") {
    return "Credentials are stored in the local SQLite database without encryption.";
  }
  if (credentialMode === "keychain") {
    return "Credentials are stored with the OS keychain.";
  }
  return "Credentials are encrypted in the local SQLite database.";
}
