// Construit la release : installateur NSIS, archive portable et sommes SHA-256 dans dist/.
// Usage : node scripts/release.mjs
import { execFileSync, execSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const v = JSON.parse(readFileSync(join(root, "tauri.conf.json"), "utf8")).version;

execSync("npx --yes @tauri-apps/cli@2 build", { cwd: root, stdio: "inherit" });

const dist = join(root, "dist");
rmSync(dist, { recursive: true, force: true });
mkdirSync(dist);

const nsis = join(root, "target", "release", "bundle", "nsis");
const setup = readdirSync(nsis).find((f) => f.includes(`_${v}_`) && f.endsWith("-setup.exe"));
if (!setup) throw new Error(`installateur ${v} introuvable dans ${nsis}`);
copyFileSync(join(nsis, setup), join(dist, `MonitorMG-${v}-setup.exe`));

// Édition portable : le fichier « portable » garde profils, journal et données WebView2 dans data\.
const stage = join(dist, "portable");
mkdirSync(stage);
copyFileSync(join(root, "target", "release", "monitormg.exe"), join(stage, "MonitorMG.exe"));
// La licence MIT doit accompagner chaque copie redistribuée.
copyFileSync(join(root, "LICENSE"), join(stage, "LICENSE.txt"));
writeFileSync(join(stage, "portable"), "Ce fichier active le mode portable de MonitorMG.\r\n");
writeFileSync(
  join(stage, "LISEZMOI.txt"),
  [
    `MonitorMG ${v}, édition portable`,
    "",
    "1. Décompressez ce dossier dans un emplacement personnel (Documents, clé USB),",
    "   pas dans Téléchargements ni dans un dossier partagé.",
    "2. Lancez MonitorMG.exe. L'icône apparaît dans la zone de notification.",
    "",
    "Le fichier « portable » garde vos profils, le journal et les données de la fenêtre",
    "dans le dossier data, à côté de MonitorMG.exe. Rien n'est écrit ailleurs, sauf si",
    "vous activez « Lancer avec Windows » (une entrée dans le registre de votre compte).",
    "Si vous déplacez le dossier, réactivez cette option.",
    "",
    "Prérequis : Windows 10 ou 11 avec Microsoft Edge WebView2, déjà présent sur Windows 11 :",
    "https://go.microsoft.com/fwlink/p/?LinkId=2124703",
    "",
    "Confidentialité : MonitorMG ne se connecte jamais à Internet et ne collecte aucune donnée.",
    "",
    "Licence : MIT, logiciel libre fourni sans garantie. Texte complet dans LICENSE.txt.",
    "",
  ].join("\r\n"),
);
// tar de Windows (bsdtar) : il sait créer des .zip, contrairement au tar de Git.
const tar = join(process.env.SystemRoot ?? "C:\Windows", "System32", "tar.exe");
execFileSync(tar, ["-a", "-c", "-f", join(dist, `MonitorMG-${v}-portable.zip`), "-C", stage, "MonitorMG.exe", "portable", "LISEZMOI.txt", "LICENSE.txt"]);
rmSync(stage, { recursive: true });

const sums = readdirSync(dist)
  .sort()
  .map((f) => `${createHash("sha256").update(readFileSync(join(dist, f))).digest("hex")}  ${f}`);
writeFileSync(join(dist, "SHA256SUMS.txt"), sums.join("\n") + "\n");
console.log(`\nRelease ${v} :\n${sums.map((s) => "  " + s.split("  ")[1]).join("\n")}\n  SHA256SUMS.txt`);
