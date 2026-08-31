import type { PgObjectFacts } from "@/lib/store";

function FactRows({ rows }: { rows: Array<[string, React.ReactNode]> }) {
  return (
    <dl className="divide-y divide-border-subtle border-y border-border-subtle text-xs">
      {rows.map(([label, value]) => (
        <div key={label} className="grid grid-cols-[10rem_1fr] gap-4 px-3 py-2">
          <dt className="text-text-muted">{label}</dt>
          <dd className="min-w-0 font-mono text-text-secondary whitespace-pre-wrap">
            {value}
          </dd>
        </div>
      ))}
    </dl>
  );
}

/** The explicit cases make new backend fact variants a compile-time task. */
export function ObjectFactsPanel({ facts }: { facts: PgObjectFacts }) {
  switch (facts.kind) {
    case "schema":
      return (
        <p className="px-3 py-4 text-xs text-text-muted">
          Schema metadata has no additional typed facts.
        </p>
      );
    case "table":
      return (
        <p className="px-3 py-4 text-xs text-text-muted">
          Table structure is available in the table workspace.
        </p>
      );
    case "view":
      return <FactRows rows={[["Definition", facts.definition]]} />;
    case "materializedView":
      return (
        <FactRows
          rows={[
            ["Definition", facts.definition],
            ["Populated", facts.populated ? "Yes" : "No"],
          ]}
        />
      );
    case "foreignTable":
      return <FactRows rows={[["Foreign server", facts.server]]} />;
    case "sequence":
      return (
        <FactRows
          rows={[
            ["Data type", facts.dataType],
            ["Start", facts.start],
            ["Increment", facts.increment],
            ["Minimum", facts.minValue],
            ["Maximum", facts.maxValue],
            ["Cycle", facts.cycle ? "Yes" : "No"],
            ["Cache", facts.cache],
            ["Last value", facts.lastValue ?? "Not read"],
            ["Owned by", facts.ownedBy ?? "None"],
          ]}
        />
      );
    case "routine":
      return (
        <FactRows
          rows={[
            ["Language", facts.language],
            ["Returns", facts.returns ?? "None"],
            ["Volatility", facts.volatility ?? "Not reported"],
            ["Arguments", facts.arguments],
          ]}
        />
      );
    case "type": {
      const attributes = facts.attributes?.map(
        (attribute) =>
          `${attribute.name} ${attribute.dataType}${attribute.nullable ? "" : " NOT NULL"}`,
      );
      return (
        <FactRows
          rows={[
            ["Type class", facts.class],
            ["Enum labels", facts.enumLabels?.join("\n") ?? "None"],
            ["Attributes", attributes?.join("\n") ?? "None"],
            ["Subtype", facts.subtype ?? "None"],
          ]}
        />
      );
    }
    case "domain":
      return (
        <FactRows
          rows={[
            ["Base type", facts.baseType],
            ["Not null", facts.notNull ? "Yes" : "No"],
            ["Default", facts.defaultValue ?? "None"],
            ["Checks", facts.checks.join("\n") || "None"],
          ]}
        />
      );
    case "extension":
      return (
        <FactRows
          rows={[
            ["Version", facts.version],
            ["Schema", facts.schema],
          ]}
        />
      );
  }
  facts satisfies never;
  return null;
}
