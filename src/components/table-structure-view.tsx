import {
  IconArrowBack,
  IconChevronDown,
  IconChevronUp,
  IconLock,
  IconSearch,
} from "@tabler/icons-react";
import { useState } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

export interface ColumnDefinition {
  name: string;
  dataType: string;
  isPrimaryKey?: boolean;
  isNullable?: boolean;
  isUnique?: boolean;
  defaultValue?: string;
  isGenerated?: boolean;
  generatedExpression?: string;
  length?: number;
  arrayDimensions?: number;
}

export interface ConstraintDefinition {
  name: string;
  type: "PRIMARY KEY" | "FOREIGN KEY" | "UNIQUE" | "CHECK";
  columns: string[];
  references?: string;
}

export interface IndexDefinition {
  name: string;
  isUnique: boolean;
  method: string;
  columns: string[];
}

export interface PolicyDefinition {
  name: string;
  command: string;
  roles: string[];
}

export interface TableStructureData {
  tableName: string;
  schema: string;
  columns: ColumnDefinition[];
  constraints: ConstraintDefinition[];
  indexes: IndexDefinition[];
  policies: PolicyDefinition[];
  rowLevelSecurity: boolean;
}

interface ColumnChange {
  originalName: string;
  newName?: string;
  originalType?: string;
  newType?: string;
  originalNullable?: boolean;
  newNullable?: boolean;
  originalPrimaryKey?: boolean;
  newPrimaryKey?: boolean;
  originalUnique?: boolean;
  newUnique?: boolean;
  originalDefault?: string;
  newDefault?: string;
}

interface TableStructureViewProps {
  data: TableStructureData;
  className?: string;
  onDiscard?: () => void;
  onCommit?: () => void;
}

type ColumnTab = "name" | "dataType" | "constraints" | "default" | "generated";
type ViewMode = "structure" | "review";

const dataTypes = [
  { category: "Numeric", types: ["integer", "smallint", "bigint"] },
  { category: "Serial", types: ["serial", "smallserial", "bigserial"] },
  {
    category: "Decimal",
    types: ["decimal", "numeric", "real", "double precision"],
  },
  { category: "Character", types: ["varchar", "char", "text"] },
  {
    category: "Date/Time",
    types: ["timestamp", "timestamptz", "date", "time", "timetz", "interval"],
  },
  { category: "Boolean", types: ["boolean"] },
  { category: "UUID", types: ["uuid"] },
  { category: "JSON", types: ["json", "jsonb"] },
  { category: "Binary", types: ["bytea"] },
];

interface ExpandableColumnProps {
  column: ColumnDefinition;
  isExpanded: boolean;
  onToggle: () => void;
  change?: ColumnChange;
  onUndo?: () => void;
  onColumnChange?: (change: Partial<ColumnChange>) => void;
}

function ExpandableColumn({
  column,
  isExpanded,
  onToggle,
  change,
  onUndo,
  onColumnChange,
}: ExpandableColumnProps) {
  const [activeTab, setActiveTab] = useState<ColumnTab>("name");
  const [columnName, setColumnName] = useState(change?.newName ?? column.name);
  const [selectedType, setSelectedType] = useState(
    (change?.newType ?? column.dataType).toLowerCase().split("(")[0],
  );
  const [typeLength, setTypeLength] = useState(column.length ?? 255);
  const [arrayDimensions, setArrayDimensions] = useState(
    column.arrayDimensions ?? 0,
  );
  const [notNull, setNotNull] = useState(
    change?.newNullable !== undefined
      ? !change.newNullable
      : !column.isNullable,
  );
  const [isPrimaryKey, setIsPrimaryKey] = useState(
    change?.newPrimaryKey ?? column.isPrimaryKey ?? false,
  );
  const [isUnique, setIsUnique] = useState(
    change?.newUnique ?? column.isUnique ?? false,
  );
  const [defaultValue, setDefaultValue] = useState(
    change?.newDefault ?? column.defaultValue ?? "",
  );
  const [isGenerated, setIsGenerated] = useState(column.isGenerated ?? false);
  const [generatedExpression, setGeneratedExpression] = useState(
    column.generatedExpression ?? "",
  );
  const [typeSearch, setTypeSearch] = useState("");

  const hasChange = change !== undefined;
  const displayName = columnName;
  const originalName = column.name;

  const constraintParts: string[] = [];
  if (isPrimaryKey) constraintParts.push("PRIMARY KEY");
  if (notNull && !isPrimaryKey) constraintParts.push("NOT NULL");

  const tabs: { id: ColumnTab; label: string }[] = [
    { id: "name", label: "Column name" },
    { id: "dataType", label: "Data type" },
    { id: "constraints", label: "Constraints" },
    { id: "default", label: "Default" },
    { id: "generated", label: "Generated" },
  ];

  const filteredDataTypes = dataTypes
    .map((category) => ({
      ...category,
      types: category.types.filter((type) =>
        type.toLowerCase().includes(typeSearch.toLowerCase()),
      ),
    }))
    .filter((category) => category.types.length > 0);

  const handleNameChange = (newName: string) => {
    setColumnName(newName);
    if (newName !== column.name) {
      onColumnChange?.({ originalName: column.name, newName });
    } else {
      onColumnChange?.({ originalName: column.name, newName: undefined });
    }
  };

  const renderTabContent = () => {
    switch (activeTab) {
      case "name":
        return (
          <div className="space-y-4">
            <div>
              <Label className="text-xs text-muted-foreground mb-1.5 block">
                Column name
              </Label>
              <Input
                value={columnName}
                onChange={(e) => handleNameChange(e.target.value)}
                className="h-9 bg-background/50 font-mono"
              />
            </div>
          </div>
        );

      case "dataType":
        return (
          <div className="flex gap-4">
            <div className="flex-1 space-y-2">
              <div className="relative">
                <IconSearch className="absolute left-3 top-1/2 -translate-y-1/2 size-4 text-muted-foreground" />
                <Input
                  placeholder="Search..."
                  value={typeSearch}
                  onChange={(e) => setTypeSearch(e.target.value)}
                  className="h-9 pl-9 bg-background/50"
                />
              </div>
              <div className="max-h-64 overflow-auto space-y-1">
                {filteredDataTypes.map((category) => (
                  <div key={category.category}>
                    {category.types.map((type) => (
                      <button
                        key={type}
                        type="button"
                        onClick={() => setSelectedType(type)}
                        className={cn(
                          "w-full text-left px-3 py-2 text-sm rounded-md transition-colors",
                          selectedType === type
                            ? "bg-muted text-foreground"
                            : "text-muted-foreground hover:bg-muted/50 hover:text-foreground",
                        )}
                      >
                        {type}
                      </button>
                    ))}
                    <div className="my-2 border-t border-dashed border-border/50" />
                  </div>
                ))}
              </div>
            </div>
            <div className="w-48 space-y-4">
              <div>
                <Label className="text-xs text-muted-foreground mb-1.5 block">
                  Array dimensions
                </Label>
                <Input
                  value={arrayDimensions}
                  onChange={(e) =>
                    setArrayDimensions(Number(e.target.value) || 0)
                  }
                  className="h-9 bg-background/50"
                  type="number"
                  min={0}
                />
              </div>
              {(selectedType === "varchar" || selectedType === "char") && (
                <div>
                  <Label className="text-xs text-muted-foreground mb-1.5 block">
                    Length
                  </Label>
                  <Input
                    value={typeLength}
                    onChange={(e) => setTypeLength(Number(e.target.value) || 0)}
                    className="h-9 bg-background/50"
                    type="number"
                    min={1}
                  />
                </div>
              )}
            </div>
          </div>
        );

      case "constraints":
        return (
          <div className="space-y-6">
            <div className="space-y-2">
              <div className="flex items-center gap-3">
                <span className="text-sm font-medium">Not null</span>
                <Switch checked={notNull} onCheckedChange={setNotNull} />
              </div>
              <p className="text-xs text-muted-foreground">
                Column must not assume the null value
              </p>
            </div>
            <div className="space-y-2">
              <div className="flex items-center gap-3">
                <span className="text-sm font-medium">Primary key</span>
                <Switch
                  checked={isPrimaryKey}
                  onCheckedChange={setIsPrimaryKey}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                Can be used as a unique identifier for rows in the table
              </p>
            </div>
            <div className="space-y-2">
              <div className="flex items-center gap-3">
                <span className="text-sm font-medium">Unique</span>
                <Switch checked={isUnique} onCheckedChange={setIsUnique} />
              </div>
              <p className="text-xs text-muted-foreground">
                Ensure that the data contained in a column is unique among all
                the rows
              </p>
            </div>
          </div>
        );

      case "default":
        return (
          <div className="space-y-4">
            <div>
              <Label className="text-xs text-muted-foreground mb-1.5 block">
                Default value expression
              </Label>
              <textarea
                value={defaultValue}
                onChange={(e) => setDefaultValue(e.target.value)}
                placeholder="Write your SQL query here..."
                className="w-full h-32 px-3 py-2 text-sm font-mono bg-background/50 border border-input rounded-md resize-none focus:outline-none focus:ring-2 focus:ring-ring"
              />
            </div>
          </div>
        );

      case "generated":
        return (
          <div className="space-y-4">
            <div className="space-y-2">
              <div className="flex items-center gap-3">
                <span className="text-sm font-medium">Is generated</span>
                <Switch
                  checked={isGenerated}
                  onCheckedChange={setIsGenerated}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                Column value is computed from an expression
              </p>
            </div>
            {isGenerated && (
              <div>
                <Label className="text-xs text-muted-foreground mb-1.5 block">
                  Generated expression
                </Label>
                <textarea
                  value={generatedExpression}
                  onChange={(e) => setGeneratedExpression(e.target.value)}
                  placeholder="Write your SQL expression here..."
                  className="w-full h-32 px-3 py-2 text-sm font-mono bg-background/50 border border-input rounded-md resize-none focus:outline-none focus:ring-2 focus:ring-ring"
                />
              </div>
            )}
          </div>
        );

      default:
        return null;
    }
  };

  return (
    <div className="border-b border-border/50 last:border-b-0">
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center justify-between px-4 py-3 text-left hover:bg-muted/30 transition-colors"
      >
        <div className="flex items-center gap-2">
          <input
            type="checkbox"
            className="size-4 rounded border-border"
            onClick={(e) => e.stopPropagation()}
          />
          {hasChange && change.newName && change.newName !== originalName ? (
            <>
              <span className="font-mono text-sm text-yellow-400 line-through">
                {originalName}
              </span>
              <span className="font-mono text-sm text-muted-foreground">→</span>
              <span className="font-mono text-sm text-yellow-400">
                {change.newName}
              </span>
            </>
          ) : (
            <span className="font-mono text-sm text-foreground">
              {displayName}
            </span>
          )}
          <span className="font-mono text-sm text-blue-400">
            {column.dataType}
          </span>
          {constraintParts.length > 0 && (
            <span className="font-mono text-sm text-rose-400">
              {constraintParts.join(" ")}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          {isExpanded ? (
            <IconChevronUp className="size-4 text-muted-foreground" />
          ) : (
            <IconChevronDown className="size-4 text-muted-foreground" />
          )}
          {hasChange && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onUndo?.();
              }}
              className="p-1 hover:bg-muted rounded transition-colors"
              title="Undo changes"
            >
              <IconArrowBack className="size-4 text-muted-foreground" />
            </button>
          )}
        </div>
      </button>

      {isExpanded && (
        <div className="grid grid-cols-[200px_1fr] border-t border-border/30 bg-muted/20">
          {/* Left sidebar - tabs */}
          <div className="border-r border-border/30 py-2">
            {tabs.map((tab) => (
              <button
                key={tab.id}
                type="button"
                onClick={() => setActiveTab(tab.id)}
                className={cn(
                  "w-full text-left px-4 py-2.5 text-sm transition-colors",
                  activeTab === tab.id
                    ? "bg-muted/50 text-foreground"
                    : "text-muted-foreground hover:bg-muted/30 hover:text-foreground",
                )}
              >
                {tab.label}
              </button>
            ))}
          </div>

          {/* Right content area */}
          <div className="p-4 min-h-50">{renderTabContent()}</div>
        </div>
      )}
    </div>
  );
}

interface SQLStatement {
  id: string;
  sql: string;
  type: "ALTER" | "CREATE" | "DROP";
}

function generateSQLStatements(
  tableName: string,
  schema: string,
  changes: Map<string, ColumnChange>,
): SQLStatement[] {
  const statements: SQLStatement[] = [];

  changes.forEach((change, columnName) => {
    if (change.newName && change.newName !== change.originalName) {
      statements.push({
        id: `rename-${columnName}`,
        type: "ALTER",
        sql: `ALTER TABLE "${schema}"."${tableName}"\nRENAME COLUMN "${change.originalName}" TO "${change.newName}";`,
      });
    }

    if (change.newType && change.newType !== change.originalType) {
      statements.push({
        id: `type-${columnName}`,
        type: "ALTER",
        sql: `ALTER TABLE "${schema}"."${tableName}"\nALTER COLUMN "${change.newName ?? change.originalName}" TYPE ${change.newType};`,
      });
    }

    if (
      change.newNullable !== undefined &&
      change.newNullable !== change.originalNullable
    ) {
      const action = change.newNullable ? "DROP NOT NULL" : "SET NOT NULL";
      statements.push({
        id: `nullable-${columnName}`,
        type: "ALTER",
        sql: `ALTER TABLE "${schema}"."${tableName}"\nALTER COLUMN "${change.newName ?? change.originalName}" ${action};`,
      });
    }
  });

  return statements;
}

function ReviewCommitView({
  tableName,
  schema,
  changes,
  onBack,
  onCommit,
}: {
  tableName: string;
  schema: string;
  changes: Map<string, ColumnChange>;
  onBack: () => void;
  onCommit: () => void;
}) {
  const statements = generateSQLStatements(tableName, schema, changes);

  return (
    <div className="flex flex-col h-full p-6">
      <div className="flex items-center justify-end gap-4 mb-6">
        <button
          type="button"
          onClick={onBack}
          className="text-sm text-muted-foreground hover:text-foreground transition-colors"
        >
          Back to schema
        </button>
        <button
          type="button"
          onClick={onCommit}
          className="h-10 px-4 text-sm font-medium bg-foreground text-background rounded-md hover:bg-foreground/90 transition-colors"
        >
          Commit changes
        </button>
      </div>

      <div className="space-y-4">
        {statements.length === 0 ? (
          <div className="text-center text-muted-foreground py-12">
            No changes to commit
          </div>
        ) : (
          statements.map((statement) => (
            <div
              key={statement.id}
              className="flex gap-4 rounded-lg border border-border/50 bg-card/50 p-4"
            >
              <div className="pt-1">
                <div className="size-3 rounded-full bg-muted-foreground/50" />
              </div>
              <pre className="flex-1 font-mono text-sm whitespace-pre-wrap">
                {statement.sql.split(/\b/).map((word, i) => {
                  const keywords = [
                    "ALTER",
                    "TABLE",
                    "RENAME",
                    "COLUMN",
                    "TO",
                    "TYPE",
                    "SET",
                    "DROP",
                    "NOT",
                    "NULL",
                    "CREATE",
                    "INDEX",
                    "ON",
                    "USING",
                  ];
                  if (keywords.includes(word.toUpperCase())) {
                    return (
                      // biome-ignore lint/suspicious/noArrayIndexKey: using index as key
                      <span key={i} className="text-rose-400">
                        {word}
                      </span>
                    );
                  }
                  // biome-ignore lint/suspicious/noArrayIndexKey: using index as key
                  return <span key={i}>{word}</span>;
                })}
              </pre>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

export function TableStructureView({
  data,
  className,
  onDiscard,
  onCommit,
}: TableStructureViewProps) {
  const [viewMode, setViewMode] = useState<ViewMode>("structure");
  const [expandedColumns, setExpandedColumns] = useState<Set<string>>(
    new Set(),
  );
  const [rowLevelSecurity, setRowLevelSecurity] = useState(
    data.rowLevelSecurity,
  );
  const [columnChanges, setColumnChanges] = useState<Map<string, ColumnChange>>(
    new Map(),
  );

  const toggleColumn = (columnName: string) => {
    setExpandedColumns((prev) => {
      const next = new Set(prev);
      if (next.has(columnName)) {
        next.delete(columnName);
      } else {
        next.add(columnName);
      }
      return next;
    });
  };

  const handleColumnChange = (
    originalName: string,
    change: Partial<ColumnChange>,
  ) => {
    setColumnChanges((prev) => {
      const next = new Map(prev);
      const existing = next.get(originalName);

      const updated: ColumnChange = {
        originalName,
        ...existing,
        ...change,
      };

      // Check if there are any actual changes
      const hasChanges =
        (updated.newName && updated.newName !== originalName) ||
        (updated.newType && updated.newType !== updated.originalType) ||
        updated.newNullable !== undefined ||
        updated.newPrimaryKey !== undefined ||
        updated.newUnique !== undefined ||
        updated.newDefault !== undefined;

      if (hasChanges) {
        next.set(originalName, updated);
      } else {
        next.delete(originalName);
      }

      return next;
    });
  };

  const handleUndoColumn = (originalName: string) => {
    setColumnChanges((prev) => {
      const next = new Map(prev);
      next.delete(originalName);
      return next;
    });
  };

  const handleDiscard = () => {
    setColumnChanges(new Map());
    onDiscard?.();
  };

  const handleReviewAndCommit = () => {
    setViewMode("review");
  };

  const handleBackToSchema = () => {
    setViewMode("structure");
  };

  const handleCommit = () => {
    onCommit?.();
    setColumnChanges(new Map());
    setViewMode("structure");
  };

  if (viewMode === "review") {
    return (
      <ReviewCommitView
        tableName={data.tableName}
        schema={data.schema}
        changes={columnChanges}
        onBack={handleBackToSchema}
        onCommit={handleCommit}
      />
    );
  }

  return (
    <div className={cn("flex flex-col h-full overflow-auto p-6", className)}>
      <div className="max-w-4xl space-y-8">
        {/* Table Header */}
        <div className="flex items-center gap-4">
          <div className="space-y-1.5">
            <Label className="text-xs text-muted-foreground">Table name</Label>
            <Input
              defaultValue={data.tableName}
              className="h-10 w-48 bg-background/50 font-medium"
            />
          </div>

          <div className="space-y-1.5">
            <Label className="text-xs text-muted-foreground">Schema</Label>
            <Select defaultValue={data.schema}>
              <SelectTrigger className="h-10 w-32 bg-background/50">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="public">public</SelectItem>
                <SelectItem value="private">private</SelectItem>
                <SelectItem value="auth">auth</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="flex items-center gap-3 rounded-lg border border-border/50 bg-background/50 px-4 py-2.5">
            <IconLock className="size-4 text-muted-foreground" />
            <span className="text-sm">Row Level Security</span>
            <Switch
              checked={rowLevelSecurity}
              onCheckedChange={setRowLevelSecurity}
            />
          </div>

          <div className="flex-1" />

          <button
            type="button"
            onClick={handleDiscard}
            className="text-sm text-muted-foreground hover:text-foreground transition-colors"
          >
            Discard changes
          </button>

          <button
            type="button"
            onClick={handleReviewAndCommit}
            className="h-10 px-4 text-sm font-medium bg-foreground text-background rounded-md hover:bg-foreground/90 transition-colors"
          >
            Review and commit
          </button>
        </div>

        {/* Columns Section */}
        <section>
          <div className="flex items-center gap-3 mb-3">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Columns
            </h3>
            <button
              type="button"
              className="text-xs text-blue-400 hover:text-blue-300 transition-colors"
            >
              Add column
            </button>
          </div>
          <div className="rounded-lg border border-border/50 bg-card/50 overflow-hidden">
            {data.columns.map((column) => (
              <ExpandableColumn
                key={column.name}
                column={column}
                isExpanded={expandedColumns.has(column.name)}
                onToggle={() => toggleColumn(column.name)}
                change={columnChanges.get(column.name)}
                onUndo={() => handleUndoColumn(column.name)}
                onColumnChange={(change) =>
                  handleColumnChange(column.name, change)
                }
              />
            ))}
          </div>
        </section>

        {/* Constraints Section */}
        <section>
          <div className="flex items-center gap-3 mb-3">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Constraints
            </h3>
            <button
              type="button"
              className="text-xs text-blue-400 hover:text-blue-300 transition-colors"
            >
              Add constraint
            </button>
          </div>
          <div className="rounded-lg border border-border/50 bg-card/50 overflow-hidden">
            {data.constraints.length === 0 ? (
              <div className="px-4 py-3 text-sm text-muted-foreground">
                No constraints defined
              </div>
            ) : (
              data.constraints.map((constraint) => (
                <div
                  key={constraint.name}
                  className="flex items-center gap-2 px-4 py-3 border-b border-border/50 last:border-b-0"
                >
                  <span className="font-mono text-sm text-purple-400">
                    CONSTRAINT
                  </span>
                  <span className="font-mono text-sm text-foreground">
                    {constraint.name}
                  </span>
                  <span className="font-mono text-sm text-purple-400">
                    {constraint.type}
                  </span>
                  <span className="font-mono text-sm text-foreground">
                    ({constraint.columns.join(", ")})
                  </span>
                </div>
              ))
            )}
          </div>
        </section>

        {/* Indexes Section */}
        <section>
          <div className="flex items-center gap-3 mb-3">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Indexes
            </h3>
            <button
              type="button"
              className="text-xs text-blue-400 hover:text-blue-300 transition-colors"
            >
              Add index
            </button>
          </div>
          <div className="rounded-lg border border-border/50 bg-card/50 overflow-hidden">
            {data.indexes.length === 0 ? (
              <div className="px-4 py-3 text-sm text-muted-foreground">
                No indexes defined
              </div>
            ) : (
              data.indexes.map((index) => (
                <div
                  key={index.name}
                  className="flex items-center gap-2 px-4 py-3 border-b border-border/50 last:border-b-0"
                >
                  {index.isUnique && (
                    <span className="font-mono text-sm text-rose-400">
                      UNIQUE
                    </span>
                  )}
                  <span className="font-mono text-sm text-rose-400">INDEX</span>
                  <span className="font-mono text-sm text-foreground">
                    {index.name}
                  </span>
                  <span className="font-mono text-sm text-muted-foreground">
                    …
                  </span>
                  <span className="font-mono text-sm text-purple-400">
                    USING
                  </span>
                  <span className="font-mono text-sm text-foreground">
                    {index.method}
                  </span>
                  <span className="font-mono text-sm text-foreground">
                    ({index.columns.join(", ")})
                  </span>
                </div>
              ))
            )}
          </div>
        </section>

        {/* Policies Section */}
        <section>
          <div className="flex items-center gap-3 mb-3">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Policies
            </h3>
            <button
              type="button"
              className="text-xs text-blue-400 hover:text-blue-300 transition-colors"
            >
              Add policy
            </button>
          </div>
          <div className="rounded-lg border border-border/50 bg-card/50 overflow-hidden">
            {data.policies.length === 0 ? (
              <div className="px-4 py-3 text-sm text-muted-foreground">
                No policies defined
              </div>
            ) : (
              data.policies.map((policy) => (
                <div
                  key={policy.name}
                  className="flex items-center gap-2 px-4 py-3 border-b border-border/50 last:border-b-0"
                >
                  <span className="font-mono text-sm text-purple-400">
                    POLICY
                  </span>
                  <span className="font-mono text-sm text-foreground">
                    {policy.name}
                  </span>
                  <span className="font-mono text-sm text-purple-400">FOR</span>
                  <span className="font-mono text-sm text-foreground">
                    {policy.command}
                  </span>
                  <span className="font-mono text-sm text-purple-400">TO</span>
                  <span className="font-mono text-sm text-foreground">
                    {policy.roles.join(", ")}
                  </span>
                </div>
              ))
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
