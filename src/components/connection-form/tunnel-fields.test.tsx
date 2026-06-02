// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { JumpChainField } from "@/components/connection-form/tunnel-fields";
import type { BastionServer } from "@/lib/store";

const bastions: BastionServer[] = [
  {
    id: "jump-1",
    name: "Jump Bastion",
    host: "jump.local",
    port: 22,
    user: "ubuntu",
    authMethod: "privateKeyPath",
    privateKeyPath: "~/.ssh/id_ed25519",
    hostKeyFingerprint: undefined,
    createdAt: "2026-06-02T00:00:00Z",
    updatedAt: "2026-06-02T00:00:00Z",
    hasPassword: false,
    hasPrivateKeyContent: false,
    hasPassphrase: false,
  },
];

afterEach(() => {
  cleanup();
});

describe("JumpChainField", () => {
  it("keeps an empty draft hop visible after Add hop", () => {
    let value: string[] = [];
    const onChange = vi.fn((next: string[]) => {
      value = next;
      rerender(
        <JumpChainField
          bastions={bastions}
          finalBastionId="final"
          value={value}
          onChange={onChange}
          errorText={null}
        />,
      );
    });
    const { rerender } = render(
      <JumpChainField
        bastions={bastions}
        finalBastionId="final"
        value={value}
        onChange={onChange}
        errorText={null}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Add hop" }));

    expect(onChange).toHaveBeenCalledWith([""]);
    expect(screen.queryByText("No jump hops configured.")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Remove jump hop 1" }),
    ).toBeTruthy();
  });
});
