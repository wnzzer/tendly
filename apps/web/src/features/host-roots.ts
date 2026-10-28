import { json, request } from './api';

/**
 * Where on the host Tendly may browse and root a workspace: what the server
 * was started with, and folders added from Settings. Only a browser on the
 * machine Tendly runs on may add or remove one (`editable`).
 */
export interface HostRoots { configured: string[]; added: string[]; editable: boolean }

export const loadHostRoots = () => request<HostRoots>('/host/roots');
export const addHostRoot = (path: string) => request<HostRoots>('/host/roots', json('POST', { path }));
export const removeHostRoot = (path: string) => request<HostRoots>('/host/roots', json('DELETE', { path }));

/** The server's refusal of a workspace outside every folder it may use. */
export const isOutsideRoots = (message: string) => message.includes('outside the folders Tendly may use');
