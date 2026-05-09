import { useEffect, useState } from "react";
import { ConnectionsView } from "@/components/connections-view";
import { Sidebar } from "@/components/sidebar";
import { WorkspaceView } from "@/components/workspace-view";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

export function AppShell() {
  const [isClient, setIsClient] = useState(false);

  const {
    activeView,
    isLeftSidebarOpen,
    setEditorTheme,
    loadConnections,
    loadQueryHistory,
  } = useAppStore();

  useEffect(() => {
    setIsClient(true);
    if (typeof document === "undefined") {
      return;
    }
    setEditorTheme(
      document.documentElement.classList.contains("dark") ? "vs-dark" : "vs",
    );
  }, [setEditorTheme]);

  useEffect(() => {
    void loadConnections();
    void loadQueryHistory();
  }, [loadConnections, loadQueryHistory]);

  return (
    <div className="flex h-screen w-screen bg-background text-foreground">
      <div
        className={cn(
          "shrink-0 overflow-hidden transition-all duration-300 ease-in-out",
          isLeftSidebarOpen ? "w-65" : "w-0",
        )}
      >
        <Sidebar />
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        {activeView === "workspace" ? (
          <WorkspaceView isClient={isClient} />
        ) : (
          <ConnectionsView />
        )}
      </div>
    </div>
  );
}
