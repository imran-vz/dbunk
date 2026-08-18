import {
  IconCheck,
  IconCopy,
  IconDatabasePlus,
  IconX,
} from "@tabler/icons-react";
import type React from "react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button, buttonVariants } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  type DockerStatus,
  type ProvisionManagedServerResult,
  useAppStore,
} from "@/lib/store";
import { isTauri, tauriInvoke } from "@/lib/tauri";
import { cn } from "@/lib/utils";

/** Keep in sync with the backend allowlists in `src-tauri/src/managed.rs`. */
// oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
const ENGINE_VERSIONS: Record<"PostgreSQL" | "MySQL", string[]> = {
  PostgreSQL: ["18", "17", "16", "15", "14"],
  MySQL: ["9", "8.4", "8.0"],
};

type DockerCheck =
  | { state: "checking" }
  | { state: "ready"; version: string | null }
  | { state: "unavailable"; error: string };

interface NewLocalDatabaseDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  trigger?: React.ReactElement;
}

export function NewLocalDatabaseDialog({
  open,
  onOpenChange,
  trigger,
}: NewLocalDatabaseDialogProps) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      {trigger ? (
        <AlertDialogTrigger render={trigger} />
      ) : (
        <AlertDialogTrigger
          aria-label="New local database"
          title="New local database"
          className={cn(
            buttonVariants({ variant: "outline", size: "icon-xs" }),
          )}
        >
          <IconDatabasePlus />
        </AlertDialogTrigger>
      )}
      <AlertDialogContent className="flex max-h-[88vh] w-[26rem] max-w-[26rem] flex-col gap-0 overflow-hidden rounded-xl border border-border-subtle bg-surface-window p-0 sm:max-w-[26rem]">
        <AlertDialogHeader className="flex-row items-center justify-between border-b border-border-subtle px-4 py-3">
          <AlertDialogTitle className="text-sm font-semibold">
            New local database
          </AlertDialogTitle>
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label="Close"
            onClick={() => onOpenChange(false)}
            className="size-7"
          >
            <IconX className="size-3.5" />
          </Button>
        </AlertDialogHeader>
        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
          {open ? (
            <NewLocalDatabaseBody onClose={() => onOpenChange(false)} />
          ) : null}
        </div>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function NewLocalDatabaseBody({ onClose }: { onClose: () => void }) {
  const provisionManagedServer = useAppStore((s) => s.provisionManagedServer);

  const [docker, setDocker] = useState<DockerCheck>({ state: "checking" });
  const [name, setName] = useState("");
  const [engine, setEngine] = useState<"PostgreSQL" | "MySQL">("PostgreSQL");
  const [version, setVersion] = useState(ENGINE_VERSIONS.PostgreSQL[0]);
  const [portText, setPortText] = useState("");
  const [isProvisioning, setIsProvisioning] = useState(false);
  const [provisionError, setProvisionError] = useState<string | null>(null);
  const [result, setResult] = useState<ProvisionManagedServerResult | null>(
    null,
  );

  useEffect(() => {
    let cancelled = false;
    if (!isTauri()) {
      setDocker({
        state: "unavailable",
        error: "Managed servers require the desktop runtime.",
      });
      return;
    }
    tauriInvoke<DockerStatus>("check_docker")
      .then((status) => {
        if (cancelled) return;
        setDocker(
          status.available
            ? { state: "ready", version: status.version }
            : {
                state: "unavailable",
                error: status.error ?? "Docker is not available.",
              },
        );
      })
      .catch((error) => {
        if (!cancelled) {
          setDocker({ state: "unavailable", error: String(error) });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleEngineChange = (value: string | null) => {
    if (value !== "PostgreSQL" && value !== "MySQL") return;
    setEngine(value);
    setVersion(ENGINE_VERSIONS[value][0]);
  };

  const port = portText.trim() === "" ? undefined : Number(portText.trim());
  const portValid =
    port === undefined ||
    (Number.isInteger(port) && port >= 1024 && port <= 65535);
  const canSubmit =
    docker.state === "ready" &&
    name.trim().length > 0 &&
    portValid &&
    !isProvisioning;

  const handleProvision = async () => {
    setIsProvisioning(true);
    setProvisionError(null);
    const outcome = await provisionManagedServer({
      name: name.trim(),
      engine,
      version,
      port,
    });
    setIsProvisioning(false);
    if (outcome.ok) {
      setResult(outcome.result);
      toast.success(`${outcome.result.server.name} is running`);
    } else {
      setProvisionError(outcome.error);
    }
  };

  if (result) {
    return <ProvisionSuccess result={result} onClose={onClose} />;
  }

  if (docker.state === "unavailable") {
    return (
      <div
        data-testid="docker-unavailable"
        className="flex flex-col gap-2 px-4 py-5 text-xs text-text-muted"
      >
        <p className="font-medium text-foreground">Docker is required</p>
        <p>
          dbunk provisions local databases as Docker containers. Install and
          start Docker Desktop (or another Docker engine), then try again.
        </p>
        <p className="break-all rounded-sm border border-border-subtle bg-surface-panel px-2 py-1 font-mono text-[0.625rem]">
          {docker.error}
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 px-4 py-3">
      <label
        htmlFor="local-db-name"
        className="flex flex-col gap-1 text-xs text-text-muted"
      >
        Name
        <Input
          id="local-db-name"
          data-testid="local-db-name"
          placeholder="my-project"
          value={name}
          onChange={(e) => setName(e.target.value)}
          disabled={isProvisioning}
        />
      </label>
      <div className="grid grid-cols-2 gap-3">
        <div className="flex flex-col gap-1 text-xs text-text-muted">
          Engine
          <Select
            value={engine}
            onValueChange={handleEngineChange}
            disabled={isProvisioning}
          >
            <SelectTrigger
              data-testid="local-db-engine"
              className="h-8 text-xs"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="PostgreSQL">PostgreSQL</SelectItem>
              <SelectItem value="MySQL">MySQL</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="flex flex-col gap-1 text-xs text-text-muted">
          Version
          <Select
            value={version}
            onValueChange={(v) => v && setVersion(v)}
            disabled={isProvisioning}
          >
            <SelectTrigger
              data-testid="local-db-version"
              className="h-8 text-xs"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {ENGINE_VERSIONS[engine].map((v) => (
                <SelectItem key={v} value={v}>
                  {v}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      <label
        htmlFor="local-db-port"
        className="flex flex-col gap-1 text-xs text-text-muted"
      >
        Port (optional — dbunk picks a free one)
        <Input
          id="local-db-port"
          data-testid="local-db-port"
          placeholder={engine === "PostgreSQL" ? "5433+" : "3307+"}
          value={portText}
          onChange={(e) => setPortText(e.target.value)}
          disabled={isProvisioning}
        />
      </label>

      {provisionError ? (
        <p
          data-testid="provision-error"
          className="break-words rounded-sm border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive"
        >
          {provisionError}
        </p>
      ) : null}

      {isProvisioning ? (
        <p className="text-xs text-text-muted">
          Creating the database… The first run for {engine} {version} may take a
          few minutes while Docker downloads the image.
        </p>
      ) : null}

      <div className="flex items-center justify-end gap-2 pb-1">
        <Button
          size="sm"
          variant="ghost"
          onClick={onClose}
          disabled={isProvisioning}
        >
          Cancel
        </Button>
        <Button
          size="sm"
          data-testid="local-db-create"
          disabled={!canSubmit}
          onClick={() => {
            void handleProvision();
          }}
        >
          {isProvisioning ? "Creating…" : "Create database"}
        </Button>
      </div>
      {docker.state === "ready" && docker.version ? (
        <p className="text-[0.625rem] text-text-muted">
          Docker {docker.version} detected
        </p>
      ) : null}
    </div>
  );
}

function ProvisionSuccess({
  result,
  onClose,
}: {
  result: ProvisionManagedServerResult;
  onClose: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const { server } = result;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(result.connectionString);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      toast.error("Could not access the clipboard");
    }
  };

  return (
    <div
      data-testid="provision-success"
      className="flex flex-col gap-3 px-4 py-3 text-xs"
    >
      <p className="text-foreground">
        <span className="font-semibold">{server.name}</span> is running —{" "}
        {server.engine} {server.version} on port {server.port}. A connection was
        added to your sidebar.
      </p>
      <div className="flex flex-col gap-1 text-text-muted">
        Connection string for your project
        <div className="flex items-center gap-1.5">
          <code className="min-w-0 flex-1 break-all rounded-sm border border-border-subtle bg-surface-panel px-2 py-1.5 font-mono text-[0.625rem]">
            {result.connectionString}
          </code>
          <Button
            size="icon-sm"
            variant="outline"
            aria-label="Copy connection string"
            onClick={() => {
              void copy();
            }}
          >
            {copied ? (
              <IconCheck className="size-3.5" />
            ) : (
              <IconCopy className="size-3.5" />
            )}
          </Button>
        </div>
      </div>
      <p className="text-text-muted">
        The password is stored with your other connection credentials. The
        server keeps running when dbunk quits; manage it under Settings → Local
        Databases.
      </p>
      <div className="flex justify-end pb-1">
        <Button size="sm" onClick={onClose}>
          Done
        </Button>
      </div>
    </div>
  );
}
