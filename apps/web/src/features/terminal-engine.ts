/**
 * Which library draws the terminal.
 *
 * wterm lays the rows out as page text, so the browser scrolls them by the
 * pixel; xterm.js paints a canvas and moves it a whole row at a time. wterm
 * draws by default. `?terminal=xterm` in the address switches this browser
 * back to xterm.js, and `?terminal=wterm` returns it; the choice is remembered.
 */
export type TerminalEngine = "xterm" | "wterm";
const KEY = "tendly.terminalEngine";

export function terminalEngine(): TerminalEngine {
  try {
    const asked = new URL(window.location.href).searchParams.get("terminal");
    if (asked === "wterm" || asked === "xterm") localStorage.setItem(KEY, asked);
    return localStorage.getItem(KEY) === "xterm" ? "xterm" : "wterm";
  } catch {
    return "wterm";
  }
}

/**
 * Ghostty logs each sequence it ignores, such as the icon title many prompts
 * set on every line, straight to the console with no way to turn it off.
 */
let ghosttyQuiet = false;
export function quietGhostty() {
  if (ghosttyQuiet) return;
  ghosttyQuiet = true;
  const log = console.log;
  console.log = (...args: unknown[]) => {
    if (args[0] !== "[ghostty-vt]") log.apply(console, args);
  };
}
