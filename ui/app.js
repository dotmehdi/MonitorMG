"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const PLUG = ""; // écran avec sa prise : « appliqué au branchement »
const CLOCK = "\uE823"; // horloge : « déclencheurs automatiques »
const GLYPH = { ok: "", warn: "", error: "", progress: "" };

const icon = (glyph) => el("span", { class: "i", "aria-hidden": "true", text: glyph });

let S = null; // état renvoyé par le backend
let ACT = null; // activité récente, de la plus récente à la plus ancienne
let VIEW = "layout";
let sel = null; // clé de l'écran sélectionné
let view = null; // projection monde -> canevas
let drag = null; // glissement en cours
let sliding = false; // curseur de luminosité tenu
let busy = 0;

// ------------------------------------------------------------ données

let refreshTimer;
function refreshSoon(ms = 120) {
  clearTimeout(refreshTimer);
  refreshTimer = setTimeout(refresh, ms);
}

async function refresh() {
  if (drag || sliding) return refreshSoon(300);
  try {
    S = await invoke("state");
  } catch (e) {
    stageMessage(t("Impossible de lire les écrans. {0}", e));
    return;
  }
  const root = document.documentElement.style;
  root.setProperty("--accent-d", S.accent[0]);
  root.setProperty("--accent-l", S.accent[1]);
  if (!S.monitors.some((m) => m.key === sel)) {
    sel = (S.monitors.find((m) => m.primary) ?? S.monitors[0])?.key ?? null;
  }
  render();
}

function setBusy(d) {
  busy += d;
  $("#main").setAttribute("aria-busy", String(busy > 0));
  document.body.classList.toggle("busy", busy > 0);
}

/**
 * Appelle le backend, une modification à la fois. Les changements d'affichage sont
 * annoncés par l'événement « activity » ; `ok` sert aux autres commandes.
 */
async function run(cmd, args = {}, ok) {
  if (busy) {
    toast(t("Une modification est déjà en cours."), "warn");
    return undefined;
  }
  setBusy(1);
  const slow = setTimeout(() => progress(true), 600);
  try {
    const r = await invoke(cmd, args);
    const msg = typeof ok === "function" ? ok(r) : ok;
    if (msg) toast(msg);
    return r;
  } catch (e) {
    toast(String(e), "error");
    return undefined;
  } finally {
    clearTimeout(slow);
    progress(false);
    setBusy(-1);
    refreshSoon();
  }
}

// ------------------------------------------------------------ rendu

/** Reconstruit l'interface en gardant le focus clavier sur le même contrôle. */
function render() {
  const f = document.activeElement;
  const back = f?.id
    ? `#${CSS.escape(f.id)}`
    : f?.dataset?.key
      ? `[data-key="${CSS.escape(f.dataset.key)}"]`
      : f?.type === "radio"
        ? `input[name="${CSS.escape(f.name)}"]:checked`
        : null;
  renderProfiles();
  renderCanvas();
  renderInspector();
  renderSettings();
  renderDdc();
  if (VIEW === "activity") renderActivity();
  for (const r of document.querySelectorAll("#topology input")) r.checked = r.value === S.topology;
  if (back && (document.activeElement === document.body || !document.activeElement)) {
    document.querySelector(back)?.focus({ preventScroll: true });
  }
}

function showView(v) {
  VIEW = v;
  for (const b of document.querySelectorAll(".nav-item")) {
    if (b.dataset.view === v) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  }
  for (const s of ["layout", "ddc", "activity", "settings"]) $(`#view-${s}`).hidden = s !== v;
  if (v === "activity") {
    $("#activity-dot").hidden = true;
    loadActivity();
  }
  if (v === "ddc" && S) {
    renderDdc();
    loadCaps();
  }
  if (v === "layout" && S) renderCanvas();
}

function stageMessage(text) {
  const m = $("#stage-msg");
  m.textContent = text ?? "";
  m.hidden = !text;
}

const numbered = () => S.monitors.map((m, i) => ({ ...m, n: i + 1 }));
/** Écran qui porte l'image : lui-même, ou celui qu'il duplique. Sa tuile le représente. */
const sourceOf = (key) => S.monitors.find((m) => m.key === key)?.mirror_of ?? key;

function renderCanvas() {
  const canvas = $("#canvas");
  const parked = $("#parked");
  const box = $("#stage").getBoundingClientRect();
  if (!box.width) return; // vue masquée : on dessinera à son retour
  canvas.replaceChildren();
  parked.replaceChildren();
  const all = numbered();
  const act = all.filter((m) => m.active && !m.mirror_of);
  const off = all.filter((m) => !m.active);

  for (const m of off) {
    parked.append(
      el(
        "div",
        { class: "parked-item" },
        el("button", {
          class: "parked-name",
          "data-key": m.key,
          "aria-pressed": String(m.key === sel),
          onclick: () => select(m.key),
        }, el("span", { class: "parked-num", text: m.n }), m.name, el("span", { class: "parked-state", text: t("désactivé") })),
        el("button", { class: "btn", text: t("Activer"), onclick: () => setEnabled(m, true) }),
      ),
    );
  }

  if (!act.length) return stageMessage(t("Aucun écran actif détecté."));
  stageMessage(null);

  const bottom = off.length ? 56 : 0;
  const minX = Math.min(...act.map((m) => m.x));
  const minY = Math.min(...act.map((m) => m.y));
  const bw = Math.max(...act.map((m) => m.x + m.w)) - minX;
  const bh = Math.max(...act.map((m) => m.y + m.h)) - minY;
  // Marge autour de la disposition : de la place pour glisser un écran sur les côtés.
  const avg = act.reduce((s, m) => s + m.w, 0) / act.length;
  const k = Math.min((box.width - 48) / (bw + avg * 0.3), (box.height - 48 - bottom) / (bh + avg * 0.12));
  view = {
    k,
    minX,
    minY,
    ox: (box.width - bw * k) / 2,
    oy: (box.height - bottom - bh * k) / 2,
  };

  for (const m of act) canvas.append(tile(m, all.filter((o) => o.mirror_of === m.key), act));
}

function place(node, x, y, w, h) {
  node.style.left = `${view.ox + (x - view.minX) * view.k}px`;
  node.style.top = `${view.oy + (y - view.minY) * view.k}px`;
  node.style.width = `${w * view.k}px`;
  node.style.height = `${h * view.k}px`;
}

function moveTo(m, others, to) {
  const list = others.map((o) => [o.key, o.x, o.y]).concat([[m.key, Math.round(to.x), Math.round(to.y)]]);
  return run("set_positions", { list });
}

/** Tuile d'un écran ; des écrans dupliqués partagent la leur, numérotée « 1 | 2 » comme dans Windows. */
function tile(m, mirrors, act) {
  const group = [m, ...mirrors];
  const name = group.map((o) => o.name).join(" + ");
  const b = el(
    "button",
    {
      class: "tile" + (act.length < 2 ? " single" : ""),
      "aria-pressed": String(m.key === sourceOf(sel)),
      "aria-label": t("Écran {0}, {1}, {2} par {3}", group.map((o) => o.n).join(t(" et ")), name, m.w, m.h) + (m.primary ? t(", principal") : ""),
      "aria-keyshortcuts": act.length > 1 ? "ArrowLeft ArrowRight ArrowUp ArrowDown" : null,
      // Les petites tuiles n'affichent que leur numéro : l'infobulle garde le reste lisible.
      title: `${name}\n${m.w} × ${m.h} · ${m.hz} Hz`,
      "data-key": m.key,
    },
    el("span", { class: "num", text: group.map((o) => o.n).join(" | ") }),
    el("span", { class: "name", text: name }),
    el("span", { class: "res", text: `${m.w} × ${m.h} · ${m.hz} Hz` }),
    m.primary && el("span", { class: "taskbar", "aria-hidden": "true" }),
  );
  place(b, m.x, m.y, m.w, m.h);
  b.style.setProperty("--num", `${Math.round(Math.max(16, Math.min(56, m.h * view.k * 0.3)))}px`);

  // Un clic sur la tuile d'écrans dupliqués garde celui déjà choisi parmi eux.
  b.addEventListener("click", () => {
    if (!b.dataset.dragged && sourceOf(sel) !== m.key) select(m.key);
  });
  if (act.length < 2) return b;
  const others = act.filter((o) => o.key !== m.key);

  // Clavier : la flèche cherche la première place aimantée dans sa direction.
  b.addEventListener("keydown", (e) => {
    const dir = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key];
    if (!dir) return;
    e.preventDefault();
    select(m.key);
    for (let step = 0.25; step <= 4; step += 0.25) {
      const to = snap({ x: m.x + dir[0] * m.w * step, y: m.y + dir[1] * m.h * step, w: m.w, h: m.h }, others);
      if ((to.x - m.x) * dir[0] + (to.y - m.y) * dir[1] > 0) return moveTo(m, others, to);
    }
  });

  b.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || busy) return;
    b.setPointerCapture(e.pointerId);
    drag = { m, node: b, sx: e.clientX, sy: e.clientY, moved: false };
  });
  b.addEventListener("pointermove", (e) => {
    if (!drag || drag.node !== b) return;
    const dx = e.clientX - drag.sx;
    const dy = e.clientY - drag.sy;
    if (!drag.moved && Math.hypot(dx, dy) < 4) return;
    drag.moved = true;
    b.classList.add("dragging");
    b.style.transform = `translate(${dx}px, ${dy}px)`;
    drag.to = snap({ x: m.x + dx / view.k, y: m.y + dy / view.k, w: m.w, h: m.h }, others);
    if (!drag.ghost) drag.ghost = $("#canvas").appendChild(el("div", { class: "ghost", "aria-hidden": "true" }));
    place(drag.ghost, drag.to.x, drag.to.y, m.w, m.h);
  });
  const end = (commit) => {
    if (!drag || drag.node !== b) return;
    const d = drag;
    drag = null;
    d.ghost?.remove();
    if (!d.moved) return;
    b.dataset.dragged = "1"; // le clic qui suit le glissement ne sélectionne pas
    b.classList.remove("dragging");
    const to = commit ? d.to : null;
    // Atterrissage animé sur la position aimantée, puis application.
    b.style.transform = to ? `translate(${(to.x - m.x) * view.k}px, ${(to.y - m.y) * view.k}px)` : "";
    if (!to || (to.x === m.x && to.y === m.y)) return setTimeout(() => delete b.dataset.dragged, 0);
    setTimeout(() => moveTo(m, others, to), 180);
  };
  b.addEventListener("pointerup", () => end(true));
  b.addEventListener("pointercancel", () => end(false));
  return b;
}

/** Sélection sans reconstruire le canevas : le focus clavier reste en place. */
function select(key) {
  sel = key;
  const src = sourceOf(key);
  for (const t of document.querySelectorAll("[data-key]")) t.setAttribute("aria-pressed", String(t.dataset.key === key || t.dataset.key === src));
  renderInspector();
}

// ------------------------------------------------------------ inspecteur

const ORIENT = [
  [0, t("Paysage")],
  [90, t("Portrait")],
  [180, t("Paysage retourné")],
  [270, t("Portrait retourné")],
];

function renderInspector() {
  const box = $("#inspector");
  box.replaceChildren();
  const all = numbered();
  const m = all.find((x) => x.key === sel);
  if (!m) return;
  const actives = all.filter((x) => x.active).length;
  const src = sourceOf(m.key);
  const group = all.filter((x) => x.active && (x.key === src || x.mirror_of === src));

  const meta = [el("span", { text: m.connector })];
  if (m.native) meta.push(el("span", { class: "dot", text: t("{0} × {1} natif", m.native[0], m.native[1]) }));
  if (m.primary) meta.push(el("span", { class: "badge", text: t("Principal") }));
  if (m.mirror_of) meta.push(el("span", { class: "dot", text: t("Dupliqué") }));

  const enabled = el("input", {
    id: "insp-active",
    type: "checkbox",
    role: "switch",
    class: "switch",
    "aria-label": t("Écran actif"),
    checked: m.active,
    disabled: m.active && actives < 2,
    onchange: (e) => setEnabled(m, e.target.checked),
  });

  box.append(
    el(
      "div",
      { class: "insp-head" },
      el(
        "div",
        { class: "insp-title" },
        el("h2", {}, el("span", { class: "insp-num", text: m.n }), m.name),
        el("p", { class: "meta" }, ...meta),
        group.length > 1 && memberPicker(group, m),
      ),
      el(
        "div",
        { class: "insp-actions" },
        m.active &&
          !m.primary &&
          !m.mirror_of &&
          el("button", { id: "insp-primary", class: "btn", onclick: () => run("set_primary", { key: m.key }) }, icon(""), t("Définir comme principal")),
        el("label", { class: "switch-row", title: m.active && actives < 2 ? t("Le dernier écran actif ne peut pas être désactivé") : null }, el("span", { text: m.active ? t("Actif") : t("Désactivé") }), enabled),
      ),
    ),
  );

  if (!m.active) {
    box.append(el("p", { class: "note", text: t("Cet écran est branché mais n'affiche rien. Activez-le pour régler sa résolution et sa place.") }));
    return;
  }
  // Un écran dupliqué suit l'image de sa source : seuls sa luminosité et son HDR lui sont propres.
  const own = !m.mirror_of;
  if (!own) {
    const name = all.find((x) => x.key === src)?.name ?? t("un autre écran");
    box.append(el("p", { class: "note", text: t("Affiche la même image que {0}. Résolution, mise à l'échelle et orientation se règlent depuis celui-ci.", name) }));
  }

  const cur = m.modes.find((md) => md.w === m.w && md.h === m.h);
  const resSelect = el(
    "select",
    {
      id: "insp-res",
      "aria-label": t("Résolution"),
      onchange: (e) => {
        const [w, h] = e.target.value.split("x").map(Number);
        const rates = m.modes.find((md) => md.w === w && md.h === h)?.rates ?? [];
        const hz = rates.includes(m.hz) ? m.hz : rates[0] ?? m.hz;
        run("set_mode", { key: m.key, w, h, hz });
      },
    },
    ...m.modes.map((md) =>
      el("option", {
        value: `${md.w}x${md.h}`,
        selected: md === cur,
        text: `${md.w} × ${md.h}` + (isNative(m, md) ? `  ${t("(natif)")}` : ""),
      }),
    ),
  );

  const rows = [
    m.brightness != null && row(t("Luminosité"), m.soft ? t("Logicielle : cet écran ne répond pas en DDC/CI, un voile l'assombrit") : null, brightnessSlider(m), true),
    own && row(t("Résolution"), null, resSelect),
    own && m.scale && row(t("Mise à l'échelle"), t("Taille du texte et des fenêtres"), scaleSelect(m)),
    own &&
      row(
        t("Fréquence"),
        t("Images par seconde (Hz)"),
        el(
          "div",
          { class: "chips", role: "radiogroup", "aria-label": t("Fréquence de rafraîchissement") },
          ...(cur?.rates ?? [m.hz]).map((hz) =>
            el(
              "label",
              { class: "chip" },
              el("input", {
                type: "radio",
                name: "hz",
                value: hz,
                checked: hz === m.hz,
                onchange: () => run("set_mode", { key: m.key, w: m.w, h: m.h, hz }),
              }),
              el("span", { text: hz }),
            ),
          ),
        ),
        true,
      ),
    own &&
      row(
        t("Orientation"),
        null,
        el(
          "select",
          { id: "insp-orient", "aria-label": t("Orientation"), onchange: (e) => run("set_rotation", { key: m.key, deg: Number(e.target.value) }) },
          ...ORIENT.map(([deg, label]) => el("option", { value: deg, selected: deg === m.rotation, text: label })),
        ),
      ),
    m.hdr !== null &&
      row(
        "HDR",
        t("Plage dynamique étendue"),
        el("input", {
          id: "insp-hdr",
          type: "checkbox",
          role: "switch",
          class: "switch",
          "aria-label": "HDR",
          checked: m.hdr,
          onchange: (e) => run("set_hdr", { key: m.key, on: e.target.checked }),
        }),
      ),
  ].filter(Boolean);
  // Une ligne seule en fin de grille prend toute la largeur.
  const narrow = rows.filter((r) => !r.classList.contains("wide"));
  if (narrow.length % 2) narrow.at(-1).classList.add("wide");
  box.append(el("div", { class: "settings" }, ...rows));
}

/** Écrans dupliqués : une tuile pour tous, on choisit ici celui à régler. */
function memberPicker(group, m) {
  return el(
    "div",
    { class: "segmented members", role: "radiogroup", "aria-label": t("Écrans dupliqués") },
    ...group.map((x) =>
      el(
        "label",
        {},
        el("input", {
          type: "radio",
          name: "member",
          value: x.key,
          checked: x.key === m.key,
          onchange: () => {
            select(x.key);
            $('input[name="member"]:checked')?.focus();
          },
        }),
        el("span", {}, el("b", { text: x.n }), x.name),
      ),
    ),
  );
}

function isNative(m, md) {
  if (!m.native) return false;
  const [w, h] = m.native;
  return (md.w === w && md.h === h) || (md.w === h && md.h === w);
}

function scaleSelect(m) {
  return el(
    "select",
    { id: "insp-scale", "aria-label": t("Mise à l'échelle"), onchange: (e) => run("set_scale", { key: m.key, pct: Number(e.target.value) }) },
    ...m.scale.options.map((p) =>
      el("option", {
        value: p,
        selected: p === m.scale.current,
        text: `${p}${NNBSP}%` + (p === m.scale.recommended ? `  ${t("(recommandé)")}` : ""),
      }),
    ),
  );
}

/** DDC/CI est lent : une requête à la fois pendant le glissement, la dernière valeur gagne. */
function brightnessSlider(m) {
  const box = el("div", { class: "level" });
  const out = el("output", { class: "level-value" });
  const input = el("input", { id: "insp-bright", type: "range", min: 0, max: 100, step: 1, value: m.brightness, "aria-label": t("Luminosité") });
  const show = (v) => {
    box.style.setProperty("--v", v);
    out.textContent = `${v}${NNBSP}%`;
    input.setAttribute("aria-valuetext", `${v}${NNBSP}%`);
  };
  const send = latest(
    async (pct) => {
      await invoke("set_brightness", { key: m.key, pct });
      const live = S.monitors.find((x) => x.key === m.key);
      if (live) live.brightness = pct;
    },
    (e) => {
      toast(String(e), "error");
      refreshSoon();
    },
  );
  input.addEventListener("input", () => {
    show(input.valueAsNumber);
    send(input.valueAsNumber);
  });
  // Pas de rafraîchissement tant que le curseur est tenu : il serait reconstruit sous le pointeur.
  input.addEventListener("pointerdown", () => {
    sliding = true;
    document.addEventListener("pointerup", () => (sliding = false), { once: true });
  });
  show(m.brightness);
  box.append(el("span", { class: "slider" }, input), out);
  return box;
}

function row(label, hint, control, wide = false) {
  return el(
    "div",
    { class: "row" + (wide ? " wide" : "") },
    el("div", { class: "row-label" }, el("span", { text: label }), hint && el("small", { text: hint })),
    control,
  );
}

function setEnabled(m, on) {
  run("set_enabled", { key: m.key, on });
}

// ------------------------------------------------------------ DDC/CI

const CAPS = {}; // clé d'écran -> "loading" ou message d'erreur de la lecture des capacités
let capsRunning = false;

// Un écran dupliqué a ses propres réglages DDC/CI : seule l'image est commune.
const ddcCapable = (m) => m.active && !m.internal;
const hasCaps = (sc) => !!sc && Object.keys(sc.caps ?? {}).length > 0;
const safeId = (key) => key.replace(/[^\w-]/g, "");
let ddcStale = false; // vue à reconstruire une fois la saisie terminée
let screenSaves = Promise.resolve();

/**
 * Préférences d'un écran : enregistrées une à la fois, chacune calculée à partir du
 * dernier état enregistré (renvoyé par le backend), jamais d'un affichage périmé.
 * `change(s)` renvoie les champs à modifier.
 */
function saveScreen(key, change) {
  screenSaves = screenSaves.then(async () => {
    const cur = S.screens[key];
    try {
      S.screens[key] = await invoke("set_screen", { key, screen: { ...cur, ...change(cur) } });
    } catch (e) {
      toast(String(e), "error");
    }
    refreshSoon();
  });
}

/** Lit les commandes annoncées par chaque écran externe (1 à 3 s chacun), une seule fois. */
async function loadCaps(force = false) {
  if (capsRunning) return;
  capsRunning = true;
  for (const m of S.monitors.filter(ddcCapable)) {
    const sc = S.screens[m.key];
    if (sc?.blocked || (!force && (hasCaps(sc) || CAPS[m.key]))) continue;
    CAPS[m.key] = "loading";
    renderDdc();
    try {
      await invoke("ddc_caps", { key: m.key, force });
      delete CAPS[m.key];
    } catch (e) {
      CAPS[m.key] = String(e);
    }
  }
  capsRunning = false;
  refresh();
}

function renderDdc() {
  if (VIEW !== "ddc" || !S) return;
  // Pas de reconstruction sous un champ en cours de saisie : elle effacerait le texte.
  const f = document.activeElement;
  if (f?.closest("#ddc-screens") && f.matches("input:not([type=checkbox]), select")) {
    ddcStale = true;
    return;
  }
  ddcStale = false;
  $("#off-mode").value = S.panel.off;
  $("#power-signal").value = String(S.panel.power);
  const mons = S.monitors.filter((m) => m.active);
  $("#ddc-screens").replaceChildren(...mons.flatMap(ddcScreen));
}

/** Deux champs « de … à … % » pour une plage. */
function rangeInputs(id, range, label, onSave) {
  const field = (i, v) =>
    el("input", {
      id: `${id}-${i}`,
      class: "input num",
      type: "number",
      min: 0,
      max: 100,
      value: v,
      "aria-label": t(i ? "{0}, maximum en %" : "{0}, minimum en %", label),
      onchange: () => {
        const r = [Number(lo.value), Number(hi.value)];
        if (!(r[0] >= 0 && r[1] <= 100 && r[0] < r[1])) {
          toast(t("Plage invalide : le minimum doit rester sous le maximum"), "error");
          [lo.value, hi.value] = range;
          return;
        }
        onSave(r);
      },
    });
  const lo = field(0, range[0]);
  const hi = field(1, range[1]);
  return el("div", { class: "range-inputs" }, lo, el("span", { text: t("à") }), hi, el("span", { text: "%" }));
}

const switchInput = (id, checked, label, onchange) => el("input", { id, type: "checkbox", role: "switch", class: "switch", "aria-label": label, checked, onchange });

function ddcScreen(m) {
  const sc = S.screens[m.key];
  const k = safeId(m.key);
  // Champs à modifier, ou fonction du dernier état enregistré (listes de réglages).
  const save = (change) => saveScreen(m.key, typeof change === "function" ? change : () => change);
  const ext = !m.internal;
  const auto = S.auto_vcp[m.key] ?? 0x10;

  const rows = [
    row(
      t("Nom dans le panneau"),
      null,
      el("input", { id: `dn-${k}`, class: "input", value: sc.name, placeholder: m.name, maxlength: 40, "aria-label": t("Nom dans le panneau"), onchange: (e) => save({ name: e.target.value }) }),
    ),
    row(t("Afficher dans le panneau"), null, switchInput(`dh-${k}`, !sc.hidden, t("Afficher dans le panneau"), (e) => save({ hidden: !e.target.checked }))),
    row(t("Plage de luminosité"), t("Harmonise des écrans qui n'éclairent pas pareil"), rangeInputs(`dr-${k}`, sc.range, t("Plage de luminosité"), (range) => save({ range }))),
  ];
  if (ext) {
    rows.push(
      row(
        t("Code de la luminosité"),
        t("Si le curseur ne change rien, essayez un autre code"),
        el(
          "select",
          { id: `dv-${k}`, "aria-label": t("Code de la luminosité"), onchange: (e) => save({ vcp: e.target.value ? Number(e.target.value) : null }) },
          el("option", { value: "", selected: sc.vcp == null, text: t("Automatique ({0})", hex(auto)) }),
          ...[0x10, 0x13, 0x6b, 0x12].map((c) => el("option", { value: c, selected: sc.vcp === c, text: `${hex(c)} · ${vcpName(c)}` })),
        ),
      ),
      row(t("Bloquer DDC/CI"), t("Pour un écran qui ralentit ou fige le PC quand on l'interroge"), switchInput(`db-${k}`, sc.blocked, t("Bloquer DDC/CI"), (e) => save({ blocked: e.target.checked }))),
    );
  }

  if (ext && !sc.blocked) rows.push(...inputToggleRows(m, sc, k, save));

  const out = [el("h2", { class: "group-title" }, m.name, el("span", { class: "group-meta", text: m.connector })), el("div", { class: "stack" }, ...rows)];
  if (!ext || sc.blocked) return out;

  const state = CAPS[m.key];
  const feats = [];
  if (state === "loading") {
    feats.push(el("p", { class: "note", text: t("Lecture des commandes de l'écran…") }));
  } else if (state) {
    feats.push(
      el(
        "div",
        { class: "row" },
        el("div", { class: "row-label" }, el("span", { text: t("Commandes illisibles") }), el("small", { text: `${state}. ${t("Vérifiez que DDC/CI est activé dans le menu de l'écran.")}` })),
        el("button", { class: "btn", onclick: () => (delete CAPS[m.key], loadCaps(true)) }, t("Réessayer")),
      ),
    );
  }
  const brightness = sc.vcp ?? auto;
  const codes = new Set(Object.keys(sc.caps).map(Number).filter((c) => !VCP_SYSTEM.has(c) && c !== brightness));
  for (const f of sc.features) codes.add(f.code);
  for (const code of [...codes].sort((a, b) => a - b)) feats.push(...featureRows(m, sc, code, save));
  feats.push(el("div", { class: "ddc-actions" }, el("button", { class: "btn", onclick: () => addCode(sc, save) }, icon(""), t("Ajouter un code VCP")), hasCaps(sc) && capsList(sc)));
  out.push(el("h3", { class: "sub-title" }, t("Réglages dans le panneau")), el("div", { class: "stack" }, ...feats));
  return out;
}

/**
 * Bascule d'entrée par raccourci : un KVM logiciel pour un écran branché à deux machines.
 * Proposée dès que l'écran annonce au moins deux entrées.
 */
function inputToggleRows(m, sc, k, save) {
  const inputs = vcpValues(sc, VCP_INPUT);
  if (inputs.length < 2) return [];
  const pair = sc.inputs ?? [];
  const selects = [0, 1].map((i) =>
    el(
      "select",
      { id: `di${i}-${k}`, "aria-label": i ? t("Seconde entrée") : t("Première entrée"), onchange: onPick },
      el("option", { value: "", selected: pair[i] == null, text: "—" }),
      ...inputs.map((v) => el("option", { value: v, selected: pair[i] === v, text: vcpValueName(VCP_INPUT, v) })),
    ),
  );
  function onPick() {
    const [a, b] = selects.map((s) => (s.value ? Number(s.value) : null));
    if (a == null || b == null) {
      if (sc.inputs) save({ inputs: null });
      return;
    }
    if (a === b) {
      toast(t("Choisissez deux entrées différentes"), "error");
      selects.forEach((s, i) => (s.value = pair[i] ?? ""));
      return;
    }
    save({ inputs: [a, b] });
  }
  const keys = sc.input_hotkey ? keyParts(sc.input_hotkey) : null;
  const what = t("basculer l'entrée de {0}", m.name);
  const hotkey = el(
    "button",
    {
      id: `dk-${k}`,
      class: "btn",
      disabled: !sc.inputs && !keys,
      "aria-label": keys ? t("Raccourci pour {0} : {1}", what, keys.join("+")) : t("Choisir un raccourci pour {0}", what),
      onclick: async () => {
        const combo = await askHotkey(sc.input_hotkey, t("Appuyez sur la combinaison qui basculera l'entrée de {0}, où que vous soyez.", m.name));
        if (combo !== undefined) run("input_hotkey", { key: m.key, hotkey: combo });
      },
    },
    ...(keys ? keys.map((x) => el("kbd", { text: x })) : [t("Choisir…")]),
  );
  return [
    el(
      "div",
      { class: "row" },
      el("div", { class: "row-label" }, el("span", { text: t("Bascule d'entrée par raccourci") }), el("small", { text: t("Entre deux entrées, pour un écran partagé par deux machines") })),
      el("div", { class: "range-inputs" }, selects[0], el("span", { text: t("et") }), selects[1]),
    ),
    el(
      "div",
      { class: "row sub" },
      el(
        "div",
        { class: "row-label" },
        el("span", { text: t("Raccourci") }),
        el("small", { text: sc.inputs ? t("Change la source affichée : l'écran peut quitter ce PC") : t("Choisissez d'abord les deux entrées") }),
      ),
      hotkey,
    ),
  ];
}

function featureRows(m, sc, code, save) {
  const k = `${safeId(m.key)}-${code}`;
  const f = sc.features.find((x) => x.code === code);
  const values = vcpValues(sc, code);
  const kind = code === VCP_POWER ? t("bouton marche/arrêt") : values.length ? t("{0} choix", values.length) : t("curseur");
  const warn = { [VCP_INPUT]: t("change la source affichée, l'écran peut quitter ce PC"), [VCP_POWER]: t("certains écrans ne se rallument qu'avec leur bouton") }[code];
  const toggle = switchInput(`df-${k}`, !!f, t("Afficher {0} dans le panneau", vcpName(code)), (e) => {
    const on = e.target.checked;
    save((s) => ({ features: [...s.features.filter((x) => x.code !== code), ...(on ? [{ code, range: [0, 100], linked: false, stop: 100 }] : [])] }));
  });
  const label = el(
    "div",
    { class: "row-label feature-label" },
    el("span", {}, icon(vcpIcon(code)), vcpName(code)),
    el("small", { text: [hex(code), kind, warn].filter(Boolean).join(" · ") }),
  );
  const rows = [el("div", { class: "row" }, label, toggle)];
  if (!f || values.length || code === VCP_POWER) return rows;

  const patch = (p) => save((s) => ({ features: s.features.map((x) => (x.code === code ? { ...x, ...p } : x)) }));
  const linked = switchInput(`dl-${k}`, f.linked, t("Suit la luminosité"), (e) => patch({ linked: e.target.checked }));
  const stop = el("input", {
    id: `ds-${k}`,
    class: "input num",
    type: "number",
    min: 1,
    max: 100,
    value: f.stop,
    "aria-label": t("Luminosité à laquelle {0} atteint son maximum, en %", vcpName(code)),
    onchange: (e) => patch({ stop: Math.min(100, Math.max(1, Number(e.target.value) || 100)) }),
  });
  rows.push(
    el(
      "div",
      { class: "row sub" },
      el("div", { class: "row-label" }, el("span", { text: t("Plage") }), el("small", { text: t("Partie de la plage de l'écran parcourue par le curseur") })),
      rangeInputs(`dfr-${k}`, f.range, vcpName(code), (range) => patch({ range })),
    ),
    el(
      "div",
      { class: "row sub" },
      el("div", { class: "row-label" }, el("span", { text: t("Suit la luminosité") }), el("small", { text: f.linked ? t("Atteint son maximum à ce niveau de luminosité") : t("Quitte le panneau et suit le curseur de luminosité") })),
      el("div", { class: "range-inputs" }, f.linked && stop, f.linked && el("span", { text: "%" }), linked),
    ),
  );
  return rows;
}

async function addCode(sc, save) {
  const v = await askName(t("Ajouter un code VCP"), "", t("Ajouter"), t("Code hexadécimal, par exemple 0x87"));
  if (v == null) return;
  const code = /^(0x)?[0-9a-f]{1,2}$/i.test(v) ? parseInt(v.replace(/^0x/i, ""), 16) : NaN;
  if (!(code > 0 && code <= 0xff)) return toast(t("Code invalide : entrez une valeur de 0x01 à 0xFF"), "error");
  if (VCP_SYSTEM.has(code)) return toast(t("Ce code est une commande (remise à zéro…), pas un réglage"), "error");
  if (sc.features.some((f) => f.code === code)) return toast(t("Ce réglage est déjà dans le panneau"), "warn");
  save((s) => ({ features: [...s.features.filter((f) => f.code !== code), { code, range: [0, 100], linked: false, stop: 100 }] }));
}

/** Tous les codes annoncés par l'écran, pour le diagnostic. */
function capsList(sc) {
  const codes = Object.keys(sc.caps).map(Number).sort((a, b) => a - b);
  return el(
    "details",
    { class: "caps" },
    el("summary", { text: t("{0} codes annoncés par l'écran", codes.length) }),
    el(
      "dl",
      {},
      ...codes.flatMap((c) => [
        el("dt", { text: hex(c) }),
        el("dd", { text: vcpName(c) + (sc.caps[c].length ? ` : ${sc.caps[c].map((v) => vcpValueName(c, v)).join(", ")}` : "") }),
      ]),
    ),
  );
}

// ------------------------------------------------------------ profils

function renderProfiles() {
  const list = $("#profile-list");
  list.replaceChildren();
  $("#profiles-empty").hidden = S.profiles.length > 0;
  S.profiles.forEach((p, idx) => {
    const n = p.tiles.length;
    const more = el(
      "button",
      {
        id: `more-${p.id}`,
        class: "icon-btn more",
        "aria-label": t("Options du profil {0}", p.name),
        "aria-haspopup": "menu",
        "aria-expanded": "false",
        onclick: (e) => profileMenu(p, idx, e.currentTarget),
      },
      icon(""),
    );
    const li = el(
      "li",
      { class: "profile", "aria-current": p.active ? "true" : null },
      el(
        "button",
        { id: `profile-${p.id}`, class: "p-main", title: t("Appliquer {0}", p.name), onclick: () => applyProfile(p) },
        thumb(p.tiles),
        el(
          "span",
          { class: "p-text" },
          el("span", { class: "p-name", text: p.name }),
          el(
            "span",
            { class: "p-sub" },
            // La vignette montre déjà le nombre d'écrans : la place va au raccourci.
            el("span", { class: p.auto || p.hotkey || triggersText(p) ? "sr-only" : null, text: n > 1 ? t("{0} écrans", n) : t("{0} écran", n) }),
            p.active && el("span", { class: "sr-only", text: t(", affichage actuel") }),
            p.auto && el("span", { class: "i", title: t("Appliqué au branchement de ses écrans"), "aria-label": t("Appliqué au branchement"), text: PLUG }),
            triggersText(p) && el("span", { class: "i", title: triggersText(p), "aria-label": triggersText(p), text: CLOCK }),
            p.hotkey && el("kbd", { title: t("Raccourci clavier"), text: keyParts(p.hotkey).join("+") }),
          ),
        ),
      ),
      more,
    );
    li.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      profileMenu(p, idx, more, e);
    });
    list.append(li);
  });
}

function thumb(tiles) {
  const box = el("span", { class: "thumb", "aria-hidden": "true" });
  if (!tiles.length) return box;
  const minX = Math.min(...tiles.map((t) => t.x));
  const minY = Math.min(...tiles.map((t) => t.y));
  const bw = Math.max(...tiles.map((t) => t.x + t.w)) - minX;
  const bh = Math.max(...tiles.map((t) => t.y + t.h)) - minY;
  const k = Math.min(52 / bw, 32 / bh);
  const ox = (52 - bw * k) / 2;
  const oy = (32 - bh * k) / 2;
  for (const t of tiles) {
    const i = el("i", { class: t.primary ? "main" : null });
    // 1 px de retrait : les écrans voisins restent distincts.
    Object.assign(i.style, {
      left: `${ox + (t.x - minX) * k}px`,
      top: `${oy + (t.y - minY) * k}px`,
      width: `${Math.max(3, t.w * k - 1)}px`,
      height: `${Math.max(3, t.h * k - 1)}px`,
    });
    box.append(i);
  }
  return box;
}

function applyProfile(p) {
  run("profile_apply", { id: p.id });
}

function profileMenu(p, idx, anchor, at) {
  const last = S.profiles.length - 1;
  openMenu(anchor, at, [
    { icon: "", label: t("Appliquer"), run: () => applyProfile(p) },
    {
      icon: PLUG,
      label: t("Appliquer au branchement"),
      checked: p.auto,
      run: () => run("profile_auto", { id: p.id, on: !p.auto }, p.auto ? t("Application au branchement retirée") : t("{0} s'appliquera au branchement de ses écrans", p.name)),
    },
    { icon: CLOCK, label: t("Déclencheurs…"), run: () => triggersProfile(p) },
    "-",
    { icon: "", label: t("Renommer…"), run: () => renameProfile(p) },
    { icon: "", label: t("Raccourci clavier…"), hint: p.hotkey ? keyParts(p.hotkey).join("+") : null, run: () => hotkeyProfile(p) },
    { icon: "", label: t("Raccourci sur le bureau"), run: () => run("profile_shortcut", { id: p.id }) },
    { icon: "", label: t("Réglages appliqués…"), run: () => extrasProfile(p) },
    { icon: "", label: t("Remplacer par l'affichage actuel…"), run: () => replaceProfile(p) },
    "-",
    { icon: "", label: t("Monter"), disabled: idx === 0, run: () => run("profile_move", { id: p.id, to: idx - 1 }) },
    { icon: "", label: t("Descendre"), disabled: idx === last, run: () => run("profile_move", { id: p.id, to: idx + 1 }) },
    "-",
    { icon: "", label: t("Supprimer…"), danger: true, run: () => deleteProfile(p) },
  ]);
}

async function saveProfile() {
  const name = await askName(t("Nouveau profil"), "", t("Enregistrer"));
  if (name) run("profile_save", { name });
}

/** Réglages facultatifs d'un profil : désactivés, l'écran garde sa valeur actuelle. */
async function extrasProfile(p) {
  const names = { scale: t("Mise à l'échelle"), hdr: "HDR", brightness: t("Luminosité"), windows: t("Place des fenêtres") };
  const rows = p.extras.map((x) =>
    el(
      "label",
      { class: "row" },
      el("span", { class: "row-label" }, el("span", { text: names[x.id] }), el("small", { text: x.stored ? t("Enregistré : {0}", x.stored) : t("Absent de ce profil") })),
      el("input", {
        type: "checkbox",
        role: "switch",
        class: "switch",
        checked: x.on && !!x.stored,
        disabled: !x.stored,
        onchange: async (e) => {
          try {
            await invoke("profile_include", { id: p.id, setting: x.id, on: e.target.checked });
          } catch (err) {
            e.target.checked = !e.target.checked;
            toast(String(err), "error");
          }
          refreshSoon();
        },
      }),
    ),
  );
  await openDialog(
    t("Réglages appliqués"),
    [
      el("p", { text: t("Désactivez un réglage pour garder sa valeur actuelle quand {0} est appliqué.", p.name) }),
      el("div", { class: "stack" }, ...rows),
      el("p", { class: "dialog-note", text: t("Disposition, activation, résolution, fréquence et orientation sont toujours appliquées.") }),
      el("p", { class: "dialog-note", text: t("Les fenêtres sont celles ouvertes à l'enregistrement du profil ; « Remplacer par l'affichage actuel » les met à jour.") }),
    ],
    [{ label: t("Fermer"), value: "ok", primary: true }],
  );
}

/** « Appliqué chaque jour à 22:00, au lancement de jeu.exe » ; vide sans déclencheur. */
function triggersText(p) {
  const tr = p.triggers ?? {};
  const parts = [
    tr.at != null && t("chaque jour à {0}", clock(tr.at)),
    tr.power === "ac" && t("au passage sur secteur"),
    tr.power === "dc" && t("au passage sur batterie"),
    tr.exe && t("au lancement de {0}", tr.exe),
  ].filter(Boolean);
  return parts.length ? t("Appliqué {0}", parts.join(", ")) : "";
}

/** Déclencheurs d'un profil : il s'applique seul à l'heure dite, au changement d'alimentation ou au lancement d'un programme. */
async function triggersProfile(p) {
  const tr = p.triggers ?? {};
  const at = el("input", { id: "trg-at", class: "input", type: "time", value: tr.at == null ? "" : clock(tr.at), "aria-label": t("Chaque jour à") });
  const power = el(
    "select",
    { id: "trg-power", "aria-label": t("Alimentation") },
    ...[
      ["", t("Jamais")],
      ["ac", t("Au passage sur secteur")],
      ["dc", t("Au passage sur batterie")],
    ].map(([value, text]) => el("option", { value, selected: (tr.power ?? "") === value, text })),
  );
  const exe = el("input", { id: "trg-exe", class: "input", value: tr.exe ?? "", placeholder: t("jeu.exe"), maxlength: 80, autocomplete: "off", spellcheck: "false", "aria-label": t("Au lancement de") });
  const r = await openDialog(
    t("Déclencheurs de {0}", p.name),
    [
      el("p", { text: t("Le profil s'applique seul quand l'un de ces événements se produit.") }),
      el(
        "div",
        { class: "stack triggers" },
        row(t("Chaque jour à"), t("Vide : jamais"), at),
        row(t("Alimentation"), t("Sur un portable"), power),
        row(t("Au lancement de"), t("Nom du programme"), exe),
      ),
      el("p", { class: "dialog-note", text: t("L'application au branchement des écrans se règle dans le menu du profil.") }),
    ],
    [
      { label: t("Enregistrer"), value: "ok", primary: true },
      { label: CANCEL, value: "cancel" },
    ],
  );
  if (r === "ok") run("profile_triggers", { id: p.id, triggers: { at: minutes(at.value), power: power.value || null, exe: exe.value.trim() } });
}

async function renameProfile(p) {
  const name = await askName(t("Renommer le profil"), p.name, t("Renommer"));
  if (name && name !== p.name) run("profile_rename", { id: p.id, name });
}

/** Écrase le profil : rien ne permet de revenir à l'ancien contenu, on demande d'abord. */
async function replaceProfile(p) {
  const r = await openDialog(
    t("Remplacer {0} ?", p.name),
    [el("p", { text: t("Le profil prendra la disposition et les réglages actuels. Son contenu actuel sera perdu.") })],
    [
      { label: t("Remplacer"), value: "ok", primary: true },
      { label: CANCEL, value: "cancel" },
    ],
  );
  if (r === "ok") run("profile_update", { id: p.id });
}

async function deleteProfile(p) {
  const r = await openDialog(
    t("Supprimer {0} ?", p.name),
    [el("p", { text: t("Son raccourci clavier et ses raccourcis sur le bureau ne fonctionneront plus.") })],
    [
      { label: t("Supprimer"), value: "ok", primary: true },
      { label: CANCEL, value: "cancel" },
    ],
  );
  if (r === "ok") run("profile_delete", { id: p.id });
}

// ------------------------------------------------------------ activité

async function loadActivity() {
  try {
    ACT = await invoke("activity");
  } catch {
    ACT = [];
  }
  renderActivity();
}

function dayLabel(ms) {
  const d = new Date(ms);
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return t("Aujourd'hui");
  if (d.toDateString() === yesterday.toDateString()) return t("Hier");
  return d.toLocaleDateString(LOCALE, { weekday: "long", day: "numeric", month: "long" });
}

function renderActivity() {
  const box = $("#activity-list");
  $("#undo-btn").disabled = !S?.undo;
  $("#undo-label").textContent = S?.undo ? t("Annuler : {0}", S.undo) : t("Annuler");
  const items = (ACT ?? []).filter((a) => a.level !== "info");
  if (!items.length) {
    box.replaceChildren(el("p", { class: "page-empty", text: t("Aucune activité pour l'instant. Chaque changement d'affichage apparaîtra ici, d'où qu'il vienne.") }));
    return;
  }
  const days = new Map();
  for (const a of items) {
    const d = dayLabel(a.at);
    if (!days.has(d)) days.set(d, []);
    days.get(d).push(a);
  }
  box.replaceChildren(
    ...[...days].flatMap(([d, list]) => [
      el("h2", { class: "day", text: d }),
      el(
        "ul",
        { class: "events" },
        ...list.map((a) =>
          el(
            "li",
            { class: `event ${a.level}` },
            el("time", { datetime: new Date(a.at).toISOString(), text: new Date(a.at).toLocaleTimeString(LOCALE, { hour: "2-digit", minute: "2-digit" }) }),
            el("span", { class: "i", "aria-label": { ok: t("Réussi"), warn: t("Avertissement"), error: t("Échec") }[a.level], text: GLYPH[a.level] }),
            el("span", { text: a.text }),
            el("span", { class: "event-source", text: a.source }),
          ),
        ),
      ),
    ]),
  );
}

function onActivity(a) {
  ACT?.unshift(a);
  if (VIEW === "activity") refreshSoon();
  else if (a.level === "warn" || a.level === "error") {
    const dot = $("#activity-dot");
    dot.hidden = false;
    dot.classList.toggle("error", a.level === "error" || dot.classList.contains("error"));
  }
  // Les erreurs de la fenêtre reviennent déjà par la commande elle-même.
  if (a.level === "info" || (a.level === "error" && a.source === t("Fenêtre"))) return;
  toast(a.text, a.level, a.undo ? { label: t("Annuler"), run: () => run("undo") } : null);
}

// ------------------------------------------------------------ paramètres

const TRAY_ICONS = [
  ["ecrans", "Écrans"],
  ["moniteur", "Moniteur"],
  ["double", "Double écran"],
  ["luminosite", "Luminosité"],
  ["pastille", "Logo"],
];

/** Aperçu de chaque icône sur une barre des tâches de la couleur de celle de Windows. */
function renderTrayIcons() {
  const box = $("#tray-icons");
  if (box.dataset.for === `${S.tray_icon}|${S.taskbar_light}`) return; // inchangé : on garde le focus
  box.dataset.for = `${S.tray_icon}|${S.taskbar_light}`;
  box.replaceChildren(
    ...TRAY_ICONS.map(([id, label]) => {
      const glyph =
        id === "pastille"
          ? el("img", { class: "glyph-img", src: `tray/${id}.svg`, alt: "" })
          : el("span", { class: "glyph", "aria-hidden": "true" });
      if (id !== "pastille") glyph.style.setProperty("--glyph", `url(tray/${id}.svg)`);
      return el(
        "label",
        { class: "icon-choice" },
        el("input", {
          type: "radio",
          name: "tray-icon",
          value: id,
          checked: id === S.tray_icon,
          onchange: () => run("set_tray_icon", { name: id }),
        }),
        el("span", { class: "taskbar-swatch" + (S.taskbar_light ? " light" : "") }, glyph),
        el("span", { class: "icon-name", text: t(label) }),
      );
    }),
  );
}

function renderSettings() {
  $("#autostart").checked = S.autostart;
  $("#confirm-on").checked = S.confirm.on;
  $("#confirm-secs").value = String(S.confirm.secs);
  $("#confirm-delay").hidden = !S.confirm.on;
  $("#confirm-off").hidden = S.confirm.on;
  $("#data-dir").textContent = S.data_dir;
  $("#about-version").textContent = `Version ${S.version}${S.portable ? t(", édition portable") : ""}`;
  renderTrayIcons();
  renderSchedule();
  $("#updates-on").checked = S.updates;
  $("#update-status").textContent = S.update ? t("MonitorMG {0} est disponible", S.update) : t("Version {0}", S.version);
  $("#update-hint").textContent = S.update ? t("Vous utilisez la version {0}", S.version) : upToDate ? t("MonitorMG est à jour") : "";
  $("#update-open").hidden = !S.update;
  for (const [id, combo, what] of [
    ["#off-hotkey", S.off_hotkey, t("éteindre les écrans")],
    ["#up-hotkey", S.up_hotkey, t("monter la luminosité")],
    ["#down-hotkey", S.down_hotkey, t("baisser la luminosité")],
  ]) {
    const keys = combo ? keyParts(combo) : null;
    $(id).replaceChildren(...(keys ? keys.map((k) => el("kbd", { text: k })) : [t("Choisir…")]));
    $(id).setAttribute("aria-label", keys ? t("Raccourci pour {0} : {1}", what, keys.join("+")) : t("Choisir un raccourci pour {0}", what));
  }
  for (const o of document.querySelectorAll("#step option, #id-lit option[value]:not([value='']), #id-dim option")) o.textContent = `${o.value}${NNBSP}%`;
  $("#id-lit").value = S.identify.lit == null ? "" : String(S.identify.lit);
  $("#id-dim").value = String(S.identify.dim);
  $("#step").value = String(S.panel.step);
  const rows = [
    ...S.profiles.filter((p) => p.hotkey).map((p) => [t("Appliquer {0}, partout dans Windows", p.name), keyParts(p.hotkey)]),
    ...S.monitors.filter((m) => S.screens[m.key]?.input_hotkey).map((m) => [t("Basculer l'entrée de {0}, partout dans Windows", m.name), keyParts(S.screens[m.key].input_hotkey)]),
    [t("Enregistrer l'affichage actuel"), ["Ctrl", "S"]],
    [t("Annuler la dernière modification"), ["Ctrl", "Z"]],
    [t("Identifier les écrans"), ["Ctrl", "I"]],
    [t("Appliquer le profil 1 à 9"), ["Ctrl", "1…9"]],
    [t("Choisir l'écran 1 à 9"), ["1…9"]],
    [t("Déplacer l'écran sélectionné"), ["←", "↑", "→", "↓"]],
    [t("Relire les écrans"), ["F5"]],
  ];
  $("#shortcuts").replaceChildren(...rows.flatMap(([label, keys]) => [el("dt", { text: label }), el("dd", {}, ...keys.map((k) => el("kbd", { text: k })))]));
}

// ------------------------------------------------------------ luminosité selon l'heure

let upToDate = false; // une vérification vient de ne rien trouver de plus récent

const SCHEDULE_MAX = 12; // comme Schedule::MAX_STEPS côté backend

const saveSchedule = (patch) => run("set_schedule", { schedule: { ...S.schedule, ...patch } });

/** Étape de la planification : heure, luminosité, retrait. Repérée par son heure, unique. */
function stepRow([min, pct]) {
  const edit = (change) => saveSchedule({ steps: S.schedule.steps.map((s) => (s[0] === min ? change(s) : s)) });
  const time = el("input", {
    id: `sched-t-${min}`,
    class: "input time",
    type: "time",
    value: clock(min),
    "aria-label": t("Heure de l'étape"),
    onchange: (e) => {
      const m = minutes(e.target.value);
      if (m == null || m === min) return void (e.target.value = clock(min));
      if (S.schedule.steps.some((s) => s[0] === m)) {
        toast(t("Une étape existe déjà à cette heure"), "error");
        e.target.value = clock(min);
        return;
      }
      edit((s) => [m, s[1]]);
    },
  });
  const level = el("input", {
    id: `sched-p-${min}`,
    class: "input num",
    type: "number",
    min: 0,
    max: 100,
    value: pct,
    "aria-label": t("Luminosité de l'étape, en %"),
    onchange: (e) => edit((s) => [s[0], clamp(Math.round(Number(e.target.value) || 0), 0, 100)]),
  });
  const remove = el(
    "button",
    { class: "icon-btn", title: t("Retirer l'étape"), "aria-label": t("Retirer l'étape de {0}", clock(min)), onclick: () => saveSchedule({ steps: S.schedule.steps.filter((s) => s[0] !== min) }) },
    icon("\uE711"),
  );
  return el("div", { class: "row sub" }, el("div", { class: "row-label" }, time), el("div", { class: "range-inputs" }, level, el("span", { text: "%" }), remove));
}

function addStep() {
  const steps = S.schedule.steps;
  // Deux heures après la dernière étape, à la même luminosité : il reste à l'ajuster.
  let min = steps.length ? (steps.at(-1)[0] + 120) % 1440 : 8 * 60;
  while (steps.some((s) => s[0] === min)) min = (min + 30) % 1440;
  saveSchedule({ steps: [...steps, [min, steps.at(-1)?.[1] ?? 80]] });
}

function renderSchedule() {
  const box = $("#sched");
  // Pas de reconstruction sous un champ en cours de saisie, ni sans changement.
  const f = document.activeElement;
  if (f?.closest("#sched") && f.matches("input:not([type=checkbox])")) return;
  const print = JSON.stringify(S.schedule);
  if (box.dataset.for === print) return;
  box.dataset.for = print;
  const { on, smooth, steps } = S.schedule;
  const rows = [
    el(
      "label",
      { class: "row" },
      el("span", { class: "row-label" }, el("span", { text: t("Régler la luminosité selon l'heure") }), el("small", { text: t("Tous les écrans réglables, chacun dans sa plage") })),
      switchInput("sched-on", on, t("Régler la luminosité selon l'heure"), (e) => saveSchedule({ on: e.target.checked })),
    ),
  ];
  if (on) {
    rows.push(
      el(
        "div",
        { class: "row" },
        el("div", { class: "row-label" }, el("span", { text: t("Étapes") }), el("small", { text: steps.length ? t("Chaque jour, à l'heure dite") : t("Ajoutez une heure et sa luminosité") })),
        el("button", { id: "sched-add", class: "btn", disabled: steps.length >= SCHEDULE_MAX, onclick: addStep }, icon("\uE710"), t("Ajouter une étape")),
      ),
      ...steps.map(stepRow),
      el(
        "label",
        { class: "row" },
        el(
          "span",
          { class: "row-label" },
          el("span", { text: t("Transition douce") }),
          el("small", { text: smooth ? t("La luminosité glisse d'une étape à la suivante, minute après minute") : t("La luminosité change d'un coup à chaque étape") }),
        ),
        switchInput("sched-smooth", smooth, t("Transition douce"), (e) => saveSchedule({ smooth: e.target.checked })),
      ),
      el("p", { class: "hint", text: t("Un réglage manuel reste en place jusqu'à l'étape suivante. Les réglages DDC/CI qui suivent la luminosité suivent aussi l'heure.") }),
    );
  }
  box.replaceChildren(...rows);
}

// ------------------------------------------------------------ raccourcis clavier

const KEY_NAMES = {
  Super: "Win",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Space: t("Espace"),
  PageUp: t("Pg préc"),
  PageDown: t("Pg suiv"),
  Insert: t("Inser"),
  Delete: t("Suppr"),
  Home: t("Début"),
  End: t("Fin"),
};

function keyParts(acc) {
  return acc.split("+").map((k) => KEY_NAMES[k] ?? k.replace(/^Digit|^Key/, "").replace(/^Numpad/, t("Pavé ")));
}

async function hotkeyProfile(p) {
  const combo = await askHotkey(p.hotkey, t("Appuyez sur la combinaison qui appliquera {0}, où que vous soyez.", p.name));
  if (combo !== undefined) run("profile_hotkey", { id: p.id, hotkey: combo });
}

const ACTIONS = {
  "@off": ["off_hotkey", "Appuyez sur la combinaison qui éteindra les écrans, où que vous soyez."],
  "@up": ["up_hotkey", "Appuyez sur la combinaison qui montera la luminosité de tous les écrans, où que vous soyez."],
  "@down": ["down_hotkey", "Appuyez sur la combinaison qui baissera la luminosité de tous les écrans, où que vous soyez."],
};

async function hotkeyAction(action) {
  const [field, text] = ACTIONS[action];
  const combo = await askHotkey(S[field], t(text));
  if (combo !== undefined) run("action_hotkey", { action, hotkey: combo });
}

/** Saisit une combinaison. Renvoie la nouvelle (null : effacée), ou undefined sans changement. */
async function askHotkey(current, text) {
  current = current ?? null;
  let combo = current;
  const keys = el("div", { class: "keys", "aria-live": "polite" });
  const warn = el("p", { class: "warn" });
  const draw = (parts) =>
    keys.replaceChildren(...(parts?.length ? parts.map((k) => el("kbd", { text: k })) : [el("span", { class: "muted", text: t("Aucun raccourci") })]));
  draw(combo && keyParts(combo));

  const dlg = $("#dlg");
  const onKey = (e) => {
    const plain = !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey;
    if (e.key === "Tab" || e.key === "Escape") return;
    if (plain && (e.key === "Enter" || e.key === " ") && e.target.tagName === "BUTTON") return;
    e.preventDefault();
    e.stopPropagation();
    const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
    if (["Control", "Alt", "Shift", "Meta", "OS", "AltGraph"].includes(e.key)) return draw(mods.map((k) => KEY_NAMES[k] ?? k));
    if (!mods.length && !/^F\d+$/.test(e.code)) {
      warn.textContent = t("Ajoutez Ctrl, Alt ou Win à la touche.");
      return;
    }
    warn.textContent = "";
    combo = [...mods, e.code].join("+");
    draw(keyParts(combo));
  };
  dlg.addEventListener("keydown", onKey, true);
  const r = await openDialog(
    t("Raccourci clavier"),
    [el("p", { text }), keys, warn],
    [
      { label: t("Enregistrer"), value: "ok", primary: true },
      { label: t("Effacer"), value: "clear" },
      { label: CANCEL, value: "cancel" },
    ],
  );
  dlg.removeEventListener("keydown", onKey, true);
  if (r === "ok" && combo !== current) return combo;
  if (r === "clear" && current) return null;
}

// ------------------------------------------------------------ menu, dialogues, notifications

function openMenu(anchor, at, items) {
  const menu = $("#menu");
  menu.replaceChildren(
    ...items.map((it) =>
      it === "-"
        ? el("div", { class: "sep", role: "separator" })
        : el(
            "button",
            {
              class: "menu-item" + (it.danger ? " danger" : ""),
              role: it.checked == null ? "menuitem" : "menuitemcheckbox",
              "aria-checked": it.checked == null ? null : String(it.checked),
              disabled: it.disabled,
              onclick: () => {
                menu.hidePopover();
                it.run();
              },
            },
            icon(it.icon),
            el("span", { text: it.label }),
            it.hint && el("kbd", { text: it.hint }),
            it.checked && el("span", { class: "i check", "aria-hidden": "true", text: "" }),
          ),
    ),
  );
  anchor.setAttribute("aria-expanded", "true");
  menu.addEventListener("toggle", (e) => e.newState === "closed" && anchor.setAttribute("aria-expanded", "false"), { once: true });
  menu.showPopover();
  const r = anchor.getBoundingClientRect();
  const x = at ? at.clientX : r.left;
  const y = at ? at.clientY : r.bottom + 4;
  menu.style.left = `${clamp(x, 8, innerWidth - menu.offsetWidth - 8)}px`;
  menu.style.top = `${y + menu.offsetHeight > innerHeight - 8 ? Math.max(8, (at ? y : r.top - 4) - menu.offsetHeight) : y}px`;
  menu.querySelector(".menu-item:not(:disabled)")?.focus();
}

$("#menu").addEventListener("keydown", (e) => {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  e.preventDefault();
  const items = [...$("#menu").querySelectorAll(".menu-item:not(:disabled)")];
  const i = items.indexOf(document.activeElement);
  items[(i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
});

/** Ouvre la boîte de dialogue ; renvoie la valeur du bouton choisi ("" si Échap). */
function openDialog(title, body, actions) {
  const dlg = $("#dlg");
  $("#dlg-title").textContent = title;
  $("#dlg-body").replaceChildren(...body);
  $("#dlg-actions").replaceChildren(
    ...actions.map((a) => el("button", { class: "btn" + (a.primary ? " primary" : ""), value: a.value, text: a.label })),
  );
  dlg.returnValue = "";
  dlg.showModal();
  // Focus sur le titre : rien ne paraît sélectionné à l'ouverture, le clavier part du haut.
  $("#dlg-title").focus();
  return new Promise((resolve) => dlg.addEventListener("close", () => resolve(dlg.returnValue), { once: true }));
}

async function askName(title, value, okLabel, label = t("Nom")) {
  const input = el("input", { id: "dlg-input", class: "input", value, maxlength: "40", autocomplete: "off", spellcheck: "false" });
  const r = openDialog(title, [el("label", { class: "field-label", for: "dlg-input", text: label }), input], [
    { label: okLabel, value: "ok", primary: true },
    { label: CANCEL, value: "cancel" },
  ]);
  input.focus();
  input.select();
  return (await r) === "ok" ? input.value.trim() || null : null;
}

/**
 * Notification. Succès : 5 s ; avertissement : 9 s ; erreur : jusqu'à fermeture.
 * Le survol ou le focus suspend la disparition. Seul le « Annuler » le plus récent reste.
 */
function toast(text, kind = "ok", action = null) {
  const box = $("#toasts");
  const n = el("div", { class: `toast ${kind}`, role: kind === "error" ? "alert" : null }, icon(GLYPH[kind]), el("span", { text }));
  let timer;
  const close = () => {
    clearTimeout(timer);
    n.classList.add("out");
    setTimeout(() => n.remove(), 180);
  };
  if (action) {
    for (const b of box.querySelectorAll(".undo")) b.remove();
    n.append(el("button", { class: "btn undo", text: action.label, onclick: () => (close(), action.run()) }));
  }
  n.append(el("button", { class: "icon-btn", "aria-label": t("Fermer la notification"), onclick: close }, icon("")));
  box.append(n);
  while (box.children.length > 3) box.firstElementChild.remove();
  if (kind === "error") return n;
  const arm = () => (timer = setTimeout(close, kind === "ok" ? 5000 : 9000));
  n.addEventListener("pointerenter", () => clearTimeout(timer));
  n.addEventListener("pointerleave", arm);
  n.addEventListener("focusin", () => clearTimeout(timer));
  n.addEventListener("focusout", arm);
  arm();
  return n;
}

/** Statut « en cours » pour les modifications qui prennent du temps (activer un écran). */
let progressToast = null;
function progress(on) {
  if (on && !progressToast) {
    progressToast = el("div", { class: "toast progress" }, icon(GLYPH.progress), el("span", { text: t("Application en cours…") }));
    $("#toasts").append(progressToast);
  } else if (!on && progressToast) {
    progressToast.remove();
    progressToast = null;
  }
}

// ------------------------------------------------------------ branchements

for (const b of document.querySelectorAll(".nav-item")) b.addEventListener("click", () => showView(b.dataset.view));
$("#add-profile").addEventListener("click", saveProfile);
$("#add-profile-empty").addEventListener("click", saveProfile);
$("#identify").addEventListener("click", () => run("identify_cmd"));
$("#undo-btn").addEventListener("click", () => run("undo"));
$("#open-data").addEventListener("click", () => run("open_data_dir"));
$("#off-hotkey").addEventListener("click", () => hotkeyAction("@off"));
$("#up-hotkey").addEventListener("click", () => hotkeyAction("@up"));
$("#down-hotkey").addEventListener("click", () => hotkeyAction("@down"));
const savePanel = (patch) => run("set_panel", { panel: { ...S.panel, ...patch } });
$("#step").addEventListener("change", (e) => savePanel({ step: Number(e.target.value) }));
$("#off-mode").addEventListener("change", (e) => savePanel({ off: e.target.value }));
$("#power-signal").addEventListener("change", (e) => savePanel({ power: Number(e.target.value) }));
$("#ddc-reread").addEventListener("click", () => loadCaps(true));
// Saisie terminée : la vue mise de côté pendant la frappe est reconstruite.
$("#ddc-screens").addEventListener("focusout", () => ddcStale && setTimeout(renderDdc, 0));
$("#sched").addEventListener("focusout", () => setTimeout(() => S && renderSchedule(), 0));
$("#updates-on").addEventListener("change", (e) =>
  run("set_updates", { on: e.target.checked }, e.target.checked ? t("Les mises à jour seront vérifiées une fois par jour") : t("Vérification des mises à jour désactivée")),
);
$("#update-check").addEventListener("click", async () => {
  const found = await run("check_update", {}, (v) => (v ? t("MonitorMG {0} est disponible", v) : t("MonitorMG est à jour")));
  if (found !== undefined) upToDate = !found;
});
$("#update-open").addEventListener("click", () => run("open_releases"));
$("#autostart").addEventListener("change", (e) => run("set_autostart", { on: e.target.checked }, e.target.checked ? t("MonitorMG se lancera avec Windows") : t("Lancement avec Windows désactivé")));
const saveConfirm = () => run("set_confirm", { on: $("#confirm-on").checked, secs: Number($("#confirm-secs").value) });
$("#confirm-on").addEventListener("change", saveConfirm);
$("#confirm-secs").addEventListener("change", saveConfirm);
const saveIdentify = () => run("set_identify", { lit: $("#id-lit").value === "" ? null : Number($("#id-lit").value), dim: Number($("#id-dim").value) });
$("#id-lit").addEventListener("change", saveIdentify);
$("#id-dim").addEventListener("change", saveIdentify);
$("#topology").addEventListener("change", (e) => run("set_topology", { kind: e.target.value }));

document.addEventListener("keydown", (e) => {
  if ($("#dlg").open || e.target.matches("input:not([type=radio]):not([type=checkbox]):not([type=range]), select")) return;
  const ctrl = e.ctrlKey && !e.altKey && !e.shiftKey;
  const digit = /^Digit([1-9])$/.exec(e.code)?.[1];
  if (ctrl && e.code === "KeyS") {
    e.preventDefault();
    saveProfile();
  } else if (ctrl && e.code === "KeyZ") {
    e.preventDefault();
    if (S?.undo) run("undo");
  } else if (ctrl && e.code === "KeyI") {
    e.preventDefault();
    run("identify_cmd");
  } else if (e.key === "F5") {
    e.preventDefault();
    refresh();
  } else if (ctrl && digit && S?.profiles[digit - 1]) {
    e.preventDefault();
    applyProfile(S.profiles[digit - 1]);
  } else if (!e.ctrlKey && !e.altKey && !e.metaKey && digit && S?.monitors[digit - 1]) {
    showView("layout");
    select(S.monitors[digit - 1].key);
  }
});

// Pas de clic droit « navigateur » hors des profils.
document.addEventListener("contextmenu", (e) => e.preventDefault());

new ResizeObserver(() => S && !drag && renderCanvas()).observe($("#stage"));

// Windows 10 n'a pas Mica : fond opaque.
navigator.userAgentData?.getHighEntropyValues(["platformVersion"]).then((v) => {
  if (parseInt(v.platformVersion, 10) < 13) document.documentElement.classList.add("solid");
});

listen("changed", () => refreshSoon());
listen("activity", (e) => onActivity(e.payload));
listen("show-view", (e) => showView(e.payload));
refresh().then(() => window.START_VIEW && showView(window.START_VIEW));
