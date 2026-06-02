import { IconPlus, IconSearch } from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";

import { BastionForm } from "@/components/bastion-servers/bastion-form";
import { BastionRow } from "@/components/bastion-servers/bastion-row";
import {
  analyzePrivateKeyContent,
  bastionReferenceCounts,
  blankDraft,
  draftFromBastion,
  filterBastions,
  normalizeDraft,
} from "@/components/bastion-servers/helpers";
import type {
  BastionDraft,
  HostKeyResetState,
  TestState,
} from "@/components/bastion-servers/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { BastionServer } from "@/lib/store";
import { useAppStore } from "@/lib/store";

export { analyzePrivateKeyContent };

export function BastionServersTab() {
  const bastions = useAppStore((state) => state.bastionServers);
  const connections = useAppStore((state) => state.connections);
  const status = useAppStore((state) => state.bastionStatus);
  const loadBastionServers = useAppStore((state) => state.loadBastionServers);
  const saveBastionServer = useAppStore((state) => state.saveBastionServer);
  const deleteBastionServer = useAppStore((state) => state.deleteBastionServer);
  const resetBastionHostKey = useAppStore((state) => state.resetBastionHostKey);
  const testBastionServer = useAppStore((state) => state.testBastionServer);

  const [draft, setDraft] = useState<BastionDraft>(() => blankDraft());
  const [editingId, setEditingId] = useState<string | null>(null);
  const [testState, setTestState] = useState<TestState>({ state: "idle" });
  const [search, setSearch] = useState("");
  const [hostKeyReset, setHostKeyReset] = useState<HostKeyResetState>(null);

  useEffect(() => {
    if (status.state === "idle") {
      void loadBastionServers();
    }
  }, [loadBastionServers, status.state]);

  const editing = useMemo(
    () => bastions.find((bastion) => bastion.id === editingId) ?? null,
    [bastions, editingId],
  );
  const filteredBastions = useMemo(
    () => filterBastions(bastions, search),
    [bastions, search],
  );
  const referenceCounts = useMemo(
    () => bastionReferenceCounts(bastions, connections),
    [bastions, connections],
  );

  const startNew = () => {
    setEditingId(null);
    setDraft(blankDraft());
    setHostKeyReset(null);
  };

  const startEdit = (bastion: BastionServer) => {
    setEditingId(bastion.id);
    setDraft(draftFromBastion(bastion));
    setHostKeyReset(null);
  };

  const handleSave = async () => {
    const ok = await saveBastionServer(normalizeDraft(draft));
    if (ok) {
      startNew();
    }
  };

  const handleDelete = async (bastion: BastionServer) => {
    if (
      window.confirm(
        `Delete Bastion Server "${bastion.name}"? Connections that reference it must be changed first.`,
      )
    ) {
      const ok = await deleteBastionServer(bastion.id);
      if (ok && editingId === bastion.id) {
        startNew();
      }
    }
  };

  const requestHostKeyReset = (bastion: BastionServer) => {
    setHostKeyReset({ id: bastion.id, confirmHost: "" });
  };

  const confirmHostKeyReset = async (bastion: BastionServer) => {
    if (hostKeyReset?.id !== bastion.id) {
      return;
    }
    if (hostKeyReset.confirmHost !== bastion.host) {
      return;
    }
    const ok = await resetBastionHostKey(bastion.id);
    if (ok) {
      setHostKeyReset(null);
    }
  };

  const handleTest = async (bastion: BastionServer) => {
    setTestState({ state: "running", id: bastion.id });
    const result = await testBastionServer(bastion.id);
    setTestState(
      result.ok
        ? { state: "success", id: bastion.id, latencyMs: result.latencyMs }
        : { state: "error", id: bastion.id, error: result.error },
    );
  };

  return (
    <>
      <SectionHeader
        title="Bastion Servers"
        subtitle="Reusable SSH bastions for database connections that need a local tunnel."
      />
      <div className="min-h-0 flex-1 overflow-auto p-6">
        <div className="mx-auto grid max-w-5xl gap-5 lg:grid-cols-[minmax(0,1fr)_22rem]">
          <section className="min-h-0 rounded-lg border border-border-subtle bg-surface-window">
            <div className="grid gap-3 border-b border-border-subtle px-4 py-3">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <h2 className="text-sm font-semibold text-foreground">
                    Saved Bastion Servers
                  </h2>
                  <p className="mt-1 text-xs text-text-muted">
                    {filteredBastions.length === bastions.length
                      ? `${bastions.length} saved. Deletion is blocked while a Connection references the server.`
                      : `Showing ${filteredBastions.length} of ${bastions.length}.`}
                  </p>
                </div>
                <Button type="button" variant="outline" onClick={startNew}>
                  <IconPlus className="size-3.5" />
                  New
                </Button>
              </div>
              <div className="relative">
                <IconSearch className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-text-muted" />
                <Input
                  aria-label="Search Bastion Servers"
                  className="pl-7"
                  value={search}
                  placeholder="Search name, host, user, auth, or fingerprint"
                  onChange={(event) => setSearch(event.target.value)}
                />
              </div>
            </div>
            {bastions.length === 0 ? (
              <div className="px-4 py-8 text-center text-xs text-text-muted">
                No Bastion Servers saved.
              </div>
            ) : filteredBastions.length === 0 ? (
              <div className="px-4 py-8 text-center text-xs text-text-muted">
                No Bastion Servers match this search.
              </div>
            ) : (
              <div className="divide-y divide-border-subtle">
                {filteredBastions.map((bastion) => (
                  <BastionRow
                    key={bastion.id}
                    bastion={bastion}
                    active={bastion.id === editingId}
                    referenceCount={referenceCounts[bastion.id] ?? 0}
                    hostKeyReset={
                      hostKeyReset?.id === bastion.id ? hostKeyReset : null
                    }
                    testState={testState}
                    onEdit={() => startEdit(bastion)}
                    onTest={() => void handleTest(bastion)}
                    onRequestHostKeyReset={() => requestHostKeyReset(bastion)}
                    onHostKeyResetInput={(confirmHost) =>
                      setHostKeyReset({ id: bastion.id, confirmHost })
                    }
                    onConfirmHostKeyReset={() =>
                      void confirmHostKeyReset(bastion)
                    }
                    onCancelHostKeyReset={() => setHostKeyReset(null)}
                    onDelete={() => handleDelete(bastion)}
                  />
                ))}
              </div>
            )}
          </section>

          <BastionForm
            draft={draft}
            editing={editing}
            onChange={setDraft}
            onSave={handleSave}
            onCancel={startNew}
          />
        </div>
        {status.state === "error" ? (
          <div className="mx-auto mt-4 max-w-5xl rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger">
            {status.error}
          </div>
        ) : null}
      </div>
    </>
  );
}

function SectionHeader({
  title,
  subtitle,
}: {
  title: string;
  subtitle: string;
}) {
  return (
    <header className="shrink-0 border-b border-border-subtle bg-surface-window px-6 py-4">
      <h1 className="text-xl font-semibold tracking-tight text-foreground">
        {title}
      </h1>
      <p className="mt-1 text-xs text-text-muted">{subtitle}</p>
    </header>
  );
}
