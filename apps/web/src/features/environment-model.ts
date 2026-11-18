import type { EnvironmentOverrides, EnvironmentValue } from "@tendly/protocol";
import { AGENT_CLIENTS, clientInfo } from "./clients";

export interface EnvironmentRow {
  id: string;
  name: string;
  kind: EnvironmentValue["kind"];
  value: string;
}
const RESERVED = new Set([
  "HOME",
  "USERPROFILE",
  "PWD",
  "OLDPWD",
  ...AGENT_CLIENTS.flatMap(id => clientInfo(id).reservedEnvironment),
]);
let nextRow = 0;
export function newEnvironmentRow(): EnvironmentRow {
  return { id: "draft-" + ++nextRow, name: "", kind: "literal", value: "" };
}
export function environmentRows(values: EnvironmentOverrides = {}): EnvironmentRow[] {
  return Object.entries(values).map(([name, entry], index) => ({
    id: "saved-" + index + ":" + name,
    name,
    kind: entry.kind,
    value: entry.kind === "literal" ? entry.value : entry.kind === "secret_ref" ? entry.reference : "",
  }));
}
export function parseEnvironmentRows(rows: EnvironmentRow[]): { environment: EnvironmentOverrides; errors: string[] } {
  const errors = new Set<string>(),
    seen = new Set<string>(),
    entries: [string, EnvironmentValue][] = [];
  const bytes = (value: string) => new TextEncoder().encode(value).byteLength;
  if (rows.length > 64) errors.add("At most 64 environment overrides are allowed.");
  for (const row of rows) {
    const key = row.name.trim(),
      upper = key.toUpperCase();
    if (!/^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(key)) {
      errors.add("Use a valid environment variable name.");
      continue;
    }
    if (seen.has(key)) errors.add("Environment variable names must be unique.");
    seen.add(key);
    if (RESERVED.has(upper) || upper.startsWith("TENDLY_"))
      errors.add("Account directories, workspace identity and Tendly internal variables cannot be overridden.");
    if (row.kind === "unset") entries.push([key, { kind: "unset" }]);
    else if (row.kind === "main_model") entries.push([key, { kind: "main_model" }]);
    else if (row.kind === "main_model_context") entries.push([key, { kind: "main_model_context" }]);
    else if (row.kind === "secret_ref") {
      const reference = row.value.trim();
      if (!/^env:TENDLY_SECRET_[A-Z0-9_]+$/.test(reference))
        errors.add("Use env:TENDLY_SECRET_NAME for a secret reference.");
      entries.push([key, { kind: "secret_ref", reference }]);
    } else if (row.kind === "literal") {
      if (bytes(row.value) > 8192 || row.value.includes("\0"))
        errors.add("Environment values must be at most 8192 bytes and cannot contain NUL.");
      if (
        ["TOKEN", "SECRET", "PASSWORD", "PRIVATE_KEY", "API_KEY", "AUTHORIZATION"].some(marker =>
          upper.includes(marker),
        )
      )
        errors.add("Sensitive variables require a secret reference, not a plain value.");
      if (["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"].includes(upper) || upper.endsWith("_BASE_URL")) {
        const authority = (row.value.includes("://") ? row.value.split("://").slice(1).join("://") : row.value)
          .replace(/^\/\//, "")
          .split(/[/?#]/)[0];
        let credentials = authority.includes("@");
        try {
          const url = new URL(row.value.includes("://") ? row.value : "http://" + row.value.replace(/^\/\//, ""));
          credentials ||= !!url.username || !!url.password;
        } catch {
          /* Literal values are not interpreted as shell commands or URLs. */
        }
        if (credentials) errors.add("URLs containing credentials require a secret reference.");
      }
      entries.push([key, { kind: "literal", value: row.value }]);
    } else errors.add("Choose a supported environment value type.");
  }
  // Object.fromEntries preserves legal names like __proto__ as data properties.
  const environment = Object.fromEntries(entries);
  if (bytes(JSON.stringify(environment)) > 64 * 1024) errors.add("Environment overrides exceed the 64 KiB limit.");
  return { environment, errors: [...errors] };
}
export function mergeEnvironment(
  base: EnvironmentOverrides = {},
  overrides: EnvironmentOverrides = {},
): EnvironmentOverrides {
  return { ...base, ...overrides };
}
