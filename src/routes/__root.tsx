import { TanStackDevtools } from "@tanstack/react-devtools";
import { createRootRoute, HeadContent, Scripts } from "@tanstack/react-router";
import { TanStackRouterDevtoolsPanel } from "@tanstack/react-router-devtools";
import { Toaster } from "sonner";

import { TooltipProvider } from "@/components/ui/tooltip";

import appCss from "../styles.css?url";

/**
 * Theme boot script — runs synchronously before React mounts so the
 * first paint has the right `.dark` class + `data-theme` on <html>
 * and there's no flash of the wrong theme. localStorage is the boot
 * cache; AppSettings / SQLite is canonical and rehydrates on
 * `loadAppSettings`. See docs/design/theme-support-plan.md.
 *
 * Intrinsically-dark presets (currently "dracula") force `.dark`
 * regardless of the stored mode, matching `applyTheme` runtime
 * behaviour.
 */
const THEME_BOOT_SCRIPT = `(function(){try{var t=localStorage.getItem("dbunk.theme");var p=localStorage.getItem("dbunk.theme.preset");var presetDark=p==="dracula";var dark=presetDark||t==="dark"||((t===null||t==="system")&&window.matchMedia("(prefers-color-scheme: dark)").matches);var r=document.documentElement;if(dark)r.classList.add("dark");if(p&&p!=="default")r.setAttribute("data-theme",p);}catch(e){}})();`;

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1" },
      { title: "Dbunk" },
    ],
    links: [{ rel: "stylesheet", href: appCss }],
  }),
  shellComponent: RootDocument,
});

function RootDocument({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <script
          id="dbunk-theme-boot"
          // oxlint-disable-next-line react/no-danger -- pre-paint theme script with a build-time-constant body; must execute before React hydrates to avoid a flash of the wrong theme.
          dangerouslySetInnerHTML={{ __html: THEME_BOOT_SCRIPT }}
        />
        <HeadContent />
      </head>
      <body>
        <TooltipProvider>{children}</TooltipProvider>
        {/* Toaster reads `theme="system"` so it follows OS prefers-
            color-scheme. If the user manually picks an explicit Light /
            Dark mode that differs from OS, toasts can briefly mismatch
            the app surface; the custom class names below pull from CSS
            variables, so this drift is limited to sonner's built-in
            chrome (icons / outline). */}
        <Toaster
          theme="system"
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
        <TanStackDevtools
          config={{ position: "bottom-right" }}
          plugins={[
            {
              name: "Tanstack Router",
              render: <TanStackRouterDevtoolsPanel />,
            },
          ]}
        />
        <Scripts />
      </body>
    </html>
  );
}
