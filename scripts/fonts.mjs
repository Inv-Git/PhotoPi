// Copie la police Manrope (paquet @fontsource-variable/manrope) dans src/fonts,
// pour que l'app n'ait pas besoin d'internet pour l'afficher.
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const pkg = join(root, "node_modules", "@fontsource-variable", "manrope");
const out = join(root, "src", "fonts");

if (!existsSync(join(pkg, "index.css"))) {
  console.warn("Police Manrope absente : lance « npm install » (l'app utilisera Segoe UI en attendant).");
  process.exit(0);
}
mkdirSync(out, { recursive: true });
let css = readFileSync(join(pkg, "index.css"), "utf8");
css = css.replace(/url\(\.\/files\/([^)]+)\)/g, (_, f) => {
  copyFileSync(join(pkg, "files", f), join(out, f));
  return `url(./${f})`;
});
writeFileSync(join(out, "manrope.css"), css);
console.log("Police Manrope copiée dans src/fonts.");
