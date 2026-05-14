// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  CELL_EDITORS,
  formatPgArrayLiteral,
  parsePgArrayLiteral,
  specializedCellKind,
} from "./cell-editors";

afterEach(() => {
  cleanup();
});

describe("specializedCellKind", () => {
  it("maps json / jsonb / array / geometry", () => {
    expect(specializedCellKind("json")).toBe("json");
    expect(specializedCellKind("jsonb")).toBe("json");
    expect(specializedCellKind("text[]")).toBe("array");
    expect(specializedCellKind("integer[]")).toBe("array");
    expect(specializedCellKind("geometry")).toBe("geometry");
    expect(specializedCellKind("geometry(Point,4326)")).toBe("geometry");
    expect(specializedCellKind("geography(LineString)")).toBe("geometry");
  });

  it("returns null for primitives that use the inline editor", () => {
    expect(specializedCellKind("text")).toBeNull();
    expect(specializedCellKind("integer")).toBeNull();
    expect(specializedCellKind("uuid")).toBeNull();
    expect(specializedCellKind("timestamp")).toBeNull();
    expect(specializedCellKind(undefined)).toBeNull();
  });
});

describe("parsePgArrayLiteral", () => {
  it("parses an empty literal", () => {
    expect(parsePgArrayLiteral("{}")).toEqual([]);
  });

  it("parses simple comma-separated values", () => {
    expect(parsePgArrayLiteral("{a,b,c}")).toEqual(["a", "b", "c"]);
  });

  it("parses quoted strings with spaces", () => {
    expect(parsePgArrayLiteral('{"hello world","a,b","c"}')).toEqual([
      "hello world",
      "a,b",
      "c",
    ]);
  });

  it("respects backslash escapes inside quoted strings", () => {
    expect(parsePgArrayLiteral('{"a\\"b","c\\\\d"}')).toEqual(['a"b', "c\\d"]);
  });

  it("falls back to a single-element list when the value isn't braced", () => {
    expect(parsePgArrayLiteral("plain")).toEqual(["plain"]);
  });
});

describe("formatPgArrayLiteral", () => {
  it("emits {} for empty input", () => {
    expect(formatPgArrayLiteral([])).toBe("{}");
  });

  it("leaves bareword elements unquoted", () => {
    expect(formatPgArrayLiteral(["a", "b", "c"])).toBe("{a,b,c}");
  });

  it("quotes elements with commas, whitespace, or special chars", () => {
    expect(formatPgArrayLiteral(["hello world", "a,b"])).toBe(
      '{"hello world","a,b"}',
    );
  });

  it("escapes inner quotes and backslashes", () => {
    expect(formatPgArrayLiteral(['a"b', "c\\d"])).toBe('{"a\\"b","c\\\\d"}');
  });

  it("round-trips through parse/format", () => {
    const input = ["foo", "bar baz", "a,b", 'c"d'];
    const literal = formatPgArrayLiteral(input);
    expect(parsePgArrayLiteral(literal)).toEqual(input);
  });
});

describe("JsonCellEditor", () => {
  const Editor = CELL_EDITORS.json;

  it("rejects invalid JSON on save", () => {
    const onSave = vi.fn();
    const onCancel = vi.fn();
    render(
      <Editor
        initialValue="{ not json"
        columnName="payload"
        onSave={onSave}
        onCancel={onCancel}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    expect(onSave).not.toHaveBeenCalled();
  });

  it("saves valid JSON verbatim", () => {
    const onSave = vi.fn();
    render(
      <Editor
        initialValue=""
        columnName="payload"
        onSave={onSave}
        onCancel={vi.fn()}
      />,
    );
    const textarea = screen.getByPlaceholderText('{"key": "value"}');
    fireEvent.change(textarea, { target: { value: '{"a":1}' } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    expect(onSave).toHaveBeenCalledWith('{"a":1}');
  });
});

describe("ArrayCellEditor", () => {
  const Editor = CELL_EDITORS.array;

  it("renders existing elements and saves an updated literal", () => {
    const onSave = vi.fn();
    render(
      <Editor
        initialValue="{a,b}"
        columnName="tags"
        onSave={onSave}
        onCancel={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /add element/i }));
    const inputs = screen.getAllByRole("textbox");
    fireEvent.change(inputs[2], { target: { value: "c" } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    expect(onSave).toHaveBeenCalledWith("{a,b,c}");
  });
});

describe("GeometryCellEditor", () => {
  const Editor = CELL_EDITORS.geometry;

  it("rejects values that don't look like WKT", () => {
    const onSave = vi.fn();
    render(
      <Editor
        initialValue="not a polygon"
        columnName="region"
        onSave={onSave}
        onCancel={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    expect(onSave).not.toHaveBeenCalled();
  });

  it("accepts valid WKT", () => {
    const onSave = vi.fn();
    render(
      <Editor
        initialValue=""
        columnName="region"
        onSave={onSave}
        onCancel={vi.fn()}
      />,
    );
    const textarea = screen.getByPlaceholderText("POINT(10 20)");
    fireEvent.change(textarea, { target: { value: "POINT(10 20)" } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    expect(onSave).toHaveBeenCalledWith("POINT(10 20)");
  });
});
