// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { EnvironmentBadge } from "@/components/environment-badge";
import type { ConnectionEnvironment } from "@/lib/store";

afterEach(cleanup);

describe("EnvironmentBadge", () => {
  it.each([
    ["test", "Test"],
    ["staging", "Staging"],
    ["production", "Production"],
  ] satisfies Array<[ConnectionEnvironment, string]>)(
    "renders the fixed %s identity",
    (environment, label) => {
      render(<EnvironmentBadge environment={environment} />);
      expect(screen.getByText(label)).toBeTruthy();
    },
  );

  it("leaves development connections unmarked", () => {
    const { container } = render(
      <EnvironmentBadge environment="development" />,
    );
    expect(container.childElementCount).toBe(0);
  });
});
