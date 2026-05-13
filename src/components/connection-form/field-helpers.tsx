/**
 * Small reusable bits shared across per-engine field components.
 * `<FieldError>` renders the single-line danger text; `<ToggleSwitchRow>`
 * consolidates the label-with-switch row (SSL / Use HTTPS / Use TLS /
 * Verify TLS certificate) that fallow's dupes pass flagged as triplicate.
 */

import { Switch } from "@/components/ui/switch";

export function FieldError({ text }: { text: string | null }) {
  if (!text) return null;
  return <p className="text-[0.6875rem] text-danger">{text}</p>;
}

interface ToggleSwitchRowProps {
  id: string;
  title: string;
  description: string;
  checked: boolean;
  defaultChecked?: boolean;
  onCheckedChange: (next: boolean) => void;
}

export function ToggleSwitchRow({
  id,
  title,
  description,
  checked,
  onCheckedChange,
}: ToggleSwitchRowProps) {
  return (
    <label
      htmlFor={id}
      className="flex items-center justify-between rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2.5"
    >
      <span className="flex flex-col gap-0.5">
        <span className="text-xs font-medium">{title}</span>
        <span className="text-[0.6875rem] text-text-muted">{description}</span>
      </span>
      <Switch id={id} checked={checked} onCheckedChange={onCheckedChange} />
    </label>
  );
}
