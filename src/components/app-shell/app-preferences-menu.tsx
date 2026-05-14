/**
 * Avatar dropdown — preferences, theme toggle, About. The theme is
 * persisted in localStorage and applied by toggling `data-theme` on
 * `document.documentElement`. The choice list reserves a slot for
 * future cloud sign-in / sign-out actions.
 */

import { IconMoon, IconSettings, IconSun } from "@tabler/icons-react";
import { useEffect, useState } from "react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useAppStore } from "@/lib/store";

const THEME_STORAGE_KEY = "dbunk.theme";

type ThemeChoice = "system" | "light" | "dark";

function readStoredTheme(): ThemeChoice {
  if (typeof window === "undefined") return "system";
  const raw = window.localStorage.getItem(THEME_STORAGE_KEY);
  if (raw === "light" || raw === "dark" || raw === "system") return raw;
  return "system";
}

function applyTheme(choice: ThemeChoice) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (choice === "system") {
    root.removeAttribute("data-theme");
    return;
  }
  root.setAttribute("data-theme", choice);
}

export function AppPreferencesMenu({ initials }: { initials: string }) {
  const [theme, setTheme] = useState<ThemeChoice>(() => readStoredTheme());
  const setActiveView = useAppStore((state) => state.setActiveView);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  const handleThemeChange = (next: string) => {
    if (next !== "system" && next !== "light" && next !== "dark") return;
    setTheme(next);
    if (typeof window !== "undefined") {
      window.localStorage.setItem(THEME_STORAGE_KEY, next);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        type="button"
        aria-label="Account menu"
        className="inline-flex size-7 items-center justify-center rounded-full bg-secondary text-[0.625rem] font-semibold text-secondary-foreground hover:bg-secondary/80"
      >
        {initials}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56 text-xs">
        <DropdownMenuLabel className="text-[0.65rem] uppercase text-text-muted">
          Account
        </DropdownMenuLabel>
        <DropdownMenuItem
          onSelect={() => setActiveView("settings")}
          className="gap-2"
        >
          <IconSettings className="size-3.5" />
          Open Settings
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="text-[0.65rem] uppercase text-text-muted">
          Theme
        </DropdownMenuLabel>
        <DropdownMenuRadioGroup value={theme} onValueChange={handleThemeChange}>
          <DropdownMenuRadioItem value="system" className="gap-2 text-xs">
            <IconSun className="size-3.5 opacity-60" />
            System
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="light" className="gap-2 text-xs">
            <IconSun className="size-3.5" />
            Light
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="dark" className="gap-2 text-xs">
            <IconMoon className="size-3.5" />
            Dark
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          // Cloud sign-in is reserved for a future PR; surface the slot
          // so users see where the option will live.
          disabled
          aria-disabled
          onSelect={(event) => event.preventDefault()}
          className="text-text-muted"
        >
          Cloud sign-in (coming soon)
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={(event) => {
            event.preventDefault();
            window.open("https://github.com/imran-vz/dbunk", "_blank");
          }}
          className="gap-2 text-xs"
        >
          About dbunk
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
