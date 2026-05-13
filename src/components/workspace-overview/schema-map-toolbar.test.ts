import { describe, expect, it } from "vitest";

import {
  safeSchemaMapSlug,
  schemaMapExportFilename,
} from "@/components/workspace-overview/schema-map-toolbar";

describe("schema map export filenames", () => {
  it("normalizes spaces, slashes, case, and diacritics", () => {
    expect(
      schemaMapExportFilename("Å Sales / Prod", "Public Data", "png"),
    ).toBe("a-sales-prod-public-data-schema.png");
  });

  it("falls back when a segment has no safe characters", () => {
    expect(safeSchemaMapSlug("///")).toBe("schema");
    expect(schemaMapExportFilename("///", "public", "svg")).toBe(
      "schema-public-schema.svg",
    );
  });
});
