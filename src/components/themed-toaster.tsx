import { useSyncExternalStore } from "react";
import { Toaster } from "sonner";

/**
 * Sonner toaster that follows the *app* theme, not the OS
 * (DESIGN-SYSTEM §4.9). The resolved theme lives as the `.dark` class
 * on <html> (set pre-paint by the boot script and by `applyTheme`),
 * so we observe that class instead of duplicating mode/preset
 * resolution here.
 */

function subscribeToRootClass(onChange: () => void): () => void {
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["class"],
  });
  return () => observer.disconnect();
}

function readIsDark(): boolean {
  return document.documentElement.classList.contains("dark");
}

export function ThemedToaster() {
  const isDark = useSyncExternalStore(
    subscribeToRootClass,
    readIsDark,
    // SSR shell renders before the boot script's target is knowable;
    // dark is the design-primary default and the client snapshot
    // corrects it on hydration.
    () => true,
  );

  return (
    <Toaster
      theme={isDark ? "dark" : "light"}
      position="bottom-right"
      richColors
      closeButton
      toastOptions={{
        classNames: {
          toast:
            "font-sans text-xs border-border-subtle bg-surface-window text-foreground",
        },
      }}
    />
  );
}
