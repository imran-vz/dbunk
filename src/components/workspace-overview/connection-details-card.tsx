import { IconEdit } from "@tabler/icons-react";
import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type { Connection } from "@/lib/store";

import { KeyValue } from "./key-value";

export function ConnectionDetailsCard({
  connection,
}: {
  connection: Connection;
}) {
  const rows: Array<[string, ReactNode]> = [
    ["Host", connection.host || "prod-db.dbunk.io"],
    ["Database", connection.database || "app_prod"],
    ["User", connection.user || "dbunk_app"],
    ["Engine", connection.engine],
    ["Region", "us-east-1"],
    ["SSL", "Enabled"],
  ];

  return (
    <Card>
      <CardHeader>
        <CardTitle>Connection Details</CardTitle>
        <CardAction>
          <Button size="sm" variant="outline">
            <IconEdit className="size-3.5" />
            Edit Connection
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="grid grid-cols-2 gap-x-6 gap-y-3 text-xs">
        {rows.map(([label, value]) => (
          <KeyValue key={label} label={label} value={value} />
        ))}
      </CardContent>
    </Card>
  );
}
