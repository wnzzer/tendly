// First, so the sign-in token leaves the address before the router reads it.
import { linkSignIn } from "./features/link-sign-in";
import { createApp } from "vue";
import "@fontsource-variable/instrument-sans";
import "@fontsource-variable/jetbrains-mono";
import "./features/theme";
import { holdPageScale } from "./features/appearance";
import App from "./App.vue";
import { installMenuDismissal } from "./features/dismiss-menus";
import { installRovingKeys } from "./features/roving-keys";
import { installContextMenuPolicy } from "./features/native-context-menu";
import { router } from "./router";

holdPageScale();
installMenuDismissal();
installRovingKeys();
installContextMenuPolicy();
void linkSignIn.then(() => createApp(App).use(router).mount("#root"));
