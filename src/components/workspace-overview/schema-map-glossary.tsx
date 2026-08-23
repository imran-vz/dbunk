import { IconHelp } from "@tabler/icons-react";
import { useState } from "react";

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";

/**
 * Canonical Schema Map terms from CONTEXT.md, phrased as UI-facing help
 * text without coining different names for the same concepts.
 */
const GLOSSARY_TERMS: Array<{ term: string; definition: string }> = [
  {
    term: "Schema Map",
    definition:
      "The visual graph of relational tables and their foreign-key relationships.",
  },
  {
    term: "Table-Level Schema Map",
    definition:
      "A Schema Map scoped to one table workspace subtab, showing the current Table Card, directly referenced Table Cards, directly referencing Table Cards, and the Relationship Edges connecting those direct neighbors.",
  },
  {
    term: "Table Card",
    definition: "One table's visual node in the Schema Map.",
  },
  {
    term: "Junction Table Card",
    definition:
      "A Table Card identified as representing a many-to-many association through its real foreign-key relationships; the Schema Map still shows the real Relationship Edges rather than replacing them with a synthetic direct edge.",
  },
  {
    term: "Column Row",
    definition: "One column entry inside a Table Card.",
  },
  {
    term: "Trigger Indicator",
    definition:
      "A Schema Map signal on a Table Card or Column Row that database trigger metadata exists for that table or explicitly targets that column.",
  },
  {
    term: "Relationship Edge",
    definition:
      "One visual line for one foreign-key constraint between Table Cards.",
  },
  {
    term: "Relationship Cardinality",
    definition:
      "The backend-provided classification for a Relationship Edge, derived from database constraints where possible and accompanied by a reason when the classification needs explanation.",
  },
  {
    term: "Relationship Detail Popover",
    definition: "The popup shown when a Relationship Edge is selected.",
  },
  {
    term: "Focused Table",
    definition:
      "The selected Table Card whose directly referencing and directly referenced Table Cards, plus connected Relationship Edges, remain emphasized while unrelated graph elements dim.",
  },
  {
    term: "Focused Relationship Edge",
    definition:
      "The selected Relationship Edge whose two endpoint Table Cards remain emphasized while unrelated graph elements dim.",
  },
];

/**
 * Toolbar entry point for the Schema Map glossary. The glossary opens
 * in a dialog so no help copy sits over the map canvas.
 */
export function SchemaMapGlossaryButton() {
  const [open, setOpen] = useState(false);

  return (
    <>
      <Button
        type="button"
        size="sm"
        variant="outline"
        onClick={() => setOpen(true)}
      >
        <IconHelp className="size-3" />
        Glossary
      </Button>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent
          data-testid="schema-map-glossary"
          className="flex max-h-[80vh] w-[30rem] max-w-[30rem] flex-col gap-0 overflow-hidden rounded-lg border border-border-subtle bg-surface-window p-0 sm:max-w-[30rem]"
        >
          <AlertDialogHeader className="border-b border-border-subtle px-4 py-3">
            <AlertDialogTitle className="text-sm font-semibold">
              Schema Map glossary
            </AlertDialogTitle>
            <AlertDialogDescription className="text-2xs text-text-muted">
              The terms the Schema Map uses for its graph elements.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <dl className="flex flex-col gap-2.5 overflow-y-auto px-4 py-3">
            {GLOSSARY_TERMS.map((entry) => (
              <div key={entry.term}>
                <dt className="text-xs font-semibold">{entry.term}</dt>
                <dd className="text-2xs text-text-muted">{entry.definition}</dd>
              </div>
            ))}
          </dl>
          <AlertDialogFooter className="border-t border-border-subtle px-4 py-2.5">
            <AlertDialogCancel>Close</AlertDialogCancel>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
