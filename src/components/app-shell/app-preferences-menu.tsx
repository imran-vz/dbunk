/**
 * Avatar dropdown — preferences, theme toggle, About. Theme is
 * persisted via the credentials slice (`setTheme` → AppSettings /
 * SQLite + `localStorage["dbunk.theme"]` boot cache). The choice
 * list reserves a slot for future cloud sign-in / sign-out actions.
 */

import { IconMoon, IconSettings, IconSun } from "@tabler/icons-react";

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
import { isThemeMode, type ThemeMode } from "@/lib/theme";

export function AppPreferencesMenu({ initials }: { initials: string }) {
  const setActiveView = useAppStore((state) => state.setActiveView);
  const setTheme = useAppStore((state) => state.setTheme);
  const theme: ThemeMode = useAppStore(
    (state) => state.appSettings?.theme ?? "system",
  );

  const handleThemeChange = (next: string) => {
    if (!isThemeMode(next)) return;
    void setTheme(next);
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
