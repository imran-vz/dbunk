import { useEffect, useMemo, useRef, useState } from "react";

import {
  SchemaRelationshipMap,
  type SchemaRelationshipMapHandle,
} from "@/components/schema-relationship-map";
import {
  type SchemaMapExportFormat,
  SchemaMapToolbar,
  schemaMapExportFilename,
} from "@/components/workspace-overview/schema-map-toolbar";
import { DEFAULT_SCHEMA_MAP_PREFS } from "@/lib/schema-graph";
import type { Connection, SchemaExplorer } from "@/lib/store";
import { useAppStore } from "@/lib/store";

export function SchemaMapTab({
  activeConnection,
  schemas,
  isClient,
}: {
  activeConnection: Connection;
  schemas: SchemaExplorer[];
  isClient: boolean;
}) {
  const mapRef = useRef<SchemaRelationshipMapHandle>(null);
  const [exportError, setExportError] = useState<string | null>(null);
  const {
    connectionSchemaMapSchema,
    setConnectionSchemaMapSchema,
    schemaMapPrefs,
    setSchemaMapPref,
    resetSchemaMapPositions,
  } = useAppStore();

  const schemaNames = useMemo(
    () => [...new Set(schemas.map((schema) => schema.name))].sort(),
    [schemas],
  );
  const fallbackSchema = useMemo(() => {
    if (schemaNames.includes("public")) {
      return "public";
    }
    return schemaNames[0] ?? activeConnection.database;
  }, [activeConnection.database, schemaNames]);
  const storedSchema = connectionSchemaMapSchema[activeConnection.id];
  const selectedSchema =
    storedSchema &&
    (schemaNames.length === 0 || schemaNames.includes(storedSchema))
      ? storedSchema
      : fallbackSchema;
  const pickerSchemas = schemaNames.includes(selectedSchema)
    ? schemaNames
    : [selectedSchema, ...schemaNames].filter(Boolean);
  const prefs =
    schemaMapPrefs[activeConnection.id]?.[selectedSchema] ??
    DEFAULT_SCHEMA_MAP_PREFS;

  useEffect(() => {
    if (selectedSchema && storedSchema !== selectedSchema) {
      setConnectionSchemaMapSchema(activeConnection.id, selectedSchema);
    }
  }, [
    activeConnection.id,
    selectedSchema,
    setConnectionSchemaMapSchema,
    storedSchema,
  ]);

  const handleExport = async (format: SchemaMapExportFormat) => {
    setExportError(null);
    try {
      await mapRef.current?.exportImage(
        format,
        schemaMapExportFilename(activeConnection.name, selectedSchema, format),
      );
    } catch (error) {
      setExportError(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <section className="flex h-[min(68vh,42rem)] min-h-[27.5rem] flex-col overflow-hidden rounded-md border border-border-subtle bg-surface-window">
      <SchemaMapToolbar
        schemas={pickerSchemas}
        selectedSchema={selectedSchema}
        prefs={prefs}
        exportError={exportError}
        onSchemaChange={(schema) =>
          setConnectionSchemaMapSchema(activeConnection.id, schema)
        }
        onPrefsChange={(patch) => {
          void setSchemaMapPref(activeConnection.id, selectedSchema, patch);
        }}
        onResetLayout={() => {
          void resetSchemaMapPositions(activeConnection.id, selectedSchema);
        }}
        onExport={(format) => {
          void handleExport(format);
        }}
      />
      <div className="min-h-0 flex-1 bg-surface-app">
        <SchemaRelationshipMap
          ref={mapRef}
          connectionId={activeConnection.id}
          schema={selectedSchema}
          activeTable={null}
          isClient={isClient}
        />
      </div>
    </section>
  );
}
