import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  // The Pages artifact is deployed as the site root. Relative assets also
  // keep the app compatible with repository pages and custom subpaths.
  base: "./",
  plugins: [react()],
});
