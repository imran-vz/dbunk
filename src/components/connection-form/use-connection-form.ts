/**
 * Owns every piece of mutable state for `<ConnectionForm>` so the
 * component file is pure JSX. The hook returns the form instance, the
 * engine handler (mode-aware: edit locks engine), cancel handler,
 * test-status state for the "Test Connection" button (both modes), and a
 * couple of password-visibility/advanced-section toggles. Submission
 * builds the `StoredConnection` via the pure helpers in `form-utils`.
 */

import { useForm } from "@tanstack/react-form";
import { useEffect, useState } from "react";
import * as z from "zod";

import { connectionFormPolicy, validateConnection } from "@/lib/engine-policy";
import { type Connection, type DatabaseEngine, useAppStore } from "@/lib/store";
import type { ConnectionDiagnosis } from "@/lib/store/types";

import {
  buildConnectionFromForm,
  buildStoredConnectionFromForm,
  connectionSchema,
  defaultValuesFromConnection,
  EMPTY_NEW_DEFAULTS,
  type Mode,
} from "./form-utils";

export type TestStatus =
  | { state: "idle" }
  | { state: "running" }
  | { state: "done"; report: ConnectionDiagnosis }
  | { state: "error"; error: string };

interface UseConnectionFormArgs {
  mode: Mode;
  connection?: Connection;
  onSaved?: () => void;
  onCancel?: () => void;
}

export function useConnectionForm({
  mode,
  connection,
  onSaved,
  onCancel,
}: UseConnectionFormArgs) {
  const initial =
    mode === "edit" && connection
      ? defaultValuesFromConnection(connection)
      : EMPTY_NEW_DEFAULTS;

  const [selectedEngine, setSelectedEngine] = useState<DatabaseEngine>(
    initial.engine,
  );
  const [showPassword, setShowPassword] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [testStatus, setTestStatus] = useState<TestStatus>({ state: "idle" });

  const addConnection = useAppStore((state) => state.addConnection);
  const updateConnection = useAppStore((state) => state.updateConnection);
  const diagnoseConnection = useAppStore((state) => state.diagnoseConnection);
  const credentialMode = useAppStore(
    (state) => state.appSettings?.credentialStorageMode,
  );

  const formMode = mode;

  const form = useForm({
    defaultValues: initial,
    onSubmit: async ({ value }) => {
      if (formMode === "edit") {
        if (!connection) return;
        const updated = buildConnectionFromForm(value, connection.id, {
          status: connection.status,
          latency: connection.latency,
          errorMessage: connection.errorMessage,
          lastActivityAt: connection.lastActivityAt,
        });
        const outcome = await updateConnection(updated);
        if (outcome !== "saved") {
          if (outcome === "failed")
            setTestStatus({
              state: "error",
              error: "Connection could not be saved. Your edits are preserved.",
            });
          return;
        }
      } else {
        const created = buildConnectionFromForm(value, crypto.randomUUID(), {
          status: "Disconnected",
          latency: "--",
        });
        await addConnection(created);
      }
      onSaved?.();
    },
    validators: {
      onChange: connectionSchema.superRefine((value, ctx) => {
        const issues = validateConnection(
          connectionFormPolicy(value.engine),
          value,
          formMode,
        );
        for (const issue of issues) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            message: issue.message,
            path: [issue.path],
          });
        }
      }),
    },
  });

  // Edit mode hydrates a different connection if the prop changes.
  /* oxlint-disable react-hooks/exhaustive-deps -- form ref is stable */
  useEffect(() => {
    if (formMode === "edit" && connection) {
      const next = defaultValuesFromConnection(connection);
      form.reset(next);
      setSelectedEngine(next.engine);
    }
  }, [connection, formMode]);
  /* oxlint-enable react-hooks/exhaustive-deps */

  const handleEngineChange = (engine: DatabaseEngine) => {
    if (formMode === "edit") return; // picker is disabled in edit mode
    form.setFieldValue("engine", engine);
    setSelectedEngine(engine);
    // Engine-specific fields reset to the new engine's neutral
    // defaults. Common fields (name/host/port/user/password/role)
    // survive the switch — the user keeps what they typed.
    resetEngineSpecificDefaults(form, engine);
  };

  const handleCancel = () => {
    if (formMode === "new") {
      form.reset(EMPTY_NEW_DEFAULTS);
      setSelectedEngine("PostgreSQL");
    }
    setShowPassword(false);
    setTestStatus({ state: "idle" });
    onCancel?.();
  };

  const runTestConnection = async () => {
    const value = form.state.values;
    setTestStatus({ state: "running" });
    // Edit mode holds a blanked password; the backend substitutes the
    // saved credential when told which record to read (ADR-0025). A
    // password typed into the field always wins.
    const hydrateFrom =
      formMode === "edit" && connection && !value.password
        ? connection.id
        : undefined;
    const result = await diagnoseConnection(
      buildStoredConnectionFromForm(value, "test-connection"),
      hydrateFrom,
    );
    if (result.ok) {
      setTestStatus({ state: "done", report: result.report });
    } else {
      setTestStatus({ state: "error", error: result.error });
    }
  };

  return {
    form,
    formMode,
    selectedEngine,
    showPassword,
    setShowPassword,
    advancedOpen,
    setAdvancedOpen,
    testStatus,
    credentialMode,
    handleEngineChange,
    handleCancel,
    runTestConnection,
  };
}

type ConnectionForm = ReturnType<typeof useConnectionForm>["form"];

/**
 * Exported for its test only — the hook itself needs the app store.
 */
export function resetEngineSpecificDefaults(
  form: ConnectionForm,
  engine: DatabaseEngine,
): void {
  // Driver options (ADR-0013) and TLS fields (ADR-0025) are PG-only.
  // Switching away clears them so values typed against Postgres don't
  // linger invisibly in form state and reappear if the user switches
  // back mid-edit.
  if (engine !== "PostgreSQL") {
    resetDriverOptions(form);
    resetTlsFields(form);
  }
  switch (engine) {
    case "PostgreSQL":
    case "MySQL":
      form.setFieldValue("ssl", true);
      return;
    case "SQLite":
      // No engine-specific fields. SQLite's `database` field is
      // a file path; the user keeps whatever they typed.
      return;
    case "ClickHouse":
      form.setFieldValue("useHttps", false);
      form.setFieldValue("urlPath", "");
      return;
    case "Redis":
      form.setFieldValue("dbNumber", 0);
      form.setFieldValue("useTls", false);
      form.setFieldValue("verifyTlsCert", true);
      return;
  }
}

function resetDriverOptions(form: ConnectionForm): void {
  form.setFieldValue("statementTimeoutMs", undefined);
  form.setFieldValue("idleInTransactionTimeoutMs", undefined);
  form.setFieldValue("connectTimeoutMs", undefined);
  form.setFieldValue("keepaliveSeconds", undefined);
  form.setFieldValue("defaultSearchPath", "");
  form.setFieldValue("defaultRole", "");
}

function resetTlsFields(form: ConnectionForm): void {
  form.setFieldValue("tlsMode", "prefer");
  form.setFieldValue("tlsRootCertPath", "");
  form.setFieldValue("tlsClientCertPath", "");
  form.setFieldValue("tlsClientKeyPath", "");
  form.setFieldValue("tlsServerName", "");
}

export type ConnectionFormApi = ConnectionForm;
