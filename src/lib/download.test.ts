// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { downloadFile } from "@/lib/download";

describe("downloadFile", () => {
  const createObjectURL = vi.fn((_blob: Blob): string => "blob:mock-url");
  const revokeObjectURL = vi.fn((_url: string) => {});
  let originalCreate: typeof URL.createObjectURL | undefined;
  let originalRevoke: typeof URL.revokeObjectURL | undefined;

  beforeEach(() => {
    createObjectURL.mockClear();
    revokeObjectURL.mockClear();
    originalCreate = URL.createObjectURL;
    originalRevoke = URL.revokeObjectURL;
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: createObjectURL,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revokeObjectURL,
    });
  });

  afterEach(() => {
    if (originalCreate) {
      Object.defineProperty(URL, "createObjectURL", {
        configurable: true,
        value: originalCreate,
      });
    }
    if (originalRevoke) {
      Object.defineProperty(URL, "revokeObjectURL", {
        configurable: true,
        value: originalRevoke,
      });
    }
  });

  it("creates an object URL, clicks an anchor, and revokes the URL", () => {
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, "click");
    clickSpy.mockImplementation(() => {});

    downloadFile("export.csv", "text/csv;charset=utf-8", "id,name\n1,Ada");

    expect(createObjectURL).toHaveBeenCalledTimes(1);
    const blobArg = createObjectURL.mock.calls[0]?.[0];
    expect(blobArg).toBeInstanceOf(Blob);
    expect(blobArg?.type).toContain("text/csv");
    expect(clickSpy).toHaveBeenCalledTimes(1);
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:mock-url");

    clickSpy.mockRestore();
  });
});
