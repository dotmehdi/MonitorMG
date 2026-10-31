"use strict";
// Fenêtre de confirmation : sans réponse, le backend rétablit l'affichage précédent.
const { invoke } = window.__TAURI__.core;
const left = document.getElementById("left");
let end = 0;
let timer;

function restart() {
  end = Date.now() + (window.SECS || 15) * 1000;
  clearInterval(timer);
  tick();
  timer = setInterval(tick, 250);
  // La fenêtre prend le focus : une frappe égarée (Entrée, Espace) doit rétablir, pas conserver.
  document.getElementById("revert").focus();
}

function tick() {
  left.textContent = Math.max(0, Math.ceil((end - Date.now()) / 1000));
}

const answer = (keep) => invoke("confirm", { keep });
document.getElementById("keep").addEventListener("click", () => answer(true));
document.getElementById("revert").addEventListener("click", () => answer(false));
document.addEventListener("keydown", (e) => e.key === "Escape" && answer(false));
document.addEventListener("contextmenu", (e) => e.preventDefault());
restart();
