import { createApp } from "vue";
import "@fontsource-variable/instrument-sans";
import "@fontsource-variable/jetbrains-mono";
import "./features/theme";
import "./features/appearance";
import ChatPreview from "./features/ChatPreview.vue";
import "./styles.css";

// A separate development entry. The production Vite build only includes index.html.
// This route uses local fixtures, never the user's sessions or an agent backend.
if (import.meta.env.DEV) createApp(ChatPreview).mount("#root");
