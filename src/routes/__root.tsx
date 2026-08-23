import { TanStackDevtools } from "@tanstack/react-devtools";
import { createRootRoute, HeadContent, Scripts } from "@tanstack/react-router";
import { TanStackRouterDevtoolsPanel } from "@tanstack/react-router-devtools";

import { ThemedToaster } from "@/components/themed-toaster";
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
const THEME_BOOT_SCRIPT = `(function(){try{var t=localStorage.getItem("dbunk.theme");var p=localStorage.getItem("dbunk.theme.preset");var presetDark=p==="dracula";var dark=presetDark||t==="dark"||((t===null||t==="system")&&window.matchMedia("(prefers-color-scheme: dark)").matches);var r=document.documentElement;if(dark)r.classList.add("dark");if(p&&p!=="default")r.setAttribute("data-theme",p);var d=localStorage.getItem("dbunk.density");if(d==="compact"||d==="comfortable")r.setAttribute("data-density",d);}catch(e){}})();`;

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
        <ThemedToaster />
        {/* Dev-only, and kept out of the bottom-right toast corner. */}
        {import.meta.env.DEV ? (
          <TanStackDevtools
            config={{ position: "bottom-left" }}
            plugins={[
              {
                name: "Tanstack Router",
                render: <TanStackRouterDevtoolsPanel />,
              },
            ]}
          />
        ) : null}
        <Scripts />
      </body>
    </html>
  );
}
