import { ref, watch } from "vue";

/**
 * How this browser draws the interface, apart from light and dark
 * (theme.ts): which palette, which faces, how large the type is. These are
 * a viewer's choices — a phone and a desk differ, and a reading face is a
 * taste — so they stay in this browser rather than with the server's
 * preferences, like the language does.
 *
 * Each becomes a data attribute on <html> that styles.css reads. The default
 * faces are bundled at startup; another is fetched the first time it is
 * chosen, and the page falls back gracefully until it arrives.
 */
export type Palette = "dock" | "slate" | "ember";
export type InterfaceFont = "instrument-sans" | "inter" | "geist" | "dm-sans" | "system";
export type ChatFont = "interface" | "lora" | "serif";
export type TextScale = "small" | "default" | "large" | "larger" | "largest";

export const PALETTES: readonly Palette[] = ["dock", "slate", "ember"];
export const INTERFACE_FONTS: readonly InterfaceFont[] = ["instrument-sans", "inter", "geist", "dm-sans", "system"];
export const CHAT_FONTS: readonly ChatFont[] = ["interface", "lora", "serif"];
export const TEXT_SCALES: readonly TextScale[] = ["small", "default", "large", "larger", "largest"];
const SCALE_VALUES: Record<TextScale, number> = { small: 0.92, default: 1, large: 1.08, larger: 1.16, largest: 1.28 };

const KEYS = { palette: "tendly.palette", font: "tendly.font", chatFont: "tendly.chat-font", textScale: "tendly.text-scale" } as const;

function read<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const value = localStorage.getItem(key);
    return allowed.includes(value as T) ? (value as T) : fallback;
  } catch { return fallback; }
}
function write(key: string, value: string, fallback: string) {
  try {
    if (value === fallback) localStorage.removeItem(key); else localStorage.setItem(key, value);
  } catch { /* Private browsing: the choice lasts for this page. */ }
}

export const palette = ref<Palette>(read(KEYS.palette, PALETTES, "dock"));
export const interfaceFont = ref<InterfaceFont>(read(KEYS.font, INTERFACE_FONTS, "instrument-sans"));
export const chatFont = ref<ChatFont>(read(KEYS.chatFont, CHAT_FONTS, "interface"));
export const textScale = ref<TextScale>(read(KEYS.textScale, TEXT_SCALES, "default"));

/** Faces fetched on demand; the default pair is imported by main.ts. */
const loaders: Partial<Record<InterfaceFont | ChatFont, () => Promise<unknown>>> = {
  inter: () => import("@fontsource-variable/inter"),
  geist: () => import("@fontsource-variable/geist"),
  "dm-sans": () => import("@fontsource-variable/dm-sans"),
  lora: () => import("@fontsource-variable/lora"),
};
const loaded = new Set<string>();
function ensureFace(name: string) {
  const load = loaders[name as InterfaceFont | ChatFont];
  if (!load || loaded.has(name)) return;
  loaded.add(name);
  load().catch(() => loaded.delete(name));
}

function apply() {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const set = (name: string, value: string, fallback: string) => {
    if (value === fallback) delete root.dataset[name]; else root.dataset[name] = value;
  };
  set("palette", palette.value, "dock");
  set("font", interfaceFont.value, "instrument-sans");
  set("chatFont", chatFont.value, "interface");
  root.style.setProperty("--text-scale", String(SCALE_VALUES[textScale.value]));
  ensureFace(interfaceFont.value);
  ensureFace(chatFont.value);
}
apply();

watch(palette, value => { write(KEYS.palette, value, "dock"); apply(); });
watch(interfaceFont, value => { write(KEYS.font, value, "instrument-sans"); apply(); });
watch(chatFont, value => { write(KEYS.chatFont, value, "interface"); apply(); });
watch(textScale, value => { write(KEYS.textScale, value, "default"); apply(); });

export function setPalette(value: Palette) { palette.value = value; }
export function setInterfaceFont(value: InterfaceFont) { interfaceFont.value = value; }
export function setChatFont(value: ChatFont) { chatFont.value = value; }
export function setTextScale(value: TextScale) { textScale.value = value; }
/** One step larger or smaller, stopping at either end: the phone's A− / A+. */
export function stepTextScale(by: 1 | -1) {
  const next = TEXT_SCALES[TEXT_SCALES.indexOf(textScale.value) + by];
  if (next) textScale.value = next;
}

/**
 * On a touch screen the page itself does not zoom; text size does instead.
 *
 * A zoomed page fights a layout built to fit the screen: a focused input
 * zoomed it in uninvited, and a pinch left it somewhere it would not come
 * back from. The viewport's maximum-scale stops the first; Safari ignores it
 * for a pinch, so its gesture is refused here, and a double tap is declined
 * by touch-action in the stylesheet. A− / A+ in the drawer, or Text size in
 * Settings, is what makes things larger, and the layout reflows to it.
 */
export function holdPageScale() {
  if (typeof document === "undefined" || !matchMedia("(pointer: coarse)").matches) return;
  for (const name of ["gesturestart", "gesturechange"]) document.addEventListener(name, event => event.preventDefault(), { passive: false });
}
