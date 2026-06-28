import { isMacPlatform } from "@/components/ui/kbd";

/** Native traffic lights sit at y=26 with ~12px controls (see tauri.conf.json). */
export const MACOS_TRAFFIC_LIGHT_GUTTER_PX = 40;

export function needsMacTitlebarGutter(isWindowFullscreen: boolean): boolean {
  return isMacPlatform() && !isWindowFullscreen;
}
