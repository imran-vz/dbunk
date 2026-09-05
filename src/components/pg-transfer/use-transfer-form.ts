import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "zustand";

import { pgToolReview } from "@/lib/pg-tool-jobs/lifecycle";
import {
  decodePgTransferError,
  DEFAULT_PG_CSV_OPTIONS,
  formatPgTransferError,
  pgTransferClient,
  type PgCsvOptions,
  type PgTransferColumnMapping,
  type PgTransferDirection,
  type PgTransferInspection,
} from "@/lib/pg-transfer/client";
import { pgTransferObserver } from "@/lib/pg-transfer/observer";
import { startReviewedPgCsvImport } from "@/lib/pg-transfer/start-import";
import { isConnectedStatus, type Connection, useAppStore } from "@/lib/store";

export type PgTransferIntent = {
  id: number;
  direction: PgTransferDirection;
  options?: Partial<PgCsvOptions>;
};

type BusyState = "picking" | "inspecting" | "starting" | null;

function defaultMapping(inspection: PgTransferInspection) {
  const eligibleTargets = inspection.targetColumns.filter(
    (column) => !column.generated && !column.identity,
  );
  const byLower = new Map(
    eligibleTargets.map((column) => [column.name.toLowerCase(), column.name]),
  );
  const used = new Set<string>();
  return inspection.sourceColumns.map((source) => {
    const target = byLower.get(source.name.toLowerCase()) ?? "";
    if (!target || used.has(target)) {
      return { sourceIndex: source.index, targetColumn: "" };
    }
    used.add(target);
    return { sourceIndex: source.index, targetColumn: target };
  });
}

export function mappingProblem(
  inspection: PgTransferInspection | null,
  mapping: PgTransferColumnMapping[],
) {
  if (!inspection || inspection.direction !== "import") return null;
  const mapped = mapping.filter((entry) => entry.targetColumn.length > 0);
  if (mapped.length === 0) return "Map at least one source column.";
  const targets = mapped.map((entry) => entry.targetColumn);
  if (new Set(targets).size !== targets.length) {
    return "Each target column can be mapped only once.";
  }
  const missing = inspection.targetColumns
    .filter(
      (column) =>
        !column.nullable &&
        !column.hasDefault &&
        !column.generated &&
        !column.identity &&
        !targets.includes(column.name),
    )
    .map((column) => column.name);
  if (missing.length) {
    return `Required target ${missing.length === 1 ? "column is" : "columns are"} not mapped: ${missing.join(", ")}.`;
  }
  return null;
}

export function csvOptionsProblem(options: PgCsvOptions) {
  const singleByte = (value: string) =>
    value.length === 1 && new TextEncoder().encode(value).length === 1;
  if (!singleByte(options.delimiter)) return "Delimiter must be one byte.";
  if (!singleByte(options.quote)) return "Quote must be one byte.";
  if (!singleByte(options.escape)) return "Escape must be one byte.";
  if (["\r", "\n", "\0"].includes(options.delimiter)) {
    return "Delimiter cannot be a line break or NUL.";
  }
  if (options.delimiter === options.quote) {
    return "Delimiter and quote must differ.";
  }
  if (options.nullToken.length > 64 || options.nullToken.includes("\0")) {
    return "NULL token must be at most 64 characters and cannot contain NUL.";
  }
  return null;
}

export function useTransferForm(
  connection: Connection,
  target: { schema: string; table: string },
  intent?: PgTransferIntent,
) {
  const [direction, setDirection] = useState<PgTransferDirection>(
    intent?.direction ?? "import",
  );
  const [options, setOptions] = useState<PgCsvOptions>({
    ...DEFAULT_PG_CSV_OPTIONS,
    ...intent?.options,
  });
  const [path, setPath] = useState<string | null>(null);
  const [inspection, setInspection] = useState<PgTransferInspection | null>(
    null,
  );
  const inspectionRef = useRef<PgTransferInspection | null>(null);
  const [mapping, setMapping] = useState<PgTransferColumnMapping[]>([]);
  const [busy, setBusy] = useState<BusyState>(null);
  const [error, setError] = useState<string | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const closing = useStore(pgToolReview, (state) => state.closing);
  const generation = useRef(0);
  const pending = useRef(false);
  const lastIntentId = useRef(intent?.id);
  const targetKey = `${connection.id}\0${target.schema}\0${target.table}`;
  const lastTargetKey = useRef(targetKey);

  const replaceInspection = (next: PgTransferInspection | null) => {
    inspectionRef.current = next;
    setInspection(next);
  };

  const releaseCurrentInspection = () => {
    const current = inspectionRef.current;
    if (!current) return;
    replaceInspection(null);
    void pgTransferClient
      .releaseInspection(current.inspectionToken)
      .catch(() => undefined);
  };

  useEffect(
    () => () => {
      // Fence picker and inspection continuations; admitted jobs are observer-owned.
      generation.current++;
      const current = inspectionRef.current;
      if (current) {
        void pgTransferClient
          .releaseInspection(current.inspectionToken)
          .catch(() => undefined);
      }
    },
    [],
  );

  useEffect(() => {
    if (intent === undefined || lastIntentId.current === intent.id) return;
    lastIntentId.current = intent.id;
    generation.current++;
    pending.current = false;
    setBusy(null);
    releaseCurrentInspection();
    setDirection(intent.direction);
    setOptions({ ...DEFAULT_PG_CSV_OPTIONS, ...intent.options });
    setPath(null);
    setMapping([]);
    setError(null);
    setJobId(null);
    // intent.id is the explicit reset signal; option object identity is irrelevant.
    // oxlint-disable-next-line react-hooks/exhaustive-deps
  }, [intent?.id]);

  useEffect(() => {
    if (lastTargetKey.current === targetKey) return;
    lastTargetKey.current = targetKey;
    generation.current++;
    pending.current = false;
    setBusy(null);
    releaseCurrentInspection();
    setPath(null);
    setMapping([]);
    setError(null);
    setJobId(null);
    // targetKey is the complete connection/relation identity reset signal.
    // oxlint-disable-next-line react-hooks/exhaustive-deps
  }, [targetKey]);

  function currentCheck(epoch: number, reviewRevision: number) {
    const review = pgToolReview.getState();
    const current = useAppStore
      .getState()
      .connections.find((candidate) => candidate.id === connection.id);
    return (
      generation.current === epoch &&
      review.revision === reviewRevision &&
      !review.closing &&
      current?.engine === "PostgreSQL" &&
      isConnectedStatus(current.status)
    );
  }

  function changeDirection(next: PgTransferDirection) {
    if (pending.current || next === direction) return;
    generation.current++;
    releaseCurrentInspection();
    setDirection(next);
    setOptions(DEFAULT_PG_CSV_OPTIONS);
    setPath(null);
    setMapping([]);
    setError(null);
    setJobId(null);
  }

  function changeOptions(next: PgCsvOptions) {
    generation.current++;
    releaseCurrentInspection();
    setOptions(next);
    setMapping([]);
    setError(null);
  }

  async function inspect(selectedPath = path) {
    if (pending.current || (direction === "import" && !selectedPath)) return;
    const optionsError = csvOptionsProblem(options);
    if (optionsError) {
      setError(optionsError);
      return;
    }
    pending.current = true;
    setBusy("inspecting");
    setError(null);
    releaseCurrentInspection();
    const epoch = generation.current;
    const reviewRevision = pgToolReview.getState().revision;
    try {
      if (!currentCheck(epoch, reviewRevision)) {
        throw { kind: "connectionClosing" };
      }
      const reviewed = await pgTransferClient.inspect({
        connectionId: connection.id,
        schema: target.schema,
        table: target.table,
        direction,
        sourcePath: direction === "import" ? selectedPath : null,
        options,
      });
      if (!currentCheck(epoch, reviewRevision)) {
        void pgTransferClient
          .releaseInspection(reviewed.inspectionToken)
          .catch(() => undefined);
        throw { kind: "connectionClosing" };
      }
      replaceInspection(reviewed);
      setMapping(direction === "import" ? defaultMapping(reviewed) : []);
    } catch (cause) {
      if (generation.current === epoch) {
        setError(formatPgTransferError(decodePgTransferError(cause)));
      }
    } finally {
      if (generation.current === epoch) {
        pending.current = false;
        setBusy(null);
      }
    }
  }

  async function pick() {
    if (pending.current) return;
    pending.current = true;
    setBusy("picking");
    setError(null);
    const epoch = generation.current;
    try {
      const selected = await pgTransferClient.pick(direction);
      if (generation.current !== epoch || !selected) return;
      setPath(selected);
      if (direction === "import") {
        pending.current = false;
        setBusy(null);
        await inspect(selected);
      }
    } catch {
      if (generation.current === epoch) {
        setError("Unable to open the native CSV file picker.");
      }
    } finally {
      if (generation.current === epoch) {
        pending.current = false;
        setBusy(null);
      }
    }
  }

  async function submit() {
    if (pending.current || !inspection || !path) return;
    const problem = mappingProblem(inspection, mapping);
    if (problem) {
      setError(problem);
      return;
    }
    pending.current = true;
    setBusy("starting");
    setError(null);
    const epoch = generation.current;
    const reviewRevision = pgToolReview.getState().revision;
    const isCurrent = () => currentCheck(epoch, reviewRevision);
    try {
      const job =
        direction === "import"
          ? await startReviewedPgCsvImport(
              {
                inspectionToken: inspection.inspectionToken,
                mapping: mapping.filter(
                  (entry) => entry.targetColumn.length > 0,
                ),
              },
              connection,
              isCurrent,
            )
          : await pgTransferObserver.startExport({
              inspectionToken: inspection.inspectionToken,
              destinationPath: path,
            });
      if (generation.current === epoch) {
        // Admission consumes the inspection. The observer owns the job now.
        replaceInspection(null);
        setMapping([]);
        setPath(null);
        setJobId(job.jobId);
      }
    } catch (cause) {
      if (generation.current === epoch) {
        const decoded = decodePgTransferError(cause);
        setError(formatPgTransferError(decoded));
        if (
          decoded.kind === "inspectionExpired" ||
          decoded.kind === "sourceChanged" ||
          decoded.kind === "targetChanged"
        ) {
          releaseCurrentInspection();
          setMapping([]);
        }
      }
    } finally {
      if (generation.current === epoch) {
        pending.current = false;
        setBusy(null);
      }
    }
  }

  const validation = useMemo(
    () => mappingProblem(inspection, mapping),
    [inspection, mapping],
  );

  return {
    direction,
    changeDirection,
    options,
    changeOptions,
    path,
    inspection,
    mapping,
    setMapping,
    busy,
    error,
    jobId,
    closing,
    validation,
    pick,
    inspect,
    submit,
  };
}
