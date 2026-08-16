import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  base: "/zk-jam/",
  plugins: [react()],
});
