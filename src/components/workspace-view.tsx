import {
  IconLayoutSidebarRight,
  IconTable,
  IconTerminal2,
} from "@tabler/icons-react";
import { useMemo, useState } from "react";
import { QueryEditorPanel } from "@/components/query-editor-panel";
import { QuerySidebar } from "@/components/query-sidebar";
import {
  TableEditorPanel,
  TableSidebar,
} from "@/components/table-editor-panel";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { WorkspaceTabs } from "@/components/workspace-tabs";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

interface WorkspaceViewProps {
  isClient: boolean;
}

export function WorkspaceView({ isClient }: WorkspaceViewProps) {
  const [isSidebarOpen, setIsSidebarOpen] = useState(false);

  const {
    activeConnectionId,
    activeTabId,
    connections,
    workspaceTabs,
    createNewQueryTab,
    createNewTableTab,
  } = useAppStore();

  const activeConnection = useMemo(
    () =>
      connections.find((connection) => connection.id === activeConnectionId) ??
      connections[0],
    [activeConnectionId, connections],
  );

  const activeTab = useMemo(
    () => workspaceTabs.find((tab) => tab.id === activeTabId),
    [activeTabId, workspaceTabs],
  );

  return (
    <>
      <header className="flex h-12 items-center justify-between border-b px-3">
        <div className="flex items-center gap-2">
          <div className="text-sm font-semibold">Workspace</div>
          <div className="text-xs text-muted-foreground">
            {activeConnection
              ? `/ ${activeConnection.name} / ${activeConnection.database}`
              : "/ No active connection"}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="secondary" onClick={createNewQueryTab}>
            <IconTerminal2 />
            New query
          </Button>
          <Button size="sm" variant="outline" onClick={createNewTableTab}>
            <IconTable />
            New table
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="size-8"
            onClick={() => setIsSidebarOpen(!isSidebarOpen)}
          >
            <IconLayoutSidebarRight className="size-4" />
          </Button>
        </div>
      </header>

      <WorkspaceTabs />

      <div className="flex min-h-0 flex-1">
        <div
          className={cn(
            "grid h-full min-h-0 flex-1 transition-all duration-300 ease-in-out",
            isSidebarOpen ? "grid-cols-[1fr_320px]" : "grid-cols-[1fr_0px]",
          )}
        >
          <section className="flex min-h-0 flex-col border-r">
            {activeTab ? (
              activeTab.kind === "query" ? (
                <QueryEditorPanel tab={activeTab} isClient={isClient} />
              ) : (
                <TableEditorPanel tab={activeTab} />
              )
            ) : (
              <div className="flex flex-1 items-center justify-center p-6">
                <Card className="border border-border">
                  <CardHeader>
                    <CardTitle>No tabs open</CardTitle>
                  </CardHeader>
                  <CardContent className="text-xs text-muted-foreground">
                    Select a table from the explorer or create a new query to
                    begin.
                  </CardContent>
                </Card>
              </div>
            )}
          </section>

          <aside
            className={cn(
              "flex min-h-0 flex-col overflow-auto bg-muted/20 transition-all duration-300 ease-in-out",
              isSidebarOpen ? "p-3 gap-2 opacity-100" : "w-0 p-0 opacity-0",
            )}
          >
            {activeTab ? (
              activeTab.kind === "query" ? (
                <QuerySidebar tab={activeTab} />
              ) : (
                <TableSidebar tab={activeTab} isClient={isClient} />
              )
            ) : (
              <Card size="sm" className="border border-border">
                <CardHeader>
                  <CardTitle>Workspace tips</CardTitle>
                </CardHeader>
                <CardContent className="text-xs text-muted-foreground">
                  Use the schema explorer to open a table or start a new query
                  from the toolbar.
                </CardContent>
              </Card>
            )}
          </aside>
        </div>
      </div>
    </>
  );
}
