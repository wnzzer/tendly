import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { LIMIT, normalizeSessions } from "./history-common.mjs";

/**
 * agy keeps each conversation in a database of its own encoding, and a summary
 * of every one in `antigravity-cli/cache/conversation_metadata.json`: its ID,
 * title, first prompt, workspace and when it last changed. That summary is
 * what is listed. A conversation reopens in agy itself, which still holds it,
 * so importing one carries no transcript.
 */
const MAX_METADATA = 16 * 1024 * 1024;
export const geminiSession = row => {
  const uri = Array.isArray(row.WorkspaceURIs) ? row.WorkspaceURIs.find(value => typeof value === "string" && value.startsWith("file://")) : undefined;
  if (!uri || row.Internal || row.Cloud || !(row.NumSteps > 0)) return undefined;
  let cwd; try { cwd = fileURLToPath(uri); } catch { return undefined; }
  return { id: row.ID, cwd, time: Date.parse(row.UpdatedAt) };
};
/** @param {string} cwd @param {string} configDir */
export async function geminiHistory(cwd, configDir, _configEnv, nativeId) {
  let conversations;
  try {
    const raw = await readFile(join(configDir, "antigravity-cli", "cache", "conversation_metadata.json"), "utf8");
    if (raw.length > MAX_METADATA) throw Error("Native history exceeds limit");
    conversations = JSON.parse(raw)?.conversations;
  } catch (error) {
    if (error instanceof Error && error.message === "Native history exceeds limit") throw error;
    return nativeId ? Promise.reject(Error("Native session not found")) : { items: [], truncated: false };
  }
  const rows = Object.values(conversations && typeof conversations === "object" ? conversations : {})
    .map(entry => entry?.summary)
    .filter(summary => summary && typeof summary === "object")
    .map(summary => ({ ...summary, name: summary.Title, preview: summary.Preview }));
  const items = await normalizeSessions(rows, "gemini", cwd, geminiSession);
  items.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  if (nativeId) {
    if (!items.some(item => item.id === nativeId)) throw Error("Native session not found");
    return { items: [], truncated: false, messages: [] };
  }
  return { items: items.slice(0, LIMIT), truncated: items.length > LIMIT };
}
