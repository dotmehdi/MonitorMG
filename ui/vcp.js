"use strict";
// Codes VCP du standard MCCS (DDC/CI) : nom, icône, et nom des valeurs des réglages à choix.
// Partagé par la fenêtre principale et le panneau de la zone de notification.

const VCP = {
  0x0c: ["Température de couleur", ""],
  0x10: ["Luminosité", "\uE706"],
  0x12: ["Contraste", "\uE793"],
  0x13: ["Rétroéclairage", "\uE706"],
  0x14: ["Préréglage couleur", "\uE790"],
  0x16: ["Gain rouge", "\uE790"],
  0x18: ["Gain vert", "\uE790"],
  0x1a: ["Gain bleu", "\uE790"],
  0x60: ["Entrée", "\uE839"],
  0x62: ["Volume", "\uE767"],
  0x6b: ["Rétroéclairage blanc", "\uE706"],
  0x6c: ["Noir rouge", "\uE790"],
  0x6e: ["Noir vert", "\uE790"],
  0x70: ["Noir bleu", "\uE790"],
  0x72: ["Gamma", "\uE793"],
  0x87: ["Netteté", "\uE9E9"],
  0x86: ["Mise à l'échelle de l'image", ""],
  0x8a: ["Saturation", "\uE790"],
  0x8d: ["Sourdine", "\uE74F"],
  0x90: ["Teinte", "\uE790"],
  0xcc: ["Langue du menu de l'écran", "\uE775"],
  0xd6: ["Alimentation", "\uE7E8"],
  0xdc: ["Mode d'affichage", "\uE7F4"],
};

const VCP_VALUES = {
  0x60: {
    0x01: "VGA 1",
    0x02: "VGA 2",
    0x03: "DVI 1",
    0x04: "DVI 2",
    0x05: "Composite 1",
    0x06: "Composite 2",
    0x07: "S-Vidéo 1",
    0x08: "S-Vidéo 2",
    0x0c: "Composantes 1",
    0x0d: "Composantes 2",
    0x0e: "Composantes 3",
    0x0f: "DisplayPort 1",
    0x10: "DisplayPort 2",
    0x11: "HDMI 1",
    0x12: "HDMI 2",
    0x1b: "USB-C",
  },
  0x14: {
    0x01: "sRGB",
    0x02: "Natif",
    0x03: "4000 K",
    0x04: "5000 K",
    0x05: "6500 K",
    0x06: "7500 K",
    0x07: "8200 K",
    0x08: "9300 K",
    0x09: "10000 K",
    0x0a: "11500 K",
    0x0b: "Personnalisé 1",
    0x0c: "Personnalisé 2",
    0x0d: "Personnalisé 3",
  },
  // Image d'une source qui n'a pas la définition de l'écran.
  0x86: {
    0x01: "1:1",
    0x02: "Proportions",
    0x03: "Hauteur maximale",
    0x04: "Largeur maximale",
    0x05: "Plein écran",
  },
  0x8d: { 0x01: "Son coupé", 0x02: "Son actif" },
  0xcc: {
    0x01: "Chinois traditionnel",
    0x02: "Anglais",
    0x03: "Français",
    0x04: "Allemand",
    0x05: "Italien",
    0x06: "Japonais",
    0x07: "Coréen",
    0x08: "Portugais",
    0x09: "Russe",
    0x0a: "Espagnol",
    0x0b: "Suédois",
    0x0c: "Turc",
    0x0d: "Chinois simplifié",
    0x0e: "Portugais (Brésil)",
  },
  0xdc: {
    0x00: "Standard",
    0x01: "Productivité",
    0x02: "Mixte",
    0x03: "Film",
    0x04: "Personnalisé",
    0x05: "Jeux",
    0x06: "Sport",
    0x07: "Professionnel",
  },
};

/** Codes jamais proposés comme réglages : commandes (remise à zéro…), valeurs en lecture
 *  seule et codes internes au protocole. Le backend refuse aussi les commandes. */
const VCP_SYSTEM = new Set([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x08, 0x0a, 0x0b, 0x1e, 0x1f, 0x52, 0xac, 0xae, 0xb0, 0xb2, 0xb6, 0xc0, 0xc6, 0xc8, 0xc9, 0xca, 0xdf]);
const VCP_POWER = 0xd6;
const VCP_INPUT = 0x60;

const hex = (c) => `0x${c.toString(16).toUpperCase().padStart(2, "0")}`;

function vcpName(code) {
  if (VCP[code]) return t(VCP[code][0]);
  return code >= 0xe0 ? t("Réglage du fabricant {0}", hex(code)) : t("Code {0}", hex(code));
}

const vcpIcon = (code) => VCP[code]?.[1] ?? "";

function vcpValueName(code, v) {
  // Gamma MCCS : valeur absolue (v + 100) / 100, de 1,0 à 3,55.
  if (code === 0x72) return (1 + v / 100).toLocaleString(LOCALE, { minimumFractionDigits: 1, maximumFractionDigits: 2 });
  const n = VCP_VALUES[code]?.[v];
  return n ? t(n) : hex(v);
}

/** Valeurs permises d'un réglage à choix, d'après les capacités de l'écran (clés JSON). */
const vcpValues = (screen, code) => screen?.caps?.[code] ?? [];
const vcpDiscrete = (screen, code) => vcpValues(screen, code).length > 0;
