// @vitest-environment jsdom
/**
 * P2 acceptance: a toolbar of Button + Input + SelectTrigger shares
 * one control height at every density. jsdom can't resolve CSS
 * variables, so this asserts the contract at the class level — all
 * three primitives must draw their height from the same
 * `--control-h` density variable (DESIGN-SYSTEM §2.3/§4.1–4.3).
 */
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { Button } from "./button";
import { Input } from "./input";
import { Select, SelectTrigger, SelectValue } from "./select";

afterEach(() => {
  cleanup();
});

describe("control alignment", () => {
  it("Button, Input, and SelectTrigger all size from --control-h", () => {
    const { container } = render(
      <div>
        <Button>Run</Button>
        <Input placeholder="Filter" />
        <Select>
          <SelectTrigger>
            <SelectValue placeholder="Table" />
          </SelectTrigger>
        </Select>
      </div>,
    );

    const button = container.querySelector('[data-slot="button"]');
    const input = container.querySelector('[data-slot="input"]');
    const trigger = container.querySelector('[data-slot="select-trigger"]');

    expect(button?.className).toContain("h-(--control-h)");
    expect(input?.className).toContain("h-(--control-h)");
    expect(trigger?.className).toContain("data-[size=default]:h-(--control-h)");
    expect(trigger?.getAttribute("data-size")).toBe("default");
  });

  it("small variants all size from --control-h-sm", () => {
    const { container } = render(
      <div>
        <Button size="sm">Run</Button>
        <Select>
          <SelectTrigger size="sm">
            <SelectValue placeholder="Table" />
          </SelectTrigger>
        </Select>
      </div>,
    );

    const button = container.querySelector('[data-slot="button"]');
    const trigger = container.querySelector('[data-slot="select-trigger"]');

    expect(button?.className).toContain("h-(--control-h-sm)");
    expect(trigger?.className).toContain("data-[size=sm]:h-(--control-h-sm)");
  });
});
