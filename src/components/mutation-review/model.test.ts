import { describe, expect, it } from "vitest";

import { formatMutationError } from "@/components/mutation-review/model";

describe("formatMutationError", () => {
  it("renders a TLS failure with its headline, not as a lost connection", () => {
    expect(
      formatMutationError({
        kind: "tlsFailed",
        tlsKind: "hostnameMismatch",
        message:
          "The server certificate does not match the expected host name: NotValidForName",
      }),
    ).toBe(
      "TLS: the certificate does not match the expected host name — NotValidForName",
    );
    expect(formatMutationError({ kind: "connectionLost" })).toBe(
      "The database connection was lost.",
    );
  });
});
