import { describe, expect, it } from "vitest";

import { routeWholeTableExport } from "./export-routing";

describe("PostgreSQL whole-table export routing", () => {
  it("routes supported CSV settings to a native transfer", () => {
    expect(
      routeWholeTableExport("PostgreSQL", {
        format: "csv",
        encoding: "utf-8",
        compression: "none",
        nullAs: "",
      }),
    ).toEqual({
      kind: "nativeCsv",
      options: { header: true, nullToken: "" },
    });
  });

  it.each([
    { encoding: "utf-16le", compression: "none" },
    { encoding: "utf-8", compression: "gzip" },
  ] as const)(
    "refuses PostgreSQL CSV $encoding/$compression instead of buffering it",
    ({ encoding, compression }) => {
      expect(
        routeWholeTableExport("PostgreSQL", {
          format: "csv",
          encoding,
          compression,
          nullAs: "NULL",
        }).kind,
      ).toBe("refused");
    },
  );

  it("preserves legacy routing for other formats and engines", () => {
    expect(
      routeWholeTableExport("PostgreSQL", {
        format: "xlsx",
        encoding: "utf-8",
        compression: "none",
        nullAs: "",
      }),
    ).toEqual({
      kind: "buffered",
      warning: "Whole-table XLSX export loads all rows into app memory.",
    });
    expect(
      routeWholeTableExport("SQLite", {
        format: "csv",
        encoding: "utf-16le",
        compression: "gzip",
        nullAs: "",
      }),
    ).toEqual({ kind: "buffered" });
  });
});
