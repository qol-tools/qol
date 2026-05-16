// Compile the three .jsx sources to .js so the HTML opens with plain file://
// (no @babel/standalone runtime, no XHR fetch of .jsx, no HTTP server).
//
// Run after editing any .jsx file:
//
//     cd qol-tray/diagram && npm install && npm run build
//
// data.js is plain JS and does not pass through here. Iterate on it freely
// and just refresh the browser.

import { transformFileAsync } from "@babel/core";
import { writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));

const SOURCES = ["tweaks-panel.jsx", "diagram.jsx", "app.jsx"];

// Classic runtime: the .jsx files use the global `React` (loaded via the
// React UMD CDN script in the HTML) and reference <React.Fragment> directly.
// Automatic runtime would inject `import { jsx } from 'react/jsx-runtime'`,
// which has no global equivalent and would 404 in the browser.
const BABEL_OPTS = {
  presets: [["@babel/preset-react", { runtime: "classic" }]],
  babelrc: false,
  configFile: false,
  sourceMaps: "inline",
};

const BANNER = "// AUTO-GENERATED from the matching .jsx file via build.mjs. Edit the .jsx, then run `npm run build`.\n";

// Each compiled file is wrapped in an IIFE so its top-level `const` / `let` /
// `function` declarations live in their own scope. Without this, files that
// both do `const { useEffect } = React` at the top blow up at parse time with
// "redeclaration of const useEffect" because classic <script> tags share the
// global lexical environment. (Babel-standalone gave each `type="text/babel"`
// script its own scope; we have to do the same explicitly.) Cross-file
// communication still works because everything that needs to be shared is
// already attached to `window` inside each .jsx file.
function wrap(code) {
  return `(function () {\n"use strict";\n${code}\n})();\n`;
}

for (const src of SOURCES) {
  const srcAbs = path.join(HERE, src);
  const outAbs = path.join(HERE, src.replace(/\.jsx$/, ".js"));
  const result = await transformFileAsync(srcAbs, BABEL_OPTS);
  if (!result || typeof result.code !== "string") {
    throw new Error(`babel returned no code for ${src}`);
  }
  await writeFile(outAbs, BANNER + wrap(result.code), "utf8");
  console.log(`compiled ${src} -> ${path.basename(outAbs)}`);
}
