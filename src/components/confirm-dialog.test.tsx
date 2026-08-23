// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { ConfirmDialogHost } from "@/components/confirm-dialog";
import { requestConfirm, requestPrompt } from "@/lib/confirm";

afterEach(cleanup);

describe("confirm service + dialog host", () => {
  it("resolves true when the confirm button is clicked", async () => {
    render(<ConfirmDialogHost />);
    const pending = requestConfirm({
      title: "Delete rows?",
      message: "This cannot be undone.",
      confirmLabel: "Delete",
      danger: true,
    });

    expect(await screen.findByText("Delete rows?")).toBeTruthy();
    const confirm = screen.getByRole("button", { name: "Delete" });
    // §6.4: destructive confirm renders in danger style, not default.
    expect(confirm.getAttribute("data-variant")).toBe("destructive");
    fireEvent.click(confirm);
    await expect(pending).resolves.toBe(true);
  });

  it("resolves false on cancel and shows the named object", async () => {
    render(<ConfirmDialogHost />);
    const pending = requestConfirm({
      title: "Delete bastion server?",
      message: "Connections that reference it must be changed first.",
      detail: "prod-jump-host",
    });

    expect(await screen.findByText("prod-jump-host")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await expect(pending).resolves.toBe(false);
  });

  it("queues sequential requests", async () => {
    render(<ConfirmDialogHost />);
    const first = requestConfirm({ title: "First?", message: "one" });
    const second = requestConfirm({ title: "Second?", message: "two" });

    fireEvent.click(await screen.findByRole("button", { name: "Confirm" }));
    await expect(first).resolves.toBe(true);

    expect(await screen.findByText("Second?")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await expect(second).resolves.toBe(false);
  });

  it("does not cancel a queued dialog when Cancel double-fires", async () => {
    render(<ConfirmDialogHost />);
    const first = requestConfirm({ title: "First?", message: "one" });
    const second = requestConfirm({ title: "Second?", message: "two" });

    // AlertDialogCancel wraps Base UI's Close primitive, so one click fires
    // both the onClick and the root onOpenChange(false); position-addressed
    // resolution would let the second call cancel the queued dialog unseen.
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    await expect(first).resolves.toBe(false);

    expect(await screen.findByText("Second?")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    await expect(second).resolves.toBe(true);
  });

  it("prompts for text and resolves the entered value", async () => {
    render(<ConfirmDialogHost />);
    const pending = requestPrompt({
      title: "Save command",
      message: "Name for this command:",
      defaultValue: "GET user:*",
    });

    // SAFETY: findByDisplayValue only matches form fields; this one is the prompt's Input.
    const input = (await screen.findByDisplayValue(
      "GET user:*",
    )) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "scan users" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await expect(pending).resolves.toBe("scan users");
  });

  it("resolves the prompt with null when cancelled", async () => {
    render(<ConfirmDialogHost />);
    const pending = requestPrompt({ title: "Name?" });
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    await expect(pending).resolves.toBeNull();
  });
});
