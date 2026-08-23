import { IconHelp } from "@tabler/icons-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

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
        <IconHelp />
        Glossary
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent size="lg" data-testid="schema-map-glossary">
          <DialogHeader>
            <DialogTitle>Schema Map glossary</DialogTitle>
            <DialogDescription className="text-xs text-text-muted">
              The terms the Schema Map uses for its graph elements.
            </DialogDescription>
          </DialogHeader>
          <DialogBody>
            <dl className="flex flex-col gap-2.5">
              {GLOSSARY_TERMS.map((entry) => (
                <div key={entry.term}>
                  <dt className="text-xs font-semibold">{entry.term}</dt>
                  <dd className="text-2xs text-text-muted">
                    {entry.definition}
                  </dd>
                </div>
              ))}
            </dl>
          </DialogBody>
        </DialogContent>
      </Dialog>
    </>
  );
}
