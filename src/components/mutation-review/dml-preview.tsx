import { IconCopy } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import type { PreviewResult } from "@/lib/result-mutation";

import { displayMutationValue } from "./model";

type DmlPreviewProps = {
  preview: PreviewResult;
  onCopy: (text: string, label: string) => void;
};

export function DmlPreview({ preview, onCopy }: DmlPreviewProps) {
  return (
    <section
      aria-label="Generated DML"
      className="m-3 border border-border-subtle bg-black"
    >
      <header className="flex min-h-8 items-center gap-2 border-b border-border-subtle px-2">
        <h2 className="text-[0.6875rem] font-semibold text-foreground">
          Generated DML · {preview.statements.length} operations
        </h2>
      </header>
      <div className="divide-y divide-border-subtle">
        {preview.statements.map((statement) => {
          const params = statement.params.map(({ value }) => value);
          return (
            <article key={statement.opIndex} className="min-w-0">
              <div className="flex items-center gap-1 border-b border-border-subtle px-2 py-1">
                <span className="text-[0.625rem] text-text-muted">
                  Operation {statement.opIndex + 1}
                </span>
                <span className="flex-1" />
                <Button
                  type="button"
                  size="xs"
                  variant="ghost"
                  onClick={() => onCopy(statement.sql, "SQL")}
                >
                  <IconCopy /> Copy SQL
                </Button>
                <Button
                  type="button"
                  size="xs"
                  variant="ghost"
                  onClick={() =>
                    onCopy(JSON.stringify(params), "ordered parameters")
                  }
                >
                  <IconCopy /> Copy params
                </Button>
              </div>
              <pre className="m-0 whitespace-pre-wrap break-words bg-[#030303] p-2 font-mono text-[0.625rem] leading-relaxed text-text-secondary">
                {statement.sql}
              </pre>
              <div className="border-t border-border-subtle px-2 py-1.5">
                <div className="mb-1 text-[0.625rem] font-medium text-text-muted">
                  Ordered parameters
                </div>
                {statement.params.length > 0 ? (
                  <pre className="m-0 whitespace-pre-wrap break-words font-mono text-[0.625rem] text-text-secondary">
                    {statement.params
                      .map(
                        (param, index) =>
                          `${index + 1}. ${displayMutationValue(param.value)}`,
                      )
                      .join("\n")}
                  </pre>
                ) : (
                  <div className="text-[0.625rem] text-text-muted">None</div>
                )}
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}
