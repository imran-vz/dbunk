/**
 * Import-from-URI (Plan 010, mock A): the first field of the
 * new-connection form. Pasting a supported URI prefills engine, host,
 * port, user, database (and password / TLS / db-number when carried);
 * ignored query parameters are disclosed, never silently dropped.
 * Only the fields the URI carries are overwritten.
 */

import { useState } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { parseConnectionUri } from "@/lib/connection-uri";

import type { ConnectionFormApi } from "./use-connection-form";

export function UriImportField({
  form,
  onEngineChange,
}: {
  form: ConnectionFormApi;
  onEngineChange: (
    engine: "PostgreSQL" | "MySQL" | "Redis",
  ) => void;
}) {
  const [value, setValue] = useState("");
  const [notice, setNotice] = useState<
    { tone: "info" | "warning" | "danger"; text: string } | undefined
  >();

  const applyUri = (raw: string) => {
    setValue(raw);
    if (!raw.trim()) {
      setNotice(undefined);
      return;
    }
    const parsed = parseConnectionUri(raw);
    if (!parsed.ok) {
      setNotice({ tone: "danger", text: parsed.reason });
      return;
    }
    const { values } = parsed;
    // Engine first so engine-specific defaults reset, then the carried
    // fields on top.
    onEngineChange(values.engine);
    form.setFieldValue("host", values.host);
    form.setFieldValue("port", values.port);
    form.setFieldValue("user", values.user);
    form.setFieldValue("database", values.database);
    if (values.password !== undefined) {
      form.setFieldValue("password", values.password);
    }
    if (values.engine === "Redis") {
      form.setFieldValue("useTls", values.useTls ?? false);
      if (values.dbNumber !== undefined) {
        form.setFieldValue("dbNumber", values.dbNumber);
      }
    }
    setNotice(
      values.ignoredParams.length > 0
        ? {
            tone: "warning",
            text: `Applied. Ignored parameters: ${values.ignoredParams.join(", ")} — set the matching options below.`,
          }
        : { tone: "info", text: `Applied ${values.engine} URI.` },
    );
  };

  return (
    <div className="grid gap-1.5">
      <Label htmlFor="connection-uri-import">Import from URI</Label>
      <Input
        id="connection-uri-import"
        placeholder="postgres://user@host:5432/db — paste to prefill"
        className="font-mono"
        value={value}
        onChange={(event) => applyUri(event.target.value)}
      />
      {notice ? (
        <p
          data-testid="uri-import-notice"
          className={
            notice.tone === "danger"
              ? "text-2xs text-danger"
              : notice.tone === "warning"
                ? "text-2xs text-warning"
                : "text-2xs text-text-muted"
          }
        >
          {notice.text}
        </p>
      ) : null}
    </div>
  );
}
