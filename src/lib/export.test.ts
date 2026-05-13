import { describe, expect, it } from "vitest";
import {
  type ExportTable,
  prepareExport,
  toCsv,
  toHtml,
  toJson,
  toMarkdown,
  toSqlInserts,
  toTxt,
  toXlsx,
} from "@/lib/export";

describe("toCsv", () => {
  it("emits a header row followed by data rows", () => {
    const table: ExportTable = {
      columns: ["id", "name"],
      rows: [
        ["1", "Ada"],
        ["2", "Grace"],
      ],
    };
    expect(toCsv(table)).toBe("id,name\n1,Ada\n2,Grace");
  });

  it("emits just the header when there are no rows", () => {
    const table: ExportTable = {
      columns: ["id", "name"],
      rows: [],
    };
    expect(toCsv(table)).toBe("id,name");
  });

  it("quotes cells that contain a comma", () => {
    const table: ExportTable = {
      columns: ["id", "name"],
      rows: [["1", "Lovelace, Ada"]],
    };
    expect(toCsv(table)).toBe('id,name\n1,"Lovelace, Ada"');
  });

  it("quotes cells that contain double quotes and escapes them", () => {
    const table: ExportTable = {
      columns: ["id", "name"],
      rows: [["1", 'Ada "Countess" Lovelace']],
    };
    expect(toCsv(table)).toBe('id,name\n1,"Ada ""Countess"" Lovelace"');
  });

  it("quotes cells that contain newlines or carriage returns", () => {
    const table: ExportTable = {
      columns: ["id", "note"],
      rows: [
        ["1", "line1\nline2"],
        ["2", "carriage\rreturn"],
      ],
    };
    expect(toCsv(table)).toBe(
      'id,note\n1,"line1\nline2"\n2,"carriage\rreturn"',
    );
  });

  it("quotes column headers when they need quoting", () => {
    const table: ExportTable = {
      columns: ["id", 'col, "weird"'],
      rows: [["1", "ok"]],
    };
    expect(toCsv(table)).toBe('id,"col, ""weird"""\n1,ok');
  });

  it("renders null as the empty string by default", () => {
    const table: ExportTable = {
      columns: ["id", "name"],
      rows: [
        ["1", null],
        ["2", "Grace"],
      ],
    };
    expect(toCsv(table)).toBe("id,name\n1,\n2,Grace");
  });

  it("renders null using the nullAs option when provided", () => {
    const table: ExportTable = {
      columns: ["id", "name"],
      rows: [
        ["1", null],
        ["2", "Grace"],
      ],
    };
    expect(toCsv(table, { nullAs: "NULL" })).toBe("id,name\n1,NULL\n2,Grace");
  });

  it("preserves empty string distinct from null when nullAs is set", () => {
    const table: ExportTable = {
      columns: ["a", "b"],
      rows: [["", null]],
    };
    expect(toCsv(table, { nullAs: "NULL" })).toBe("a,b\n,NULL");
  });

  it("does not quote an empty string", () => {
    const table: ExportTable = {
      columns: ["a", "b"],
      rows: [["", "x"]],
    };
    expect(toCsv(table)).toBe("a,b\n,x");
  });
});

describe("toJson", () => {
  it("emits an array of objects keyed by column name", () => {
    const table: ExportTable = {
      columns: ["id", "name"],
      rows: [
        ["1", "Ada"],
        ["2", "Grace"],
      ],
    };
    expect(toJson(table)).toBe(
      '[{"id":"1","name":"Ada"},{"id":"2","name":"Grace"}]',
    );
  });

  it("emits an empty array when there are no rows", () => {
    const table: ExportTable = {
      columns: ["id", "name"],
      rows: [],
    };
    expect(toJson(table)).toBe("[]");
  });

  it("preserves null cells as JSON null", () => {
    const table: ExportTable = {
      columns: ["id", "name"],
      rows: [["1", null]],
    };
    expect(toJson(table)).toBe('[{"id":"1","name":null}]');
  });

  it("indents output with 2 spaces when pretty is true", () => {
    const table: ExportTable = {
      columns: ["id"],
      rows: [["1"]],
    };
    expect(toJson(table, { pretty: true })).toBe(
      '[\n  {\n    "id": "1"\n  }\n]',
    );
  });

  it("emits [] for empty rows even when pretty", () => {
    const table: ExportTable = {
      columns: ["id"],
      rows: [],
    };
    expect(toJson(table, { pretty: true })).toBe("[]");
  });
});

describe("additional export formats", () => {
  const table: ExportTable = {
    columns: ["id", "name", "note"],
    rows: [
      ["1", "Ada", "hello"],
      ["2", "Grace", null],
    ],
  };

  it("emits SQL insert statements with quoted identifiers and literals", () => {
    expect(toSqlInserts(table, { tableName: "public.people" })).toBe(
      `INSERT INTO "public"."people" ("id", "name", "note") VALUES ('1', 'Ada', 'hello');\n` +
        `INSERT INTO "public"."people" ("id", "name", "note") VALUES ('2', 'Grace', NULL);`,
    );
  });

  it("emits HTML, Markdown, and TXT table exports", () => {
    expect(toHtml(table, "NULL")).toContain("<td>NULL</td>");
    expect(toMarkdown(table, "NULL")).toContain("| 2 | Grace | NULL |");
    expect(toTxt(table, "NULL")).toBe(
      "id\tname\tnote\n1\tAda\thello\n2\tGrace\tNULL",
    );
  });

  it("emits a valid XLSX zip payload", () => {
    const payload = toXlsx(table);
    expect(payload[0]).toBe(0x50);
    expect(payload[1]).toBe(0x4b);
    expect(new TextDecoder().decode(payload)).toContain(
      "xl/worksheets/sheet1.xml",
    );
  });

  it("prepares encoded filenames and content", () => {
    const prepared = prepareExport(table, {
      format: "txt",
      filenameBase: "people",
      encoding: "utf-16le",
      compression: "none",
      nullAs: "NULL",
    });
    expect(prepared.filename).toBe("people.txt");
    expect(prepared.content).toBeInstanceOf(Uint8Array);
  });
});
