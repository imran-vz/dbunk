import { IconColumns3, IconPlus, IconTrash } from "@tabler/icons-react";
import { cloneElement, useCallback, useEffect, useMemo, useState } from "react";

import { DdlPlanPreviewGroups, DdlReviewDialog } from "@/components/object-ddl";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { formatObjectDdlError, previewObjectDdl } from "@/lib/object-ddl";
import {
  isConnectedStatus,
  type DdlPlanPreview,
  type PgObjectError,
  type PgObjectOp,
  useAppStore,
} from "@/lib/store";
import {
  buildTableDesignerOps,
  newTableDesignerForm,
  splitSqlExpressionList,
  tableDesignerFieldForOpIndex,
  type TableDesignerColumn,
  type TableDesignerForm,
  type TableDesignerIndex,
  type TableDesignerValidation,
  validateTableDesignerForm,
} from "@/lib/table-designer";

type DesignerPreview =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ready"; preview: DdlPlanPreview; reviewedKey: string }
  | { state: "error"; error: PgObjectError; reviewedKey: string };

const PREVIEW_DEBOUNCE_MS = 250;

const splitList = (value: string): string[] =>
  value.trim() === "" ? [] : value.split(",").map((part) => part.trim());

const nextId = () => crypto.randomUUID();

export function TableDesigner({
  tabId,
  connectionId,
  schema,
}: {
  tabId: string;
  connectionId: string;
  schema: string;
}) {
  const storedForm = useAppStore(
    (state) =>
      state.workspaceTabs.find((tab) => tab.id === tabId)?.tableDesignerDraft,
  );
  const updateTableDesignerDraft = useAppStore(
    (state) => state.updateTableDesignerDraft,
  );
  const setTableDesignerApplying = useAppStore(
    (state) => state.setTableDesignerApplying,
  );
  const [fallbackForm] = useState(() => newTableDesignerForm(schema));
  const form = storedForm ?? fallbackForm;
  const setForm = useCallback<
    React.Dispatch<React.SetStateAction<TableDesignerForm>>
  >(
    (update) => updateTableDesignerDraft(tabId, update),
    [tabId, updateTableDesignerDraft],
  );
  const [reviewOpen, setReviewOpen] = useState(false);
  const [partiallyCreatedTable, setPartiallyCreatedTable] = useState<{
    schema: string;
    name: string;
  } | null>(null);
  const validation = useMemo(() => validateTableDesignerForm(form), [form]);
  const ops = useMemo(() => buildTableDesignerOps(form), [form]);
  const opsKey = useMemo(() => JSON.stringify(ops), [ops]);
  const ddlVersion = useAppStore((state) => state.pgObjectDdlVersion);
  const connectionEpoch = useAppStore(
    (state) => state.connectionEpochs[connectionId] ?? 0,
  );
  const connectionGeneration = useAppStore(
    (state) => state.pgObjectCatalog[connectionId]?.generation ?? 0,
  );
  const connectionReady = useAppStore((state) => {
    const connection = state.connections.find(
      (candidate) => candidate.id === connectionId,
    );
    return (
      connection?.engine === "PostgreSQL" &&
      isConnectedStatus(connection.status) &&
      !state.connectionTransitionIds.includes(connectionId)
    );
  });
  const previewIdentity =
    validation.valid && connectionReady
      ? JSON.stringify([
          connectionId,
          connectionEpoch,
          connectionGeneration,
          ddlVersion,
          opsKey,
        ])
      : null;
  const [loadedPreview, setLoadedPreview] = useState<DesignerPreview>({
    state: "idle",
  });

  useEffect(() => {
    if (!storedForm) {
      updateTableDesignerDraft(tabId, fallbackForm);
    }
  }, [fallbackForm, storedForm, tabId, updateTableDesignerDraft]);

  useEffect(() => {
    if (previewIdentity === null) {
      setLoadedPreview({ state: "idle" });
      return;
    }
    let stale = false;
    setLoadedPreview({ state: "loading" });
    const connectionLifetimeIsCurrent = () => {
      const current = useAppStore.getState();
      const connection = current.connections.find(
        (candidate) => candidate.id === connectionId,
      );
      return (
        connection?.engine === "PostgreSQL" &&
        isConnectedStatus(connection.status) &&
        !current.connectionTransitionIds.includes(connectionId) &&
        (current.connectionEpochs[connectionId] ?? 0) === connectionEpoch &&
        (current.pgObjectCatalog[connectionId]?.generation ?? 0) ===
          connectionGeneration &&
        current.pgObjectDdlVersion === ddlVersion
      );
    };
    const timer = window.setTimeout(() => {
      if (stale || !connectionLifetimeIsCurrent()) return;
      // SAFETY: opsKey is JSON.stringify of buildTableDesignerOps' PgObjectOp[].
      const previewOps = JSON.parse(opsKey) as PgObjectOp[];
      void previewObjectDdl({ connectionId, ops: previewOps }).then(
        (result) => {
          if (stale || !connectionLifetimeIsCurrent()) return;
          setLoadedPreview(
            result.kind === "ok"
              ? {
                  state: "ready",
                  preview: result.value,
                  reviewedKey: previewIdentity,
                }
              : result.kind === "error"
                ? {
                    state: "error",
                    error: result.error,
                    reviewedKey: previewIdentity,
                  }
                : { state: "idle" },
          );
        },
      );
    }, PREVIEW_DEBOUNCE_MS);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [
    connectionEpoch,
    connectionGeneration,
    connectionId,
    ddlVersion,
    opsKey,
    previewIdentity,
  ]);

  // Derive this synchronously from the complete connection lifetime identity.
  // A transition can therefore never render or enable Create with a preview
  // from the prior lifetime while the effect cleanup catches up.
  const preview: DesignerPreview = (() => {
    if (previewIdentity === null) return { state: "idle" };
    if (loadedPreview.state === "ready") {
      return loadedPreview.reviewedKey === previewIdentity
        ? loadedPreview
        : { state: "loading" };
    }
    if (loadedPreview.state === "error") {
      return loadedPreview.reviewedKey === previewIdentity
        ? loadedPreview
        : { state: "loading" };
    }
    return loadedPreview;
  })();

  const previewField =
    preview.state === "error" && preview.error.kind === "invalidOp"
      ? tableDesignerFieldForOpIndex(form, preview.error.opIndex)
      : null;
  const previewMessage =
    preview.state === "error" ? formatObjectDdlError(preview.error) : null;

  const setColumn = (
    index: number,
    update: (column: TableDesignerColumn) => TableDesignerColumn,
  ) => {
    setForm((current) => ({
      ...current,
      columns: current.columns.map((column, position) =>
        position === index ? update(column) : column,
      ),
    }));
  };

  const addColumn = () => {
    setForm((current) => ({
      ...current,
      columns: [
        ...current.columns,
        {
          id: nextId(),
          name: "",
          dataType: "text",
          nullable: true,
          identity: "none",
          defaultKind: "none",
          defaultValue: "",
          comment: "",
        },
      ],
    }));
  };

  const addIndex = () => {
    const index: TableDesignerIndex = {
      id: nextId(),
      name: "",
      columns: form.columns[0]?.name ? [form.columns[0].name] : [],
      unique: false,
      method: "btree",
      include: [],
      wherePredicate: "",
      concurrently: true,
    };
    setForm((current) => ({
      ...current,
      indexes: [...current.indexes, index],
    }));
  };

  const openCreatedTable = async (created: {
    schema: string;
    name: string;
  }) => {
    const store = useAppStore.getState();
    const designerStillExists = store.workspaceTabs.some(
      (tab) => tab.id === tabId && tab.kind === "table-designer",
    );
    if (!designerStillExists) return;

    // The table itself persists even when a later standalone index fails.
    store.setTableDesignerApplying(tabId, false);
    useAppStore.setState({ activeConnectionId: connectionId });
    store.openTableTab(created.schema, created.name);
    await store.closeTab(tabId);
  };

  return (
    <section className="flex h-full min-h-0 flex-col bg-background text-foreground">
      <header className="flex min-h-12 shrink-0 items-center gap-3 border-b border-border-subtle px-4">
        <IconColumns3 className="size-4 text-accent" />
        <div className="min-w-0">
          <h1 className="text-sm font-semibold">Create table</h1>
          <p className="font-mono text-2xs text-text-muted">{schema}</p>
        </div>
        <div className="ml-auto flex items-center gap-2">
          <label className="flex items-center gap-1.5 text-2xs text-text-muted">
            <input
              type="checkbox"
              checked={form.unlogged}
              onChange={(event) =>
                setForm((current) => ({
                  ...current,
                  unlogged: event.target.checked,
                }))
              }
            />
            UNLOGGED
          </label>
          <Button
            data-testid="table-designer-create"
            size="sm"
            disabled={!validation.valid || preview.state !== "ready"}
            onClick={() => setReviewOpen(true)}
          >
            Create
          </Button>
        </div>
      </header>

      <div className="grid min-h-0 flex-1 grid-cols-[minmax(34rem,1fr)_minmax(20rem,0.42fr)]">
        <div className="min-h-0 overflow-auto border-r border-border-subtle p-4">
          <div className="mx-auto max-w-5xl space-y-5">
            <div className="grid grid-cols-2 gap-3">
              <Field label="Table name" error={validation.fields.name}>
                <Input
                  aria-label="Table name"
                  value={form.name}
                  onChange={(event) =>
                    setForm((current) => ({
                      ...current,
                      name: event.target.value,
                    }))
                  }
                  className="font-mono"
                />
              </Field>
              <Field
                label="Comment"
                error={
                  validation.fields.comment ??
                  (previewField === "comment"
                    ? (previewMessage ?? undefined)
                    : undefined)
                }
              >
                <Input
                  aria-label="Table comment"
                  value={form.comment}
                  onChange={(event) =>
                    setForm((current) => ({
                      ...current,
                      comment: event.target.value,
                    }))
                  }
                />
              </Field>
            </div>

            {previewField === "table" && previewMessage ? (
              <div
                className="border border-danger px-3 py-2 text-xs text-danger"
                data-testid="table-designer-invalid-op"
                role="alert"
              >
                Table definition: {previewMessage}
              </div>
            ) : null}

            <DesignerSection
              title="Columns"
              action={
                <Button size="sm" variant="outline" onClick={addColumn}>
                  <IconPlus /> Add column
                </Button>
              }
            >
              <div className="grid grid-cols-[1.1fr_1fr_7rem_5rem_7rem_1fr_1fr_2rem] gap-1 border-b border-border-subtle px-2 py-1.5 text-2xs uppercase tracking-wide text-text-muted">
                <span>Name</span>
                <span>Type</span>
                <span>Identity</span>
                <span>Null</span>
                <span>Default</span>
                <span>Value</span>
                <span>Comment</span>
                <span />
              </div>
              <div className="divide-y divide-border-subtle">
                {form.columns.map((column, index) => {
                  const prefix = `columns.${index}`;
                  const identity = column.identity !== "none";
                  return (
                    <div key={column.id} className="px-2 py-2">
                      <div className="grid grid-cols-[1.1fr_1fr_7rem_5rem_7rem_1fr_1fr_2rem] items-center gap-1">
                        <Input
                          aria-label={`Column ${index + 1} name`}
                          value={column.name}
                          onChange={(event) =>
                            setColumn(index, (current) => ({
                              ...current,
                              name: event.target.value,
                            }))
                          }
                          className="h-7 font-mono text-xs"
                        />
                        <ControlError
                          id={`column-${column.id}-type-error`}
                          error={validation.fields[`${prefix}.dataType`]}
                        >
                          <Input
                            aria-label={`Column ${index + 1} type`}
                            value={column.dataType}
                            onChange={(event) =>
                              setColumn(index, (current) => ({
                                ...current,
                                dataType: event.target.value,
                              }))
                            }
                            className="h-7 font-mono text-xs"
                          />
                        </ControlError>
                        <select
                          aria-label={`Column ${index + 1} identity`}
                          value={column.identity}
                          onChange={(event) => {
                            const value = event.target.value;
                            setColumn(index, (current) => ({
                              ...current,
                              identity:
                                value === "always" || value === "by-default"
                                  ? value
                                  : "none",
                              nullable:
                                value === "none" ? current.nullable : false,
                              defaultValue:
                                value === "none" ? current.defaultValue : "",
                            }));
                          }}
                          className="h-7 border border-border-subtle bg-surface-input px-1 text-2xs"
                        >
                          <option value="none">None</option>
                          <option value="always">Always</option>
                          <option value="by-default">By default</option>
                        </select>
                        <input
                          aria-label={`Column ${index + 1} nullable`}
                          type="checkbox"
                          checked={column.nullable}
                          disabled={identity}
                          onChange={(event) =>
                            setColumn(index, (current) => ({
                              ...current,
                              nullable: event.target.checked,
                            }))
                          }
                        />
                        <select
                          aria-label={`Column ${index + 1} default kind`}
                          value={column.defaultKind}
                          disabled={identity}
                          onChange={(event) =>
                            setColumn(index, (current) => ({
                              ...current,
                              defaultKind:
                                event.target.value === "expression" ||
                                event.target.value === "literal"
                                  ? event.target.value
                                  : "none",
                            }))
                          }
                          className="h-7 border border-border-subtle bg-surface-input px-1 text-2xs"
                        >
                          <option value="none">None</option>
                          <option value="literal">Literal</option>
                          <option value="expression">Expression</option>
                        </select>
                        <ControlError
                          id={`column-${column.id}-default-error`}
                          error={validation.fields[`${prefix}.defaultValue`]}
                        >
                          <Input
                            aria-label={`Column ${index + 1} default`}
                            value={column.defaultValue}
                            disabled={identity || column.defaultKind === "none"}
                            onChange={(event) =>
                              setColumn(index, (current) => ({
                                ...current,
                                defaultValue: event.target.value,
                              }))
                            }
                            className="h-7 font-mono text-xs"
                          />
                        </ControlError>
                        <Input
                          aria-label={`Column ${index + 1} comment`}
                          value={column.comment}
                          onChange={(event) =>
                            setColumn(index, (current) => ({
                              ...current,
                              comment: event.target.value,
                            }))
                          }
                          className="h-7 text-xs"
                        />
                        <Button
                          size="icon-sm"
                          variant="ghost"
                          aria-label={`Remove column ${column.name || index + 1}`}
                          onClick={() =>
                            setForm((current) => ({
                              ...current,
                              columns: current.columns.filter(
                                (_, position) => position !== index,
                              ),
                            }))
                          }
                        >
                          <IconTrash />
                        </Button>
                      </div>
                      {validation.fields[`${prefix}.name`] ||
                      (previewField === `${prefix}.comment` &&
                        previewMessage) ? (
                        <p className="mt-1 text-2xs text-danger">
                          {validation.fields[`${prefix}.name`] ??
                            previewMessage}
                        </p>
                      ) : null}
                    </div>
                  );
                })}
              </div>
            </DesignerSection>

            <ConstraintEditors
              form={form}
              setForm={setForm}
              validation={validation}
            />

            <DesignerSection
              title="Indexes"
              action={
                <Button size="sm" variant="outline" onClick={addIndex}>
                  <IconPlus /> Add index
                </Button>
              }
            >
              {form.indexes.length === 0 ? (
                <EmptyLine>No extra indexes.</EmptyLine>
              ) : (
                <div className="divide-y divide-border-subtle">
                  {form.indexes.map((index, position) => (
                    <div
                      key={index.id}
                      className="px-2 py-2"
                      data-testid={`table-designer-index-${position}`}
                    >
                      <div className="grid grid-cols-[1fr_1.2fr_7rem_auto_auto_2rem] items-start gap-2">
                        <Input
                          aria-label={`Index ${position + 1} name`}
                          value={index.name}
                          placeholder="name (optional)"
                          onChange={(event) =>
                            updateIndex(setForm, position, {
                              name: event.target.value,
                            })
                          }
                          className="h-7 font-mono text-xs"
                        />
                        <ControlError
                          id={`index-${index.id}-columns-error`}
                          error={
                            validation.fields[`indexes.${position}.columns`]
                          }
                        >
                          <Input
                            aria-label={`Index ${position + 1} columns`}
                            value={index.columns.join(", ")}
                            placeholder="columns / expressions"
                            onChange={(event) =>
                              updateIndex(setForm, position, {
                                columns: splitSqlExpressionList(
                                  event.target.value,
                                ),
                              })
                            }
                            className="h-7 font-mono text-xs"
                          />
                        </ControlError>
                        <ControlError
                          id={`index-${index.id}-method-error`}
                          error={
                            validation.fields[`indexes.${position}.method`]
                          }
                        >
                          <Input
                            aria-label={`Index ${position + 1} method`}
                            value={index.method}
                            onChange={(event) =>
                              updateIndex(setForm, position, {
                                method: event.target.value,
                              })
                            }
                            className="h-7 font-mono text-xs"
                          />
                        </ControlError>
                        <CheckLabel
                          label="Unique"
                          checked={index.unique}
                          onChange={(unique) =>
                            updateIndex(setForm, position, { unique })
                          }
                        />
                        <CheckLabel
                          label="Concurrent"
                          checked={index.concurrently}
                          onChange={(concurrently) =>
                            updateIndex(setForm, position, { concurrently })
                          }
                        />
                        <Button
                          size="icon-sm"
                          variant="ghost"
                          aria-label={`Remove index ${position + 1}`}
                          onClick={() =>
                            setForm((current) => ({
                              ...current,
                              indexes: current.indexes.filter(
                                (_, i) => i !== position,
                              ),
                            }))
                          }
                        >
                          <IconTrash />
                        </Button>
                      </div>
                      <ControlError
                        id={`index-${index.id}-predicate-error`}
                        error={
                          validation.fields[
                            `indexes.${position}.wherePredicate`
                          ]
                        }
                      >
                        <Input
                          aria-label={`Index ${position + 1} predicate`}
                          value={index.wherePredicate}
                          placeholder="WHERE predicate (optional)"
                          onChange={(event) =>
                            updateIndex(setForm, position, {
                              wherePredicate: event.target.value,
                            })
                          }
                          className="mt-1 h-7 font-mono text-xs"
                        />
                      </ControlError>
                      {previewField === `indexes.${position}` &&
                      previewMessage ? (
                        <p className="mt-1 text-2xs text-danger">
                          {previewMessage}
                        </p>
                      ) : null}
                    </div>
                  ))}
                </div>
              )}
            </DesignerSection>
          </div>
        </div>

        <aside className="min-h-0 overflow-auto bg-surface-window">
          <div className="sticky top-0 border-b border-border-subtle bg-surface-window px-4 py-3">
            <h2 className="text-xs font-semibold uppercase tracking-wide">
              Live DDL preview
            </h2>
            <p className="mt-1 text-2xs text-text-muted">
              Regenerated from typed operations on every edit.
            </p>
          </div>
          {!validation.valid ? (
            <div className="p-4 text-xs text-danger">
              {Object.values(validation.fields)[0]}
            </div>
          ) : preview.state === "loading" ? (
            <div className="p-4 text-xs text-text-muted">
              Generating preview…
            </div>
          ) : preview.state === "error" ? (
            <div className="p-4 text-xs text-danger">{previewMessage}</div>
          ) : preview.state === "ready" ? (
            <DdlPlanPreviewGroups preview={preview.preview} />
          ) : (
            <div className="p-4 text-xs text-text-muted">
              Enter a valid table definition to preview DDL.
            </div>
          )}
        </aside>
      </div>

      {reviewOpen ? (
        <DdlReviewDialog
          open
          connectionId={connectionId}
          ops={ops}
          onOpenChange={(open) => {
            setReviewOpen(open);
            if (!open && partiallyCreatedTable) {
              void openCreatedTable(partiallyCreatedTable);
            }
          }}
          onApplyingChange={(applying) =>
            setTableDesignerApplying(tabId, applying)
          }
          onPartiallyApplied={() =>
            setPartiallyCreatedTable({
              schema: form.schema.trim(),
              name: form.name.trim(),
            })
          }
          onApplied={() =>
            openCreatedTable({
              schema: form.schema.trim(),
              name: form.name.trim(),
            })
          }
        />
      ) : null}
    </section>
  );
}

function ConstraintEditors({
  form,
  setForm,
  validation,
}: {
  form: TableDesignerForm;
  setForm: React.Dispatch<React.SetStateAction<TableDesignerForm>>;
  validation: TableDesignerValidation;
}) {
  return (
    <div className="grid gap-4 xl:grid-cols-2">
      <DesignerSection
        title="Primary key & unique"
        action={
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              setForm((current) => ({
                ...current,
                uniques: [
                  ...current.uniques,
                  { id: nextId(), name: null, columns: [] },
                ],
              }))
            }
          >
            <IconPlus /> Add unique
          </Button>
        }
      >
        <div className="space-y-2 p-2">
          <Field
            label="Primary key columns"
            error={validation.fields["primaryKey.columns"]}
          >
            <Input
              aria-label="Primary key columns"
              value={form.primaryKey?.columns.join(", ") ?? ""}
              onChange={(event) =>
                setForm((current) => ({
                  ...current,
                  primaryKey:
                    event.target.value.trim() === ""
                      ? null
                      : { name: null, columns: splitList(event.target.value) },
                }))
              }
              className="h-7 font-mono text-xs"
            />
          </Field>
          {form.uniques.length === 0 ? (
            <EmptyLine>No unique constraints.</EmptyLine>
          ) : (
            form.uniques.map((unique, index) => (
              <div
                className="grid grid-cols-[1fr_2rem] items-start gap-2"
                key={unique.id}
              >
                <ControlError
                  id={`unique-${index}-columns-error`}
                  error={validation.fields[`uniques.${index}.columns`]}
                >
                  <Input
                    aria-label={`Unique ${index + 1} columns`}
                    value={unique.columns.join(", ")}
                    placeholder="unique columns"
                    onChange={(event) =>
                      setForm((current) => ({
                        ...current,
                        uniques: current.uniques.map((candidate, position) =>
                          position === index
                            ? {
                                ...candidate,
                                columns: splitList(event.target.value),
                              }
                            : candidate,
                        ),
                      }))
                    }
                    className="h-7 font-mono text-xs"
                  />
                </ControlError>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label={`Remove unique ${index + 1}`}
                  onClick={() =>
                    setForm((current) => ({
                      ...current,
                      uniques: current.uniques.filter(
                        (_, position) => position !== index,
                      ),
                    }))
                  }
                >
                  <IconTrash />
                </Button>
              </div>
            ))
          )}
        </div>
      </DesignerSection>
      <DesignerSection
        title="Checks"
        action={
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              setForm((current) => ({
                ...current,
                checks: [
                  ...current.checks,
                  { id: nextId(), name: null, expression: "" },
                ],
              }))
            }
          >
            <IconPlus /> Add check
          </Button>
        }
      >
        <div className="space-y-2 p-2">
          {form.checks.length === 0 ? (
            <EmptyLine>No checks.</EmptyLine>
          ) : (
            form.checks.map((check, index) => (
              <div
                className="grid grid-cols-[1fr_2rem] items-start gap-2"
                key={check.id}
              >
                <ControlError
                  id={`check-${index}-expression-error`}
                  error={validation.fields[`checks.${index}.expression`]}
                >
                  <Input
                    aria-label={`Check ${index + 1} expression`}
                    value={check.expression}
                    placeholder="price > 0"
                    onChange={(event) =>
                      setForm((current) => ({
                        ...current,
                        checks: current.checks.map((candidate, position) =>
                          position === index
                            ? { ...candidate, expression: event.target.value }
                            : candidate,
                        ),
                      }))
                    }
                    className="h-7 font-mono text-xs"
                  />
                </ControlError>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label={`Remove check ${index + 1}`}
                  onClick={() =>
                    setForm((current) => ({
                      ...current,
                      checks: current.checks.filter(
                        (_, position) => position !== index,
                      ),
                    }))
                  }
                >
                  <IconTrash />
                </Button>
              </div>
            ))
          )}
        </div>
      </DesignerSection>
      <DesignerSection
        title="Foreign keys"
        action={
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              setForm((current) => ({
                ...current,
                foreignKeys: [
                  ...current.foreignKeys,
                  {
                    id: nextId(),
                    name: null,
                    columns: [],
                    referencedSchema: current.schema,
                    referencedTable: "",
                    referencedColumns: [],
                    onUpdate: "no-action",
                    onDelete: "no-action",
                    deferrable: false,
                    initiallyDeferred: false,
                  },
                ],
              }))
            }
          >
            <IconPlus /> Add foreign key
          </Button>
        }
      >
        <div className="space-y-2 p-2">
          {form.foreignKeys.map((foreignKey, index) => (
            <div
              className="grid grid-cols-2 gap-2 border border-border-subtle p-2"
              key={foreignKey.id}
            >
              <Field
                label="Local columns"
                error={validation.fields[`foreignKeys.${index}.columns`]}
              >
                <Input
                  aria-label={`Foreign key ${index + 1} columns`}
                  value={foreignKey.columns.join(", ")}
                  onChange={(event) =>
                    updateForeignKey(setForm, index, {
                      columns: splitList(event.target.value),
                    })
                  }
                  className="h-7 font-mono text-xs"
                />
              </Field>
              <Field
                label="Referenced schema"
                error={
                  validation.fields[`foreignKeys.${index}.referencedSchema`]
                }
              >
                <Input
                  aria-label={`Foreign key ${index + 1} schema`}
                  value={foreignKey.referencedSchema}
                  onChange={(event) =>
                    updateForeignKey(setForm, index, {
                      referencedSchema: event.target.value,
                    })
                  }
                  className="h-7 font-mono text-xs"
                />
              </Field>
              <Field
                label="Referenced table"
                error={
                  validation.fields[`foreignKeys.${index}.referencedTable`]
                }
              >
                <Input
                  aria-label={`Foreign key ${index + 1} table`}
                  value={foreignKey.referencedTable}
                  onChange={(event) =>
                    updateForeignKey(setForm, index, {
                      referencedTable: event.target.value,
                    })
                  }
                  className="h-7 font-mono text-xs"
                />
              </Field>
              <Field
                label="Referenced columns"
                error={
                  validation.fields[`foreignKeys.${index}.referencedColumns`]
                }
              >
                <Input
                  aria-label={`Foreign key ${index + 1} referenced columns`}
                  value={foreignKey.referencedColumns.join(", ")}
                  onChange={(event) =>
                    updateForeignKey(setForm, index, {
                      referencedColumns: splitList(event.target.value),
                    })
                  }
                  className="h-7 font-mono text-xs"
                />
              </Field>
              <CheckLabel
                label="Deferrable"
                checked={foreignKey.deferrable}
                onChange={(deferrable) =>
                  updateForeignKey(setForm, index, { deferrable })
                }
              />
              <div>
                <CheckLabel
                  label="Initially deferred"
                  checked={foreignKey.initiallyDeferred}
                  onChange={(initiallyDeferred) =>
                    updateForeignKey(setForm, index, { initiallyDeferred })
                  }
                />
                {validation.fields[`foreignKeys.${index}.deferrable`] ? (
                  <p className="mt-1 text-2xs text-danger">
                    {validation.fields[`foreignKeys.${index}.deferrable`]}
                  </p>
                ) : null}
              </div>
              <Button
                className="col-span-2 justify-self-end"
                size="icon-sm"
                variant="ghost"
                aria-label={`Remove foreign key ${index + 1}`}
                onClick={() =>
                  setForm((current) => ({
                    ...current,
                    foreignKeys: current.foreignKeys.filter(
                      (_, position) => position !== index,
                    ),
                  }))
                }
              >
                <IconTrash />
              </Button>
            </div>
          ))}
          {form.foreignKeys.length === 0 ? (
            <EmptyLine>No foreign keys.</EmptyLine>
          ) : null}
        </div>
      </DesignerSection>
    </div>
  );
}

function updateForeignKey(
  setForm: React.Dispatch<React.SetStateAction<TableDesignerForm>>,
  position: number,
  update: Partial<TableDesignerForm["foreignKeys"][number]>,
) {
  setForm((current) => ({
    ...current,
    foreignKeys: current.foreignKeys.map((foreignKey, index) =>
      index === position ? { ...foreignKey, ...update } : foreignKey,
    ),
  }));
}

function updateIndex(
  setForm: React.Dispatch<React.SetStateAction<TableDesignerForm>>,
  position: number,
  update: Partial<TableDesignerIndex>,
) {
  setForm((current) => ({
    ...current,
    indexes: current.indexes.map((index, indexPosition) =>
      indexPosition === position ? { ...index, ...update } : index,
    ),
  }));
}

function DesignerSection({
  title,
  action,
  children,
}: {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="border border-border-subtle">
      <header className="flex min-h-9 items-center border-b border-border-subtle px-3">
        <h2 className="text-xs font-semibold uppercase tracking-wide">
          {title}
        </h2>
        {action ? <div className="ml-auto">{action}</div> : null}
      </header>
      {children}
    </section>
  );
}

function Field({
  label,
  error,
  children,
}: {
  label: string;
  error?: string;
  children: React.ReactElement<{ "aria-invalid"?: boolean }>;
}) {
  return (
    <label className="grid gap-1 text-xs text-text-secondary">
      <span>{label}</span>
      {error ? cloneElement(children, { "aria-invalid": true }) : children}
      {error ? <span className="text-2xs text-danger">{error}</span> : null}
    </label>
  );
}

function ControlError({
  id,
  error,
  children,
}: {
  id: string;
  error?: string;
  children: React.ReactElement<{
    "aria-describedby"?: string;
    "aria-invalid"?: boolean;
  }>;
}) {
  return (
    <div className="grid gap-1">
      {error
        ? cloneElement(children, {
            "aria-describedby": id,
            "aria-invalid": true,
          })
        : children}
      {error ? (
        <p id={id} className="text-2xs text-danger">
          {error}
        </p>
      ) : null}
    </div>
  );
}

function CheckLabel({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-1 text-2xs text-text-muted">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label}
    </label>
  );
}

function EmptyLine({ children }: { children: React.ReactNode }) {
  return <p className="px-2 py-3 text-xs text-text-muted">{children}</p>;
}
