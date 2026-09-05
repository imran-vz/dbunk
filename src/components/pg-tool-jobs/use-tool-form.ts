import { useEffect, useRef, useState } from "react";
import { useStore } from "zustand";

import {
  decodePgToolError,
  formatPgToolError,
  pgToolClient,
  type PgArchiveFormat,
  type PgBackupScope,
  type PgToolJob,
} from "@/lib/pg-tool-jobs/client";
import { pgToolReview } from "@/lib/pg-tool-jobs/lifecycle";
import { pgToolObserver } from "@/lib/pg-tool-jobs/observer";
import { startReviewedPgRestore } from "@/lib/pg-tool-jobs/restore";
import { isConnectedStatus, type Connection, useAppStore } from "@/lib/store";

export function useToolForm(
  connection: Connection,
  initialScope: PgBackupScope,
  initialOperation: PgToolJob["kind"],
) {
  const [operation, setOperation] = useState(initialOperation);
  const [format, setFormat] = useState<PgArchiveFormat>("custom");
  const [scope, setScope] = useState(initialScope);
  const [path, setPath] = useState<string | null>(null);
  const [clean, setClean] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const closing = useStore(pgToolReview, (s) => s.closing);
  const lifetime = useRef(0);
  const pending = useRef(false);
  useEffect(() => {
    lifetime.current++;
    return () => {
      // oxlint-disable-next-line react-hooks/exhaustive-deps -- This monotonic counter fences async work; it is not a DOM ref.
      lifetime.current++;
    };
  }, []);
  function changeOperation(next: PgToolJob["kind"]) {
    if (pending.current) return;
    lifetime.current++;
    setOperation(next);
    setPath(null);
    setClean(false);
    setError(null);
  }
  function changeFormat(next: PgArchiveFormat) {
    lifetime.current++;
    setFormat(next);
    setPath(null);
    setClean(false);
    setError(null);
  }
  async function pick() {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    const epoch = lifetime.current;
    try {
      const selected = await pgToolClient.pick(operation, format);
      if (lifetime.current === epoch && selected) {
        setPath(selected);
        setError(null);
      }
    } catch {
      if (lifetime.current === epoch)
        setError("Unable to open the native file picker.");
    } finally {
      pending.current = false;
      if (lifetime.current === epoch) setBusy(false);
    }
  }
  async function submit() {
    if (pending.current || !path) return;
    pending.current = true;
    setBusy(true);
    setError(null);
    const epoch = lifetime.current;
    const revision = pgToolReview.getState().revision;
    const isCurrent = () => {
      const review = pgToolReview.getState();
      const current = useAppStore
        .getState()
        .connections.find((c) => c.id === connection.id);
      return (
        epoch === lifetime.current &&
        review.revision === revision &&
        !review.closing &&
        current?.engine === "PostgreSQL" &&
        isConnectedStatus(current.status)
      );
    };
    try {
      if (!isCurrent()) throw { kind: "connectionClosing" };
      const job =
        operation === "backup"
          ? await pgToolObserver.startBackup({
              connectionId: connection.id,
              destinationPath: path,
              format,
              scope,
              clean: format === "plain" && clean,
            })
          : await startReviewedPgRestore(
              {
                connectionId: connection.id,
                sourcePath: path,
                format,
                clean: format === "custom" && clean,
              },
              connection,
              isCurrent,
            );
      if (epoch === lifetime.current) {
        setJobId(job.jobId);
        setPath(null);
      }
    } catch (cause) {
      if (epoch === lifetime.current)
        setError(formatPgToolError(decodePgToolError(cause)));
    } finally {
      pending.current = false;
      if (epoch === lifetime.current) setBusy(false);
    }
  }
  return {
    operation,
    changeOperation,
    format,
    changeFormat,
    scope,
    setScope,
    path,
    clean,
    setClean,
    busy,
    error,
    jobId,
    closing,
    pick,
    submit,
  };
}
