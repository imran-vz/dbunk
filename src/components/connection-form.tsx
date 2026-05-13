/**
 * Unified ConnectionForm — the single form component for both creating
 * a new Connection and editing an existing one. The `mode` prop forks
 * the user-visible behavior:
 *
 * - `mode: "new"` — fresh connection. Engine picker is editable;
 *   common fields (name/host/port/user/password) carry across
 *   engine-switches; the Test Connection button + credential-storage
 *   hint render in the footer.
 * - `mode: "edit"` — existing connection. Engine picker is disabled
 *   (changing engine on a tagged-union record is a delete-and-recreate,
 *   not an edit). Footer is just Cancel + Save changes.
 *
 * Field rendering is policy-driven (`connectionFormPolicy(engine)`):
 * the form switches on `policy.kind` to decide which engine-specific
 * fields appear (`ssl` for host-auth, `useHttps`/`urlPath` for
 * clickhouse-http, `dbNumber`/`useTls`/`verifyTlsCert` for redis,
 * nothing for file). Validation is delegated to `validateConnection`
 * — one shared validator, mode-aware password rule.
 *
 * The body splits into:
 *   - `useConnectionForm`        — every piece of mutable state
 *   - `common-fields.tsx`        — name/engine/host/port/database/etc.
 *   - `<Engine>Fields` modules   — engine-specific toggles
 *   - `form-footer.tsx`          — submit row + credential hint
 *
 * See ADR-0012 for the unified-form decision and ADR-0010 for the
 * `ssl` wiring this form makes user-editable.
 */

import { IconChevronDown, IconChevronRight } from "@tabler/icons-react";

import { ClickHouseFields } from "@/components/connection-form/clickhouse-fields";
import {
  DatabaseField,
  EnginePickerField,
  HostPortRow,
  NameField,
  PasswordField,
  RoleField,
  UserField,
} from "@/components/connection-form/common-fields";
import { FormFooter } from "@/components/connection-form/form-footer";
import { MySqlFields } from "@/components/connection-form/mysql-fields";
import { PgFields } from "@/components/connection-form/pg-fields";
import {
  RedisAdvancedFields,
  RedisDbNumberField,
} from "@/components/connection-form/redis-fields";
import { SqliteFields } from "@/components/connection-form/sqlite-fields";
import {
  type ConnectionFormApi,
  useConnectionForm,
} from "@/components/connection-form/use-connection-form";
import { connectionFormPolicy } from "@/lib/engine-policy";
import type { Connection } from "@/lib/store";

export type { Mode } from "@/components/connection-form/form-utils";

export interface ConnectionFormProps {
  mode: "new" | "edit";
  /** Required when `mode === "edit"`; ignored otherwise. */
  connection?: Connection;
  /** Called after a successful save. Caller controls dialog dismiss. */
  onSaved?: () => void;
  /** Called when the user clicks Cancel. Caller controls dialog dismiss. */
  onCancel?: () => void;
}

export function ConnectionForm({
  mode,
  connection,
  onSaved,
  onCancel,
}: ConnectionFormProps) {
  const ctrl = useConnectionForm({ mode, connection, onSaved, onCancel });
  const {
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
  } = ctrl;

  const policy = connectionFormPolicy(selectedEngine);
  const isSqlite = policy.kind === "file";
  const isRedis = policy.kind === "redis";

  return (
    <form
      className="flex h-full min-h-0 flex-col"
      onSubmit={(event) => {
        event.preventDefault();
        event.stopPropagation();
        form.handleSubmit();
      }}
    >
      <div className="flex flex-1 min-h-0 flex-col gap-4 overflow-auto p-4">
        <NameField form={form} />
        <EnginePickerField
          form={form}
          formMode={formMode}
          onEngineChange={handleEngineChange}
        />

        {!isSqlite ? <HostPortRow form={form} policy={policy} /> : null}

        {isRedis ? (
          <RedisDbNumberField form={form} policy={policy} />
        ) : (
          <DatabaseField form={form} isSqlite={isSqlite} />
        )}

        {!isSqlite ? (
          <HostAuthSection
            form={form}
            engineKind={policy.kind}
            formMode={formMode}
            isRedis={isRedis}
            showPassword={showPassword}
            onTogglePassword={() => setShowPassword((prev) => !prev)}
            advancedOpen={advancedOpen}
            onToggleAdvanced={() => setAdvancedOpen((prev) => !prev)}
            policy={policy}
          />
        ) : (
          <SqliteFields />
        )}
      </div>

      <FormFooter
        form={form}
        formMode={formMode}
        testStatus={testStatus}
        credentialMode={credentialMode}
        onTest={runTestConnection}
        onCancel={onCancel}
        handleCancel={handleCancel}
      />
    </form>
  );
}

interface HostAuthSectionProps {
  form: ConnectionFormApi;
  engineKind: ReturnType<typeof connectionFormPolicy>["kind"];
  formMode: "new" | "edit";
  isRedis: boolean;
  showPassword: boolean;
  onTogglePassword: () => void;
  advancedOpen: boolean;
  onToggleAdvanced: () => void;
  policy: ReturnType<typeof connectionFormPolicy>;
}

/**
 * The shared user/password + SSL + advanced-options block rendered for
 * every non-SQLite engine. Splitting it out of `<ConnectionForm>` cuts
 * its cyclomatic count below fallow's threshold without changing the
 * rendered tree.
 */
function HostAuthSection({
  form,
  engineKind,
  formMode,
  isRedis,
  showPassword,
  onTogglePassword,
  advancedOpen,
  onToggleAdvanced,
  policy,
}: HostAuthSectionProps) {
  const isClickHouse = engineKind === "clickhouse-http";
  const showSslToggle =
    policy.kind === "host-auth" ? policy.showSslToggle : false;
  const sslEngine = form.state.values.engine;

  return (
    <>
      <UserField form={form} isRedis={isRedis} />
      <PasswordField
        form={form}
        formMode={formMode}
        isRedis={isRedis}
        showPassword={showPassword}
        onTogglePassword={onTogglePassword}
      />
      {showSslToggle ? (
        <PgMySqlSslToggle form={form} engine={sslEngine} />
      ) : null}

      <AdvancedToggle open={advancedOpen} onToggle={onToggleAdvanced} />
      {advancedOpen ? (
        <>
          <RoleField form={form} />
          {isClickHouse ? <ClickHouseFields form={form} /> : null}
          {isRedis ? <RedisAdvancedFields form={form} /> : null}
        </>
      ) : null}
    </>
  );
}

function PgMySqlSslToggle({
  form,
  engine,
}: {
  form: ConnectionFormApi;
  engine: string;
}) {
  // PG + MySQL share the same SSL surface; named components per engine
  // keep parity with `<ClickHouseFields>` / `<RedisFields>` / `<SqliteFields>`.
  return engine === "MySQL" ? (
    <MySqlFields form={form} />
  ) : (
    <PgFields form={form} />
  );
}

function AdvancedToggle({
  open,
  onToggle,
}: {
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      className="flex items-center gap-1 self-start text-xs font-medium text-text-secondary hover:text-foreground"
    >
      {open ? (
        <IconChevronDown className="size-3.5" />
      ) : (
        <IconChevronRight className="size-3.5" />
      )}
      Advanced Options
    </button>
  );
}
