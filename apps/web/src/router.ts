import { createRouter, createWebHistory } from "vue-router";

/**
 * Where you are, in the address bar.
 *
 * The canvas is the app and stays mounted whatever the route, so a running
 * terminal or chat never reconnects because a page opened over it. The other
 * routes are pages drawn over the canvas by App.vue, which owns the data they
 * show; the route only says which one, and the browser's back button and a
 * reload both land where they should.
 */
const Page = { render: () => null };

export const SETTINGS_SECTIONS = ["preferences", "agents", "endpoints", "accounts", "remote", "machines", "updates"] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

export const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    { path: "/", name: "canvas", component: Page },
    { path: "/sessions", name: "sessions", component: Page },
    { path: "/system", name: "system", component: Page },
    { path: "/usage", name: "usage", component: Page },
    { path: "/settings/:section?", name: "settings", component: Page, beforeEnter: to => {
      const section = String(to.params.section ?? "");
      if (!(SETTINGS_SECTIONS as readonly string[]).includes(section)) return { name: "settings", params: { section: "preferences" }, replace: true };
    } },
    // A link to one session: it opens on the canvas, and the address returns to it.
    { path: "/session/:id", name: "session", component: Page },
    { path: "/:rest(.*)*", redirect: "/" },
  ],
});
