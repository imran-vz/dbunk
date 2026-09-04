import { IconPlus, IconTrash } from "@tabler/icons-react";
import { useMemo, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import type {
  PgGrantee,
  PgPolicyCommand,
  PgPrivilege,
  PolicyInfo,
  PrivilegeInfo,
  RowSecurityInfo,
  TriggerInfo,
} from "@/lib/store/types";
import {
  asPgPolicyCommand,
  asPgTriggerEvent,
  buildGrantOp,
  buildPolicyOps,
  buildRevokeOp,
  buildTriggerOps,
  PG_POLICY_COMMAND_OPTIONS,
  PG_TRIGGER_EVENT_OPTIONS,
} from "@/lib/structure-changes";

import { EmptyRow, MiniSelect, type PgStructureOps, Section } from "./shared";

const RELATION_PRIVILEGES: readonly PgPrivilege[] = [
  "select",
  "insert",
  "update",
  "delete",
  "truncate",
  "references",
  "trigger",
  "maintain",
];
// MAINTAIN was added in PostgreSQL 17. Keep it recognizable for introspected
// PG17 grants, but do not offer it without a server-version capability signal.
const GRANTABLE_RELATION_PRIVILEGES = RELATION_PRIVILEGES.filter(
  (privilege) => privilege !== "maintain",
);

/** Catalog identity retained for an overload-safe trigger-function picker.
 * A null return type means its description has not been loaded yet. */
export type TriggerFunctionOption = {
  schema: string;
  name: string;
  identityArgs: string;
  returns: string | null;
};

const triggerFunctionLabel = (fn: TriggerFunctionOption): string =>
  `${fn.schema}.${fn.name}(${fn.identityArgs}) → ${fn.returns ?? "unknown return"}`;

const relationPrivilege = (value: string): PgPrivilege | null => {
  const normalized = value.toLowerCase();
  return (
    RELATION_PRIVILEGES.find((privilege) => privilege === normalized) ?? null
  );
};

type PolicyGranteeDraft = {
  id: string;
  grantee: PgGrantee;
  sourceName: string | null;
  resolvedByUser: boolean;
};

const policyGranteeNeedsResolution = (draft: PolicyGranteeDraft): boolean =>
  draft.sourceName === "public" && !draft.resolvedByUser;

export function TriggersSection({
  triggers,
  supported,
  pg,
  functions,
}: {
  triggers: TriggerInfo[];
  supported: boolean;
  pg: PgStructureOps | null;
  functions: TriggerFunctionOption[];
}) {
  const [adding, setAdding] = useState(false);
  if (!supported) return null;
  return (
    <Section
      title="Triggers"
      testId="structure-triggers"
      action={
        pg ? (
          <Button
            size="sm"
            variant="outline"
            onClick={() => setAdding((value) => !value)}
          >
            <IconPlus /> New trigger
          </Button>
        ) : null
      }
    >
      {pg && adding ? (
        <TriggerForm
          pg={pg}
          functions={functions}
          onDone={() => setAdding(false)}
        />
      ) : null}
      {triggers.length === 0 ? (
        <EmptyRow>No triggers defined.</EmptyRow>
      ) : (
        <div className="divide-y divide-border-subtle">
          {triggers.map((trigger) => (
            <TriggerRow key={trigger.name} trigger={trigger} pg={pg} />
          ))}
        </div>
      )}
    </Section>
  );
}

function TriggerRow({
  trigger,
  pg,
}: {
  trigger: TriggerInfo;
  pg: PgStructureOps | null;
}) {
  const [cascade, setCascade] = useState(false);
  return (
    <div className="flex flex-wrap items-center gap-2 px-3 py-2 text-xs">
      <span className="font-mono text-foreground">{trigger.name}</span>
      <Badge variant="outline">{trigger.timing}</Badge>
      <span className="text-text-muted">
        {trigger.events.join(" OR ")} · {trigger.level}
      </span>
      <span className="font-mono text-text-muted">
        {trigger.functionSchema}.{trigger.functionName}
      </span>
      <Badge
        variant={trigger.enabled === "disabled" ? "destructive" : "secondary"}
      >
        {trigger.enabled}
      </Badge>
      {pg ? (
        <div className="ml-auto flex items-center gap-1">
          <Button
            size="sm"
            variant="ghost"
            onClick={() =>
              pg.queueOp({
                op: "setTriggerEnabled",
                schema: pg.schema,
                table: pg.table,
                name: trigger.name,
                mode: trigger.enabled === "disabled" ? "enable" : "disable",
              })
            }
          >
            {trigger.enabled === "disabled" ? "Enable" : "Disable"}
          </Button>
          <label className="flex items-center gap-1 text-2xs text-text-muted">
            <input
              type="checkbox"
              checked={cascade}
              onChange={(event) => setCascade(event.target.checked)}
            />
            cascade
          </label>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={`Drop trigger ${trigger.name}`}
            onClick={() =>
              pg.queueOp({
                op: "dropTrigger",
                schema: pg.schema,
                table: pg.table,
                name: trigger.name,
                cascade,
              })
            }
          >
            <IconTrash />
          </Button>
        </div>
      ) : null}
    </div>
  );
}

function TriggerForm({
  pg,
  functions,
  onDone,
}: {
  pg: PgStructureOps;
  functions: TriggerFunctionOption[];
  onDone: () => void;
}) {
  const [name, setName] = useState("");
  const [timing, setTiming] = useState<"before" | "after">("before");
  const [events, setEvents] = useState(new Set(["update"]));
  const [level, setLevel] = useState<"row" | "statement">("row");
  const [functionName, setFunctionName] = useState("");
  const [functionSchema, setFunctionSchema] = useState(pg.schema);
  const [when, setWhen] = useState("");
  const [createFunction, setCreateFunction] = useState(false);
  const [body, setBody] = useState("BEGIN\n  RETURN NEW;\nEND;");
  const selectedEvents = [...events].map(asPgTriggerEvent);
  const functionChoices = useMemo(() => {
    const choices = new Map<
      string,
      { label: string; option: TriggerFunctionOption }
    >();
    for (const option of functions) {
      if (
        option.identityArgs.trim() !== "" ||
        option.returns?.trim().toLowerCase() !== "trigger"
      )
        continue;
      const label = triggerFunctionLabel(option);
      choices.set(label, { label, option });
    }
    return [...choices.values()];
  }, [functions]);
  const selectedFunction = functionChoices.find(
    (choice) => choice.label === functionName,
  )?.option;
  return (
    <div
      data-testid="structure-trigger-form"
      className="space-y-2 border-b border-border-subtle p-3"
    >
      <div className="grid grid-cols-4 gap-2">
        <Input
          aria-label="Trigger name"
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="trigger name"
        />
        <MiniSelect
          ariaLabel="Trigger timing"
          value={timing}
          options={[
            { value: "before", label: "BEFORE" },
            { value: "after", label: "AFTER" },
          ]}
          onChange={(value) =>
            setTiming(value === "after" ? "after" : "before")
          }
        />
        <Input
          aria-label="Trigger function schema"
          value={functionSchema}
          onChange={(event) => setFunctionSchema(event.target.value)}
        />
        <Input
          aria-label="Trigger function"
          list="trigger-function-options"
          value={functionName}
          onChange={(event) => setFunctionName(event.target.value)}
          placeholder="function"
        />
        <datalist id="trigger-function-options">
          {functionChoices.map(({ label, option }) => (
            <option
              key={`${option.schema}\u0000${option.name}\u0000${option.identityArgs}`}
              value={label}
            >
              {label}
            </option>
          ))}
        </datalist>
      </div>
      <div className="flex flex-wrap gap-3 text-xs">
        {PG_TRIGGER_EVENT_OPTIONS.map((event) => (
          <label key={event.value} className="flex items-center gap-1">
            <input
              type="checkbox"
              checked={events.has(event.value)}
              onChange={() =>
                setEvents((current) => {
                  const next = new Set(current);
                  if (next.has(event.value)) next.delete(event.value);
                  else next.add(event.value);
                  return next;
                })
              }
            />
            {event.label}
          </label>
        ))}
        <MiniSelect
          ariaLabel="Trigger level"
          value={level}
          options={[
            { value: "row", label: "ROW" },
            { value: "statement", label: "STATEMENT" },
          ]}
          onChange={(value) =>
            setLevel(value === "statement" ? "statement" : "row")
          }
        />
      </div>
      <Input
        aria-label="Trigger when"
        value={when}
        onChange={(event) => setWhen(event.target.value)}
        placeholder="WHEN condition (optional)"
      />
      <label className="flex items-center gap-1.5 text-xs">
        <input
          type="checkbox"
          checked={createFunction}
          onChange={(event) => setCreateFunction(event.target.checked)}
        />
        Create function inline
      </label>
      {createFunction ? (
        <Textarea
          aria-label="Trigger function body"
          value={body}
          onChange={(event) => setBody(event.target.value)}
          className="min-h-24 font-mono"
        />
      ) : null}
      <div className="flex justify-end gap-1">
        <Button size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button
          size="sm"
          disabled={
            !name.trim() ||
            !functionName.trim() ||
            selectedEvents.length === 0 ||
            (events.has("truncate") && level === "row")
          }
          onClick={() => {
            for (const op of buildTriggerOps({
              schema: pg.schema,
              table: pg.table,
              name,
              timing,
              events: selectedEvents,
              forEach: level,
              when,
              functionSchema: selectedFunction?.schema ?? functionSchema,
              functionName: selectedFunction?.name ?? functionName,
              createFunction: createFunction
                ? { language: "plpgsql", body }
                : undefined,
            }))
              pg.queueOp(op);
            onDone();
          }}
        >
          Queue trigger
        </Button>
      </div>
    </div>
  );
}

export function RowLevelSecuritySection({
  rowSecurity,
  policies,
  supported,
  pg,
  roles,
}: {
  rowSecurity: RowSecurityInfo | null;
  policies: PolicyInfo[];
  supported: boolean;
  pg: PgStructureOps | null;
  roles: string[];
}) {
  const [editing, setEditing] = useState<PolicyInfo | null | "new">(null);
  if (!supported) return null;
  const enabled = rowSecurity?.enabled ?? false;
  const forced = enabled && (rowSecurity?.forced ?? false);
  return (
    <Section
      title="Row-level security"
      testId="structure-row-security"
      action={
        pg ? (
          <Button size="sm" variant="outline" onClick={() => setEditing("new")}>
            <IconPlus /> New policy
          </Button>
        ) : null
      }
    >
      {pg ? (
        <RowLevelSecurityControls
          key={`${pg.schema}\u0000${pg.table}\u0000${enabled}:${forced}`}
          enabled={enabled}
          forced={forced}
          pg={pg}
        />
      ) : null}
      {editing && pg ? (
        <PolicyForm
          key={editing === "new" ? "create" : `edit:${editing.name}`}
          pg={pg}
          roles={roles}
          policy={editing === "new" ? null : editing}
          onDone={() => setEditing(null)}
        />
      ) : null}
      {policies.length === 0 ? (
        <EmptyRow>No policies defined.</EmptyRow>
      ) : (
        <div className="divide-y divide-border-subtle">
          {policies.map((policy) => (
            <div key={policy.name} className="space-y-1 px-3 py-2 text-xs">
              <div className="flex items-center gap-2">
                <span className="font-mono">{policy.name}</span>
                <Badge variant="outline">{policy.command}</Badge>
                <span className="text-text-muted">
                  {policy.permissive ? "permissive" : "restrictive"} ·{" "}
                  {policy.roles.join(", ")}
                </span>
                {pg ? (
                  <div className="ml-auto flex gap-1">
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => setEditing(policy)}
                    >
                      Edit
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label={`Drop policy ${policy.name}`}
                      onClick={() =>
                        pg.queueOp({
                          op: "dropPolicy",
                          schema: pg.schema,
                          table: pg.table,
                          name: policy.name,
                        })
                      }
                    >
                      <IconTrash />
                    </Button>
                  </div>
                ) : null}
              </div>
              {policy.using ? (
                <code className="block text-text-muted">
                  USING ({policy.using})
                </code>
              ) : null}
              {policy.withCheck ? (
                <code className="block text-text-muted">
                  WITH CHECK ({policy.withCheck})
                </code>
              ) : null}
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}

function RowLevelSecurityControls({
  enabled: initialEnabled,
  forced: initialForced,
  pg,
}: {
  enabled: boolean;
  forced: boolean;
  pg: PgStructureOps;
}) {
  const [enabled, setEnabled] = useState(initialEnabled);
  const [forced, setForced] = useState(initialForced);

  return (
    <div className="flex items-center gap-3 border-b border-border-subtle px-3 py-2 text-xs">
      <span>RLS</span>
      <label className="flex items-center gap-1.5">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(event) => {
            setEnabled(event.target.checked);
            if (!event.target.checked) setForced(false);
          }}
        />
        Enabled
      </label>
      <label className="flex items-center gap-1.5">
        <input
          type="checkbox"
          checked={forced}
          disabled={!enabled}
          onChange={(event) => setForced(event.target.checked)}
        />
        Forced
      </label>
      <Button
        size="sm"
        variant="ghost"
        disabled={enabled === initialEnabled && forced === initialForced}
        onClick={() =>
          pg.queueOp({
            op: "setRowLevelSecurity",
            schema: pg.schema,
            table: pg.table,
            enabled,
            force: forced,
          })
        }
      >
        Queue RLS
      </Button>
    </div>
  );
}

function PolicyForm({
  pg,
  roles,
  policy,
  onDone,
}: {
  pg: PgStructureOps;
  roles: string[];
  policy: PolicyInfo | null;
  onDone: () => void;
}) {
  const [name, setName] = useState(policy?.name ?? "");
  const [command, setCommand] = useState<PgPolicyCommand>(() =>
    asPgPolicyCommand(policy?.command ?? "all"),
  );
  const [grantees, setGrantees] = useState<PolicyGranteeDraft[]>(() =>
    policy?.roles.length
      ? policy.roles.map((role) => ({
          id: crypto.randomUUID(),
          grantee:
            role === "public"
              ? { kind: "public" }
              : { kind: "role", name: role },
          sourceName: role,
          resolvedByUser: role !== "public",
        }))
      : [
          {
            id: crypto.randomUUID(),
            grantee: { kind: "public" },
            sourceName: null,
            resolvedByUser: true,
          },
        ],
  );
  const [permissive, setPermissive] = useState(policy?.permissive ?? true);
  const [using, setUsing] = useState(policy?.using ?? "");
  const [withCheck, setWithCheck] = useState(policy?.withCheck ?? "");
  const policyAllowsUsing = command !== "insert";
  const policyAllowsWithCheck = command !== "select" && command !== "delete";
  return (
    <div
      data-testid="structure-policy-form"
      className="space-y-2 border-b border-border-subtle p-3"
    >
      <div className="grid grid-cols-3 gap-2">
        <Input
          aria-label="Policy name"
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="policy name"
        />
        <MiniSelect
          ariaLabel="Policy command"
          value={command}
          options={PG_POLICY_COMMAND_OPTIONS}
          onChange={(value) => setCommand(asPgPolicyCommand(value))}
        />
      </div>
      <div className="space-y-1">
        {grantees.map((draft, index) => (
          <div
            key={draft.id}
            className="grid grid-cols-[1fr_auto] items-start gap-1"
          >
            <GranteeEditor
              label={`Policy role ${index + 1}`}
              grantee={draft.grantee}
              roles={roles}
              ambiguous={policyGranteeNeedsResolution(draft)}
              onChange={(grantee) => {
                setGrantees((current) =>
                  current.map((candidate) =>
                    candidate.id === draft.id
                      ? {
                          ...candidate,
                          grantee,
                          sourceName: null,
                          resolvedByUser: true,
                        }
                      : candidate,
                  ),
                );
              }}
            />
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label={`Remove policy role ${index + 1}`}
              onClick={() =>
                setGrantees((current) =>
                  current.filter((candidate) => candidate.id !== draft.id),
                )
              }
            >
              <IconTrash />
            </Button>
          </div>
        ))}
        <Button
          size="sm"
          variant="ghost"
          onClick={() =>
            setGrantees((current) => [
              ...current,
              {
                id: crypto.randomUUID(),
                grantee: { kind: "role", name: "" },
                sourceName: null,
                resolvedByUser: true,
              },
            ])
          }
        >
          <IconPlus /> Add role
        </Button>
      </div>
      <label className="flex items-center gap-1.5 text-xs">
        <input
          type="checkbox"
          checked={permissive}
          onChange={(event) => setPermissive(event.target.checked)}
        />
        Permissive
      </label>
      <Input
        aria-label="Policy using"
        disabled={!policyAllowsUsing}
        value={using}
        onChange={(event) => setUsing(event.target.value)}
        placeholder="USING expression"
      />
      <Input
        aria-label="Policy with check"
        disabled={!policyAllowsWithCheck}
        value={withCheck}
        onChange={(event) => setWithCheck(event.target.value)}
        placeholder="WITH CHECK expression"
      />
      <div className="flex justify-end gap-1">
        <Button size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button
          size="sm"
          disabled={
            !name.trim() ||
            grantees.length === 0 ||
            grantees.some(policyGranteeNeedsResolution) ||
            grantees.some(
              ({ grantee }) =>
                grantee.kind === "role" && grantee.name.trim() === "",
            )
          }
          onClick={() => {
            for (const op of buildPolicyOps({
              schema: pg.schema,
              table: pg.table,
              dropExisting: policy?.name,
              name,
              permissive,
              command,
              roles: grantees.map(({ grantee }) => grantee),
              using,
              withCheck,
            }))
              pg.queueOp(op);
            onDone();
          }}
        >
          Queue policy
        </Button>
      </div>
    </div>
  );
}

function GranteeEditor({
  label,
  grantee,
  roles,
  ambiguous = false,
  onChange,
}: {
  label: string;
  grantee: PgGrantee;
  roles: string[];
  ambiguous?: boolean;
  onChange: (grantee: PgGrantee) => void;
}) {
  const listId = `${label.toLowerCase().replaceAll(" ", "-")}-options`;
  return (
    <div className="space-y-1">
      <div className="grid grid-cols-[7rem_1fr] gap-1">
        <select
          aria-label={`${label} kind`}
          value={ambiguous ? "" : grantee.kind}
          onChange={(event) =>
            onChange(
              event.target.value === "public"
                ? { kind: "public" }
                : { kind: "role", name: "" },
            )
          }
          className="h-8 border border-border-subtle bg-surface-input px-2 text-xs"
        >
          {ambiguous ? <option value="">Choose target</option> : null}
          <option value="public">PUBLIC</option>
          <option value="role">Role</option>
        </select>
        {grantee.kind === "role" && !ambiguous ? (
          <>
            <Input
              aria-label={label}
              list={listId}
              value={grantee.name}
              onChange={(event) =>
                onChange({ kind: "role", name: event.target.value })
              }
              placeholder="app_reader"
            />
            <datalist id={listId}>
              {roles.map((role) => (
                <option key={role} value={role}>
                  {role}
                </option>
              ))}
            </datalist>
          </>
        ) : (
          <div className="flex h-8 items-center px-2 font-mono text-xs text-text-secondary">
            PUBLIC
          </div>
        )}
      </div>
      {ambiguous ? (
        <p className="text-2xs text-warning">
          Catalog data cannot prove whether this is PUBLIC or a quoted role.
          Choose the intended target.
        </p>
      ) : null}
    </div>
  );
}

export function PrivilegesSection({
  privileges,
  supported,
  pg,
  roles,
}: {
  privileges: PrivilegeInfo[];
  supported: boolean;
  pg: PgStructureOps | null;
  roles: string[];
}) {
  const [granting, setGranting] = useState(false);
  const grouped = useMemo(() => {
    const groups = new Map<string, Map<string, PrivilegeInfo>>();
    for (const privilege of privileges) {
      const entries = groups.get(privilege.grantee) ?? new Map();
      const privilegeKey = privilege.privilege.toUpperCase();
      const existing = entries.get(privilegeKey);
      entries.set(
        privilegeKey,
        existing
          ? {
              ...existing,
              grantable: existing.grantable || privilege.grantable,
            }
          : privilege,
      );
      groups.set(privilege.grantee, entries);
    }
    return new Map(
      [...groups].map(([grantee, entries]) => [grantee, [...entries.values()]]),
    );
  }, [privileges]);
  if (!supported) return null;
  return (
    <Section
      title="Privileges"
      testId="structure-privileges"
      action={
        pg ? (
          <Button
            size="sm"
            variant="outline"
            onClick={() => setGranting((value) => !value)}
          >
            <IconPlus /> Grant
          </Button>
        ) : null
      }
    >
      {granting && pg ? (
        <GrantForm pg={pg} roles={roles} onDone={() => setGranting(false)} />
      ) : null}
      {grouped.size === 0 ? (
        <EmptyRow>No explicit privileges.</EmptyRow>
      ) : (
        <div className="divide-y divide-border-subtle">
          {[...grouped].map(([grantee, entries]) => (
            <PrivilegeRow
              key={grantee}
              grantee={grantee}
              entries={entries}
              pg={pg}
            />
          ))}
        </div>
      )}
    </Section>
  );
}

function PrivilegeRow({
  grantee,
  entries,
  pg,
}: {
  grantee: string;
  entries: PrivilegeInfo[];
  pg: PgStructureOps | null;
}) {
  const [cascade, setCascade] = useState(false);
  const needsExplicitTarget = grantee === "PUBLIC";
  const [revokeChoice, setRevokeChoice] = useState<PgGrantee | null>(null);
  const revokeGrantee =
    revokeChoice ??
    (grantee === "PUBLIC"
      ? needsExplicitTarget
        ? null
        : { kind: "public" as const }
      : { kind: "role" as const, name: grantee });
  const typed = RELATION_PRIVILEGES.filter((privilege) =>
    entries.some((entry) => relationPrivilege(entry.privilege) === privilege),
  );
  return (
    <div className="flex flex-wrap items-center gap-2 px-3 py-2 text-xs">
      <span className="min-w-28 font-mono">{grantee}</span>
      {entries.map((entry) => (
        <Badge key={entry.privilege} variant="outline">
          {entry.privilege}
          {entry.grantable ? "*" : ""}
        </Badge>
      ))}
      {pg ? (
        <div className="ml-auto flex items-center gap-1">
          {needsExplicitTarget ? (
            <select
              aria-label={`Revoke target for ${grantee}`}
              value={
                revokeGrantee === null
                  ? ""
                  : revokeGrantee.kind === "public"
                    ? "public"
                    : "role"
              }
              onChange={(event) =>
                setRevokeChoice(
                  event.target.value === "public"
                    ? { kind: "public" }
                    : event.target.value === "role"
                      ? { kind: "role", name: grantee }
                      : null,
                )
              }
              className="h-7 border border-border-subtle bg-surface-input px-1 text-2xs"
            >
              <option value="">Choose revoke target</option>
              <option value="public">PUBLIC pseudo-role</option>
              <option value="role">quoted role “PUBLIC”</option>
            </select>
          ) : null}
          <label className="flex items-center gap-1 text-2xs text-text-muted">
            <input
              type="checkbox"
              checked={cascade}
              onChange={(event) => setCascade(event.target.checked)}
            />
            cascade
          </label>
          <Button
            size="sm"
            variant="ghost"
            disabled={typed.length === 0 || revokeGrantee === null}
            onClick={() => {
              if (!revokeGrantee) return;
              pg.queueOp(
                buildRevokeOp({
                  schema: pg.schema,
                  table: pg.table,
                  grantee: revokeGrantee,
                  privileges: typed,
                  cascade,
                }),
              );
            }}
          >
            Revoke
          </Button>
        </div>
      ) : null}
    </div>
  );
}

function GrantForm({
  pg,
  roles,
  onDone,
}: {
  pg: PgStructureOps;
  roles: string[];
  onDone: () => void;
}) {
  const [grantee, setGrantee] = useState<PgGrantee>({ kind: "public" });
  const [selected, setSelected] = useState(new Set<PgPrivilege>(["select"]));
  const [withGrantOption, setWithGrantOption] = useState(false);
  return (
    <div
      data-testid="structure-grant-form"
      className="space-y-2 border-b border-border-subtle p-3"
    >
      <GranteeEditor
        label="Privilege grantee"
        grantee={grantee}
        roles={roles}
        onChange={setGrantee}
      />
      <div className="flex flex-wrap gap-3">
        {GRANTABLE_RELATION_PRIVILEGES.map((privilege) => (
          <label key={privilege} className="flex items-center gap-1 text-xs">
            <input
              type="checkbox"
              checked={selected.has(privilege)}
              onChange={() =>
                setSelected((current) => {
                  const next = new Set(current);
                  if (next.has(privilege)) next.delete(privilege);
                  else next.add(privilege);
                  return next;
                })
              }
            />
            {privilege.toUpperCase()}
          </label>
        ))}
      </div>
      <label className="flex items-center gap-1 text-xs">
        <input
          type="checkbox"
          checked={withGrantOption}
          onChange={(event) => setWithGrantOption(event.target.checked)}
        />
        WITH GRANT OPTION
      </label>
      <div className="flex justify-end gap-1">
        <Button size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button
          size="sm"
          disabled={
            selected.size === 0 ||
            (grantee.kind === "role" && grantee.name.trim() === "")
          }
          onClick={() => {
            pg.queueOp(
              buildGrantOp({
                schema: pg.schema,
                table: pg.table,
                grantee,
                privileges: [...selected],
                withGrantOption,
              }),
            );
            onDone();
          }}
        >
          Queue grant
        </Button>
      </div>
    </div>
  );
}
