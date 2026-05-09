import MonacoEditor from "@monaco-editor/react";
import {
  IconDatabase,
  IconDeviceFloppy,
  IconPlayerPlay,
  IconSearch,
  IconTerminal2,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useMemo, useRef } from "react";
import { DataGrid } from "@/components/data-grid";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  type QueryPreviewData,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";

// Minimal shape we need from the Monaco editor instance. Avoids pulling the
// full monaco-editor types in (the package isn't installed for runtime use).
type MonacoEditorInstance = {
  getSelection: () => unknown;
  getModel: () => { getValueInRange: (range: unknown) => string } | null;
};

interface QueryEditorPanelProps {
  tab: WorkspaceTab;
  isClient: boolean;
}

export function QueryEditorPanel({ tab, isClient }: QueryEditorPanelProps) {
  const {
    queryPreviews,
    queryEdits,
    editorTheme,
    updateQuery,
    runQuery,
    setQueryEdit,
    discardQueryEdits,
  } = useAppStore();

  const activeQueryPreview: QueryPreviewData | null = useMemo(() => {
    if (tab.kind !== "query") {
      return null;
    }
    return (
      queryPreviews[tab.label] ?? {
        columns: ["column"],
        rows: [],
        runtime: "--",
        rowCount: "0",
        cache: "Cold",
      }
    );
  }, [tab, queryPreviews]);

  const currentEdits = useMemo(
    () => queryEdits[tab.id] ?? {},
    [queryEdits, tab.id],
  );

  const editorRef = useRef<MonacoEditorInstance | null>(null);

  const getEditorSelectionText = useCallback((): string => {
    const editor = editorRef.current;
    if (!editor) {
      return "";
    }
    const selection = editor.getSelection();
    const model = editor.getModel();
    if (!selection || !model) {
      return "";
    }
    return model.getValueInRange(selection) ?? "";
  }, []);

  const handleRun = useCallback(() => {
    void runQuery(tab.id, { overrideSql: getEditorSelectionText() });
  }, [getEditorSelectionText, runQuery, tab.id]);

  const hasEdits = Object.keys(currentEdits).length > 0;

  const editorOptions = useMemo(
    () =>
      ({
        minimap: { enabled: false },
        fontSize: 13,
        fontFamily: "JetBrains Mono Variable, monospace",
        scrollBeyondLastLine: false,
        wordWrap: "on" as const,
        lineNumbersMinChars: 3,
        padding: { top: 12, bottom: 12 },
        renderLineHighlight: "none",
        overviewRulerBorder: false,
        hideCursorInOverviewRuler: true,
        scrollbar: {
          vertical: "visible",
          horizontal: "visible",
          useShadows: false,
        },
      }) as const,
    [],
  );

  return (
    <div className="flex h-full flex-col bg-background">
      {/* Top Toolbar */}
      <div className="flex h-10 shrink-0 items-center justify-between border-b px-3">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <IconTerminal2 className="size-4" />
            <span className="font-medium text-foreground">{tab.label}</span>
          </div>
          <div className="h-4 w-px bg-border" />
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <IconDatabase className="size-3" />
            <span>{tab.schema}</span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {hasEdits && (
            <>
              <Button
                size="sm"
                variant="ghost"
                className="h-7 px-2 text-xs"
                onClick={() => discardQueryEdits(tab.id)}
              >
                <IconX className="mr-1 size-3.5" /> Discard
              </Button>
              <Button size="sm" className="h-7 px-2 text-xs">
                <IconDeviceFloppy className="mr-1 size-3.5" /> Save changes
              </Button>
              <div className="h-4 w-px bg-border" />
            </>
          )}

          <Button
            size="sm"
            variant="secondary"
            className="h-7 px-3 text-xs shadow-none"
            onClick={handleRun}
          >
            <IconPlayerPlay className="mr-1.5 size-3.5" />
            Run
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="h-7 px-3 text-xs shadow-none"
          >
            <IconSearch className="mr-1.5 size-3.5" />
            Explain
          </Button>
        </div>
      </div>

      {/* Split Pane Container */}
      <div className="flex min-h-0 flex-1 flex-col">
        {/* Editor Section - Fixed Height for now */}
        <div className="relative h-60 shrink-0 border-b">
          {isClient ? (
            <MonacoEditor
              height="100%"
              language="sql"
              theme={editorTheme}
              value={tab.query ?? ""}
              options={editorOptions}
              onChange={(value) => updateQuery(tab.id, value ?? "")}
              onMount={(editor) => {
                editorRef.current = editor as MonacoEditorInstance;
              }}
            />
          ) : (
            <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
              Loading editor...
            </div>
          )}
        </div>

        {/* Results Section */}
        <div className="flex min-h-0 flex-1 flex-col bg-background max-w-[calc(100vw-16rem)]">
          {/* Results Info Bar */}
          <div className="flex h-8 shrink-0 items-center gap-2 border-b bg-muted/20 px-2">
            <Badge
              variant="outline"
              className="h-5 rounded-sm border-transparent bg-transparent px-1 text-[0.65rem] font-normal text-muted-foreground hover:bg-muted"
            >
              {activeQueryPreview?.rowCount ?? 0} rows
            </Badge>
            <div className="h-3 w-px bg-border" />
            <Badge
              variant="outline"
              className="h-5 rounded-sm border-transparent bg-transparent px-1 text-[0.65rem] font-normal text-muted-foreground hover:bg-muted"
            >
              {activeQueryPreview?.runtime ?? "--"}
            </Badge>
            <div className="h-3 w-px bg-border" />
            <Badge
              variant="outline"
              className="h-5 rounded-sm border-transparent bg-transparent px-1 text-[0.65rem] font-normal text-muted-foreground hover:bg-muted"
            >
              ReadOnly
            </Badge>
          </div>

          {/* Data Grid */}
          <div className="flex-1 overflow-hidden max-w-[calc(100vw-16rem)]">
            {activeQueryPreview?.rows.length ? (
              <DataGrid
                data={activeQueryPreview?.rows ?? []}
                columns={activeQueryPreview?.columns ?? []}
                edits={currentEdits}
                onEdit={(rowIndex, colIndex, value) =>
                  setQueryEdit(tab.id, rowIndex, colIndex, value)
                }
              />
            ) : (
              <div className="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
                <div className="rounded-full bg-muted p-3">
                  <IconTerminal2 className="size-6 opacity-50" />
                </div>
                <div className="text-xs">Run the query to see results</div>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
