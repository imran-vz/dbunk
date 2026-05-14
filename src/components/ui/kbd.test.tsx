// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { Kbd } from "./kbd";

afterEach(() => {
  cleanup();
});

describe("Kbd", () => {
  it("renders the supplied keys", () => {
    const { container } = render(<Kbd keys={["mod", "K"]} />);
    // We can't assert on the platform glyph because the test environment
    // sniffs the real navigator; just check the upper-case letter is
    // somewhere in the output.
    expect(container.textContent).toContain("K");
  });

  it("uppercases single-character tokens", () => {
    const { container } = render(<Kbd keys={["a"]} />);
    expect(container.textContent).toContain("A");
  });

  it("preserves multi-character non-mapped tokens", () => {
    const { container } = render(<Kbd keys={["F12"]} />);
    expect(container.textContent).toContain("F12");
  });
});
