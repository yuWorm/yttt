import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development the console talks to a local control plane through this
// proxy, so the session cookie stays same-origin as in production.
const control = process.env.YTTT_CONTROL_URL ?? "http://127.0.0.1:18080";

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/v1": control,
      "/install": control,
    },
  },
  build: {
    outDir: "dist",
    // The control plane's CSP allows no inline scripts.
    modulePreload: { polyfill: false },
  },
});
