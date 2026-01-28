import {
  IconChevronDown,
  IconDatabase,
  IconPlayerPlay,
  IconPlus,
  IconSearch,
  IconSettings,
  IconTable,
  IconTerminal2,
  IconX,
} from "@tabler/icons-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";

const connections = [
  {
    id: "primary",
    name: "Primary Postgres",
    database: "core",
    status: "Connected",
  },
  {
    id: "warehouse",
    name: "Warehouse",
    database: "analytics",
    status: "Read only",
  },
  {
    id: "staging",
    name: "Staging",
    database: "staging",
    status: "Disconnected",
  },
];

const tabs = [
  {
    id: "tab-users",
    type: "Table",
    label: "users",
    icon: IconTable,
  },
  {
    id: "tab-activity",
    type: "Table",
    label: "activity_log",
    icon: IconTable,
  },
  {
    id: "tab-query-1",
    type: "Query",
    label: "monthly_revenue.sql",
    icon: IconTerminal2,
  },
];

const activeTabId = "tab-query-1";

const recentQueries = [
  "select count(*) from users where status = 'active'",
  "select sum(total) from orders where created_at > now() - interval '7 days'",
  "select * from invoices where status = 'overdue' limit 50",
];

export function AppShell() {
  return (
    <div className="flex h-screen w-screen bg-background text-foreground">
      <aside className="flex h-full w-[260px] flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground">
        <div className="flex items-center justify-between px-4 py-3">
          <div className="flex items-center gap-2">
            <div className="flex size-8 items-center justify-center rounded-md bg-sidebar-primary text-sidebar-primary-foreground">
              <IconDatabase className="size-4" />
            </div>
            <div>
              <div className="text-sm font-semibold">Dbunk</div>
              <div className="text-xs text-muted-foreground">
                Gateway Console
              </div>
            </div>
          </div>
          <Button size="icon-sm" variant="ghost" aria-label="Open settings">
            <IconSettings />
          </Button>
        </div>
        <div className="px-4 pb-3">
          <Input placeholder="Search tables, schemas, queries" />
        </div>
        <Separator />
        <div className="flex items-center justify-between px-4 py-3">
          <div className="text-xs font-semibold uppercase text-muted-foreground">
            Connections
          </div>
          <Button size="icon-xs" variant="outline" aria-label="Add connection">
            <IconPlus />
          </Button>
        </div>
        <div className="flex flex-1 flex-col gap-2 overflow-auto px-3 pb-4">
          {connections.map((connection) => {
            const isActive = connection.id === "primary";
            return (
              <div
                key={connection.id}
                className={cn(
                  "flex flex-col gap-1 rounded-md border px-3 py-2 text-xs transition",
                  isActive
                    ? "border-sidebar-accent bg-sidebar-accent/10"
                    : "border-transparent hover:border-sidebar-border hover:bg-sidebar-accent/5",
                )}
              >
                <div className="flex items-center justify-between">
                  <div className="text-sm font-medium text-sidebar-foreground">
                    {connection.name}
                  </div>
                  <Badge
                    variant={
                      connection.status === "Disconnected"
                        ? "secondary"
                        : "default"
                    }
                    className="text-[0.625rem]"
                  >
                    {connection.status}
                  </Badge>
                </div>
                <div className="text-muted-foreground">
                  {connection.database}
                </div>
              </div>
            );
          })}
        </div>
        <Separator />
        <div className="px-4 py-3 text-xs text-muted-foreground">
          Connected as admin@primary
        </div>
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center justify-between border-b px-4 py-3">
          <div>
            <div className="text-sm font-semibold">Workspace</div>
            <div className="text-xs text-muted-foreground">
              Primary Postgres / core.public
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Button size="sm" variant="secondary">
              <IconTerminal2 />
              New query
            </Button>
            <Button size="sm" variant="outline">
              <IconTable />
              New table
            </Button>
          </div>
        </header>
        <div className="flex items-center gap-2 border-b px-4 py-2">
          <div className="flex min-w-0 flex-1 items-center gap-2 overflow-x-auto">
            {tabs.map((tab) => {
              const isActive = tab.id === activeTabId;
              const TabIcon = tab.icon;
              return (
                <div
                  key={tab.id}
                  className={cn(
                    "flex items-center gap-2 rounded-md border px-2 py-1 text-xs",
                    isActive
                      ? "border-border bg-muted"
                      : "border-transparent bg-transparent hover:border-border hover:bg-muted/40",
                  )}
                >
                  <TabIcon className="size-3.5" />
                  <span className="max-w-[160px] truncate">{tab.label}</span>
                  <Badge variant="secondary" className="text-[0.625rem]">
                    {tab.type}
                  </Badge>
                  <Button
                    size="icon-xs"
                    variant="ghost"
                    aria-label={`Close ${tab.label}`}
                  >
                    <IconX />
                  </Button>
                </div>
              );
            })}
          </div>
          <Button size="icon-sm" variant="ghost" aria-label="Open tab menu">
            <IconChevronDown />
          </Button>
        </div>
        <div className="flex min-h-0 flex-1">
          <div className="grid h-full min-h-0 flex-1 grid-cols-[1fr_320px]">
            <section className="flex min-h-0 flex-col border-r">
              <div className="flex items-center justify-between border-b px-4 py-3">
                <div className="flex items-center gap-2">
                  <div className="flex size-8 items-center justify-center rounded-md bg-muted">
                    <IconTable className="size-4" />
                  </div>
                  <div>
                    <div className="text-sm font-semibold">
                      monthly_revenue.sql
                    </div>
                    <div className="text-xs text-muted-foreground">
                      Query • last run 2 min ago
                    </div>
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <Button size="sm" variant="secondary">
                    <IconPlayerPlay />
                    Run
                  </Button>
                  <Button size="sm" variant="outline">
                    <IconSearch />
                    Explain
                  </Button>
                </div>
              </div>
              <div className="grid min-h-0 flex-1 grid-rows-[auto_1fr] gap-3 p-4">
                <Card className="border border-border">
                  <CardHeader className="flex flex-row items-center justify-between">
                    <CardTitle>SQL editor</CardTitle>
                    <Badge variant="secondary" className="text-[0.625rem]">
                      142 ms
                    </Badge>
                  </CardHeader>
                  <CardContent className="text-xs text-muted-foreground">
                    select date_trunc('month', created_at) as month, sum(total)
                    as revenue from invoices group by 1 order by 1 desc limit 12
                  </CardContent>
                </Card>
                <Card className="flex min-h-0 flex-col border border-border">
                  <CardHeader>
                    <CardTitle>Results preview</CardTitle>
                  </CardHeader>
                  <CardContent className="min-h-0 flex-1">
                    <div className="grid min-h-0 grid-rows-[auto_1fr] overflow-hidden rounded-md border">
                      <div className="grid grid-cols-[160px_1fr_140px] border-b bg-muted px-3 py-2 text-[0.625rem] uppercase tracking-wide text-muted-foreground">
                        <div>month</div>
                        <div>revenue</div>
                        <div>invoices</div>
                      </div>
                      <div className="min-h-0 overflow-auto">
                        {[
                          ["2025-12", "$182,430", "812"],
                          ["2025-11", "$171,902", "754"],
                          ["2025-10", "$162,110", "701"],
                          ["2025-09", "$159,402", "690"],
                          ["2025-08", "$148,775", "641"],
                        ].map((row) => (
                          <div
                            key={row[0]}
                            className="grid grid-cols-[160px_1fr_140px] border-b px-3 py-2 text-xs last:border-b-0"
                          >
                            <div className="text-muted-foreground">
                              {row[0]}
                            </div>
                            <div className="font-medium">{row[1]}</div>
                            <div className="text-muted-foreground">
                              {row[2]}
                            </div>
                          </div>
                        ))}
                      </div>
                    </div>
                  </CardContent>
                </Card>
              </div>
            </section>
            <aside className="flex min-h-0 flex-col gap-3 bg-muted/20 p-4">
              <Card size="sm" className="border border-border">
                <CardHeader>
                  <CardTitle>Query details</CardTitle>
                </CardHeader>
                <CardContent className="space-y-2 text-xs text-muted-foreground">
                  <div className="flex items-center justify-between">
                    <span>Rows</span>
                    <span className="text-foreground">12</span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span>Runtime</span>
                    <span className="text-foreground">142 ms</span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span>Cache</span>
                    <Badge variant="secondary" className="text-[0.625rem]">
                      Warm
                    </Badge>
                  </div>
                </CardContent>
              </Card>
              <Card size="sm" className="border border-border">
                <CardHeader>
                  <CardTitle>Recent queries</CardTitle>
                </CardHeader>
                <CardContent className="space-y-2 text-xs">
                  {recentQueries.map((query) => (
                    <div key={query} className="rounded-md border px-2 py-1">
                      <div className="truncate text-muted-foreground">
                        {query}
                      </div>
                    </div>
                  ))}
                </CardContent>
              </Card>
              <Card size="sm" className="border border-border">
                <CardHeader>
                  <CardTitle>Table insights</CardTitle>
                </CardHeader>
                <CardContent className="space-y-2 text-xs text-muted-foreground">
                  <div className="flex items-center justify-between">
                    <span>Primary key</span>
                    <span className="text-foreground">invoice_id</span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span>Indexes</span>
                    <span className="text-foreground">4</span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span>Last vacuum</span>
                    <span className="text-foreground">2 days ago</span>
                  </div>
                </CardContent>
              </Card>
            </aside>
          </div>
        </div>
      </div>
    </div>
  );
}
