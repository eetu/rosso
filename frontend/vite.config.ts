import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

// Dev server on :5173, proxying the backend's routes to :3008 so dev is
// same-origin like prod. `/auth` is the oauth2-proxy edge route — not a backend
// route, so it is deliberately not proxied; locally DEV_AUTH=1 bypasses the gate.
export default defineConfig({
  plugins: [sveltekit()],
  server: {
    proxy: {
      "/api": "http://localhost:3008",
      "/status": "http://localhost:3008",
    },
  },
});
