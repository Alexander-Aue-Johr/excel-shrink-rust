import { build } from "esbuild";

await build({
  entryPoints: ["main.tsx"],
  bundle: true,
  outfile: "../assets/app.js",
  format: "esm",
  splitting: false,
  minify: true,
  jsx: "automatic",
  alias: { "@": "." },
  legalComments: "eof",
});
