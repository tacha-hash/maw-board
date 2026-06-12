import { persisted } from "svelte-persisted-store";
import themes, { type ThemeName, defaultTheme } from "./ui/themes";
import { derived, type Readable } from "svelte/store";

export type Settings = {
  name: string;
  theme: ThemeName;
  scrollback: number;
  background: string; // board background color (CSS color string)
};

export const DEFAULT_BACKGROUND = "#0e0e10";

const storedSettings = persisted<Partial<Settings>>("sshx-settings-store", {});

/** A persisted store for settings of the current user. */
export const settings: Readable<Settings> = derived(
  storedSettings,
  ($storedSettings) => {
    // Do some validation on all of the stored settings.
    const name = $storedSettings.name ?? "";

    let theme = $storedSettings.theme;
    if (!theme || !Object.hasOwn(themes, theme)) {
      theme = defaultTheme;
    }

    let scrollback = $storedSettings.scrollback;
    if (typeof scrollback !== "number" || scrollback < 0) {
      scrollback = 5000;
    }

    const background =
      typeof $storedSettings.background === "string" &&
      $storedSettings.background
        ? $storedSettings.background
        : DEFAULT_BACKGROUND;

    return {
      name,
      theme,
      scrollback,
      background,
    };
  },
);

export function updateSettings(values: Partial<Settings>) {
  storedSettings.update((settings) => ({ ...settings, ...values }));
}
