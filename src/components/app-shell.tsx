import { useEffect, useRef, useState } from "react";

import { useTauriWindowControls } from "@/components/app-shell/use-tauri-window-controls";
import { windowViewportZoomStyle } from "@/components/app-shell/use-window-viewport-zoom";
import {
  WindowDragFrame,
  WindowDragSurface,
} from "@/components/app-shell/window-drag";
import { CommandPalette } from "@/components/command-palette/command-palette";
import { ConfirmDialogHost } from "@/components/confirm-dialog";
import {
  CredentialOnboarding,
  CredentialUnlock,
} from "@/components/credential-onboarding";
import { SafetyConfirmDialog } from "@/components/safety-confirm-dialog";
import { SettingsView } from "@/components/settings-view";
import { KeyValueWorkbench } from "@/components/workbench/keyvalue-workbench";
import { RelationalWorkbench } from "@/components/workbench/relational-workbench";
import { TabShortcuts } from "@/components/workbench/tab-shortcuts";
import { isKeyValueConnection } from "@/components/workbench/workbench-policy";
import { startSessionPersistence } from "@/lib/session-persistence";
import { useAppStore } from "@/lib/store";
import { applyTheme, subscribeSystem } from "@/lib/theme";
import { initUiState, isUiStateReady } from "@/lib/ui-state";
import { cn } from "@/lib/utils";

/**
 * Foreground health-check tick: runs once after the connection list loads,
 * then every 30 s while the tab is visible. Pauses when the user switches
 * tabs to avoid burning credentials checks against a backgrounded app.
 */
function useForegroundHealthCheck(
  credentialState: string | undefined,
  // oxlint-disable-next-line anti-slop/no-unknown-returns -- The value is handled at a typed library or domain boundary here.
  runHealthChecks: () => Promise<unknown>,
) {
  useEffect(() => {
    if (credentialState !== "ready") {
      return;
    }
    let timer: ReturnType<typeof setInterval> | null = null;
    const start = () => {
      if (timer) return;
      // Run once immediately so the UI doesn't sit on stale cached statuses
      // for 30 s after launch.
      void runHealthChecks();
      timer = setInterval(() => {
        void runHealthChecks();
      }, 30_000);
    };
    const stop = () => {
      if (timer) {
        clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") start();
      else stop();
    };
    if (document.visibilityState === "visible") {
      start();
    }
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stop();
    };
  }, [credentialState, runHealthChecks]);
}

export function AppShell() {
  const [isClient, setIsClient] = useState(false);
  // P8: the SQLite-backed UI-state cache must be loaded before the
  // workbench mounts (panel sizes, grid layouts, and the session
  // restore all read it synchronously). In a plain browser the store
  // passes through to localStorage and is ready immediately.
  const [uiStateReady, setUiStateReady] = useState(() => isUiStateReady());
  useEffect(() => {
    if (uiStateReady) return;
    let cancelled = false;
    void initUiState().then(() => {
      if (!cancelled) setUiStateReady(true);
    });
    return () => {
      cancelled = true;
    };
  }, [uiStateReady]);

  const {
    isWindowFullscreen,
    windowViewportZoom,
    onTopBarPointerDown,
    onTopBarDoubleClick,
  } = useTauriWindowControls();

  const {
    activeView,
    activeConnectionId,
    connections,
    appSettings,
    appSettingsStatus,
    setEditorTheme,
    loadAppSettings,
    loadBastionServers,
    loadConnections,
    loadManagedServers,
    loadQueryHistory,
    loadSavedQueries,
    runHealthChecks,
  } = useAppStore();
  const themeMode = appSettings?.theme ?? "system";
  const themePreset = appSettings?.themePreset ?? "default";

  const activeConnection = connections.find(
    (connection) => connection.id === activeConnectionId,
  );

  useEffect(() => {
    setIsClient(true);
  }, []);

  // Mirror `<html>.dark` onto Monaco's theme. Bail when the resolved
  // value hasn't changed so unrelated class mutations don't trigger a
  // store write + re-render storm.
  useEffect(() => {
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
    if (typeof document === "undefined") return;
    let last: "vs" | "vs-dark" | null = null;
    const sync = () => {
      const next = document.documentElement.classList.contains("dark")
        ? "vs-dark"
        : "vs";
      if (next === last) return;
      last = next;
      setEditorTheme(next);
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });
    return () => observer.disconnect();
  }, [setEditorTheme]);

  // Follow OS theme when mode is "system". Preset is read through a
  // ref so changing presets doesn't tear down and re-add the matchMedia
  // listener — only mode actually matters for the subscription.
  const themePresetRef = useRef(themePreset);
  themePresetRef.current = themePreset;
  useEffect(() => {
    if (themeMode !== "system") return;
    return subscribeSystem(() => applyTheme("system", themePresetRef.current));
  }, [themeMode]);

  useEffect(() => {
    void loadAppSettings();
  }, [loadAppSettings]);

  const sessionRestoredRef = useRef(false);
  useEffect(() => {
    if (appSettings?.credentialState !== "ready") {
      return;
    }
    // P8 session restore: connections and the UI-state cache must both
    // be in place before rebuilding tabs; persistence starts only after
    // restore so an empty boot state can't clobber the stored session.
    void (async () => {
      await loadConnections();
      await initUiState();
      if (!sessionRestoredRef.current) {
        sessionRestoredRef.current = true;
        useAppStore.getState().restoreSession();
        startSessionPersistence();
      }
    })();
    void loadBastionServers();
    void loadManagedServers();
    void loadQueryHistory();
    void loadSavedQueries();
  }, [
    appSettings?.credentialState,
    loadBastionServers,
    loadConnections,
    loadManagedServers,
    loadQueryHistory,
    loadSavedQueries,
  ]);

  useForegroundHealthCheck(appSettings?.credentialState, runHealthChecks);

  if (
    appSettingsStatus.state === "loading" ||
    appSettingsStatus.state === "idle" ||
    !uiStateReady
  ) {
    return (
      <div className="fixed inset-0 flex items-center justify-center bg-surface-app text-xs text-text-muted">
        <WindowDragSurface
          onDoubleClick={onTopBarDoubleClick}
          onPointerDown={onTopBarPointerDown}
        />
        Loading settings…
      </div>
    );
  }

  if (appSettingsStatus.state === "error") {
    return (
      <div className="fixed inset-0 flex items-center justify-center bg-surface-app p-6 text-foreground">
        <WindowDragSurface
          onDoubleClick={onTopBarDoubleClick}
          onPointerDown={onTopBarPointerDown}
        />
        <div className="max-w-md rounded-lg border border-danger/30 bg-danger/10 p-5 text-sm text-danger">
          {appSettingsStatus.error}
        </div>
      </div>
    );
  }

  if (appSettings?.credentialState === "needs-onboarding") {
    return (
      <WindowDragFrame
        onDoubleClick={onTopBarDoubleClick}
        onPointerDown={onTopBarPointerDown}
      >
        <CredentialOnboarding />
      </WindowDragFrame>
    );
  }

  if (appSettings?.credentialState === "needs-unlock") {
    return (
      <WindowDragFrame
        onDoubleClick={onTopBarDoubleClick}
        onPointerDown={onTopBarPointerDown}
      >
        <CredentialUnlock />
      </WindowDragFrame>
    );
  }

  return (
    <div
      data-testid="app-shell"
      data-window-viewport-zoom={
        windowViewportZoom
          ? windowViewportZoom.active
            ? "active"
            : "idle"
          : undefined
      }
      style={windowViewportZoomStyle(windowViewportZoom)}
      className={cn(
        "fixed flex min-h-0 min-w-0 flex-col overflow-hidden bg-surface-app text-foreground",
        windowViewportZoom ? "top-0 left-0" : "inset-0",
      )}
    >
      <div className="relative flex min-h-0 flex-1 overflow-hidden">
        {activeView === "settings" ||
        !isKeyValueConnection(activeConnection) ? (
          <RelationalWorkbench
            isClient={isClient}
            isWindowFullscreen={isWindowFullscreen}
            onPointerDown={onTopBarPointerDown}
            onDoubleClick={onTopBarDoubleClick}
            settingsView={activeView === "settings" ? <SettingsView /> : null}
          />
        ) : (
          <KeyValueWorkbench
            isWindowFullscreen={isWindowFullscreen}
            onPointerDown={onTopBarPointerDown}
            onDoubleClick={onTopBarDoubleClick}
          />
        )}
      </div>
      <CommandPalette />
      <SafetyConfirmDialog />
      <ConfirmDialogHost />
      <TabShortcuts />
    </div>
  );
}
