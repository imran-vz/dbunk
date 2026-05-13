import type {
  MouseEvent as ReactMouseEvent,
  ReactNode,
  PointerEvent as ReactPointerEvent,
} from "react";

const TOP_BAR_INTERACTIVE_SELECTOR = [
  "a[href]",
  "button",
  "input",
  "select",
  "textarea",
  "[contenteditable='true']",
  "[role='button']",
  "[role='checkbox']",
  "[role='combobox']",
  "[role='menuitem']",
  "[role='textbox']",
].join(",");

export interface WindowDragHandlers {
  onDoubleClick: (event: ReactMouseEvent<HTMLElement>) => void;
  onPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
}

/**
 * Decide if a pointer event on the top bar should initiate a native window
 * drag. We skip when the event lands on an interactive control or has been
 * defaultPrevented, and require the target to be inside an explicitly
 * marked drag region.
 */
export function shouldStartTopBarDrag(
  event: ReactMouseEvent<HTMLElement> | ReactPointerEvent<HTMLElement>,
): boolean {
  if (event.button !== 0 || event.defaultPrevented) {
    return false;
  }
  const target = event.target;
  if (!(target instanceof Element)) {
    return false;
  }
  if (target.closest(TOP_BAR_INTERACTIVE_SELECTOR)) {
    return false;
  }
  return Boolean(target.closest("[data-window-drag-region]"));
}

export function WindowDragFrame({
  children,
  onDoubleClick,
  onPointerDown,
}: WindowDragHandlers & { children: ReactNode }) {
  return (
    <div className="fixed inset-0 overflow-hidden bg-surface-app">
      <WindowDragSurface
        onDoubleClick={onDoubleClick}
        onPointerDown={onPointerDown}
      />
      {children}
    </div>
  );
}

export function WindowDragSurface({
  onDoubleClick,
  onPointerDown,
}: WindowDragHandlers) {
  return (
    <div
      data-window-drag-region
      data-testid="window-drag-surface"
      aria-hidden="true"
      className="absolute inset-x-0 top-0 z-50 h-10 select-none"
      onDoubleClick={onDoubleClick}
      onPointerDown={onPointerDown}
    />
  );
}
