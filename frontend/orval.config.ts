import { defineConfig } from "orval";

export default defineConfig({
  remera: {
    // Same live-server source your old gen:types script used.
    input: "http://localhost:8080/openapi.json",
    output: {
      // "tags-split" groups generated files by the `tag` you set on
      // each #[utoipa::path] handler (e.g. "Meta") — one file per group
      // instead of one giant file.
      mode: "tags-split",
      target: "src/lib/api/generated.ts",
      schemas: "src/lib/api/models",
      client: "axios",
      override: {
        mutator: {
          path: "src/lib/axios-instance.ts",
          name: "customInstance",
        },
      },
    },
  },
});
