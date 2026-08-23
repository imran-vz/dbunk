import { isMacPlatform } from "@/components/ui/kbd";

/** Native traffic lights sit at y=26 with ~12px controls (see tauri.conf.json). */
export const MACOS_TRAFFIC_LIGHT_GUTTER_PX = 40;

/**
 * The lights start at x=18 and span ~52px, so they overhang the 40px
 * activity rail into the header by ~30px. The header's left padding
 * clears that overhang. This is the only place the number lives.
 */
export const MACOS_TRAFFIC_LIGHT_HEADER_INSET_PX = 36;

export function needsMacTitlebarGutter(isWindowFullscreen: boolean): boolean {
  return isMacPlatform() && !isWindowFullscreen;
}
