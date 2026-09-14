import { IconArrowRight } from "@tabler/icons-react";
import { useId } from "react";

import { EnvironmentBadge } from "@/components/environment-badge";
import { Input } from "@/components/ui/input";
import type { SchemaCompareEndpoint } from "@/lib/pg-schema-compare/protocol";
import type { SchemaCompareSide } from "@/lib/pg-schema-compare/reader";
import { ENVIRONMENT_META } from "@/lib/safety-policy";
import type { Connection } from "@/lib/store";

import type { EndpointDraft } from "./use-compare-form";

// Matches the native-tool select in pg-tool-jobs: strong border, one crisp ring.
const selectClass =
  "h-(--control-h) min-w-0 rounded-sm border border-border-strong bg-surface-input px-2 text-xs outline-none focus-visible:ring-1 focus-visible:ring-accent disabled:opacity-50";

/** Both identities stay visible: connection name, environment and schema. */
export function EndpointIdentity({
  endpoint,
  connections,
}: {
  endpoint: SchemaCompareEndpoint;
  connections: Connection[];
}) {
  const connection = connections.find((c) => c.id === endpoint.connectionId);
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5">
      <span className="truncate">
        {connection?.name ?? "Removed connection"}
      </span>
      <EnvironmentBadge environment={connection?.environment} short />
      <span className="font-mono text-text-secondary">{endpoint.schema}</span>
    </span>
  );
}

function EndpointFields({
  side,
  label,
  draft,
  connections,
  schemas,
  disabled,
  onChange,
}: {
  side: SchemaCompareSide;
  label: string;
  draft: EndpointDraft;
  connections: Connection[];
  schemas: string[];
  disabled: boolean;
  onChange: (side: SchemaCompareSide, next: Partial<EndpointDraft>) => void;
}) {
  const listId = useId();
  const selected = connections.find((c) => c.id === draft.connectionId);
  const missing = draft.connectionId !== "" && !selected;
  return (
    <fieldset
      disabled={disabled}
      className="flex min-w-0 flex-wrap items-center gap-1.5"
    >
      <legend className="sr-only">{label} endpoint</legend>
      <span className="w-12 text-2xs font-semibold tracking-wide text-text-muted uppercase">
        {label}
      </span>
      <select
        aria-label={`${label} connection`}
        className={selectClass}
        value={missing ? "" : draft.connectionId}
        onChange={(event) =>
          onChange(side, { connectionId: event.target.value })
        }
      >
        <option value="">
          {missing ? "Removed connection" : "Connection"}
        </option>
        {connections.map((connection) => (
          <option key={connection.id} value={connection.id}>
            {connection.name} · {connection.database}
            {connection.environment && connection.environment !== "development"
              ? ` · ${ENVIRONMENT_META[connection.environment].shortLabel}`
              : ""}
          </option>
        ))}
      </select>
      <EnvironmentBadge environment={selected?.environment} short />
      <Input
        aria-label={`${label} schema`}
        list={schemas.length ? listId : undefined}
        placeholder="schema"
        autoComplete="off"
        spellCheck={false}
        value={draft.schema}
        onChange={(event) => onChange(side, { schema: event.target.value })}
        className="w-36 font-mono text-xs"
      />
      {schemas.length ? (
        <datalist id={listId}>
          {schemas.map((schema) => (
            <option key={schema}>{schema}</option>
          ))}
        </datalist>
      ) : null}
    </fieldset>
  );
}

export function EndpointForm({
  source,
  target,
  connections,
  schemasFor,
  disabled,
  onChange,
  action,
}: {
  source: EndpointDraft;
  target: EndpointDraft;
  connections: Connection[];
  schemasFor: (connectionId: string) => string[];
  disabled: boolean;
  onChange: (side: SchemaCompareSide, next: Partial<EndpointDraft>) => void;
  action: React.ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-border-subtle px-3 py-1.5">
      <EndpointFields
        side="source"
        label="Source"
        draft={source}
        connections={connections}
        schemas={schemasFor(source.connectionId)}
        disabled={disabled}
        onChange={onChange}
      />
      <IconArrowRight
        aria-hidden="true"
        className="size-4 shrink-0 text-text-muted"
      />
      <EndpointFields
        side="target"
        label="Target"
        draft={target}
        connections={connections}
        schemas={schemasFor(target.connectionId)}
        disabled={disabled}
        onChange={onChange}
      />
      <div className="ml-auto flex items-center gap-1.5">{action}</div>
    </div>
  );
}
