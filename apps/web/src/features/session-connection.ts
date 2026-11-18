import { createSessionConnections } from "./session-connection-state";
import { json, request } from "./api";
import type { Session } from "@tendly/protocol";
export const sessionConnections = createSessionConnections(session =>
  request<Session>(`/sessions/${encodeURIComponent(session.id)}/start`, json("POST")),
);
