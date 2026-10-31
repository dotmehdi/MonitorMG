# MonitorMG · guide du logo

**L'idée :** un écran qui est aussi un interrupteur. Le cadre marine dit « écran », la pastille ambre dit « allumé ». Un clic, c'est réglé.

## Fichiers

| Besoin | Fichier |
| --- | --- |
| Logo principal | `final/monitormg-horizontal.svg` |
| Logo empilé | `final/monitormg-stacked.svg` |
| Symbole seul | `final/monitormg-symbol.svg` |
| Nom seul | `final/monitormg-wordmark.svg` |
| Sur fond sombre | les fichiers `-reversed` (cadre clair, pastille ambre) |
| Une couleur | `-black`, `-white`, `export/*/…-mono-13294b.svg` |
| Moins de 32 px | `final/monitormg-symbol-small.svg` (calé sur la grille de 16 px) |
| Icône de l'application | `export/app/monitormg.ico` (16 à 256 px), `monitormg-app-icon.svg`, PNG 16 à 1024 |
| Web | `export/symbol/` : `favicon.ico`, `favicon.svg`, icônes PWA, `site.webmanifest`, `head-snippet.html` |

## Couleurs

| | Encre | Ambre | Clair |
| --- | --- | --- | --- |
| HEX | `#13294B` | `#FFA300` | `#F4F6FA` |
| RGB | 19 41 75 | 255 163 0 | 244 246 250 |
| CMJN (approx.) | 100 76 0 55 | 0 41 100 0 | 2 1 0 2 |
| Pantone (approx.) | 2767 C | 137 C | |

Contrastes : encre sur blanc 14,6:1, ambre sur encre 7,3:1. L'ambre sur blanc (2:1) suffit pour la pastille mais **jamais pour du texte**. Les équivalences CMJN et Pantone sont à valider avec l'imprimeur.

## Règles

- **Zone de protection :** laisser autour du logo au moins la hauteur de la pastille.
- **Tailles minimales :** symbole 16 px (version petite taille sous 32 px), logo horizontal 120 px de large, 25 mm en impression.
- **Fonds :** blanc ou clair avec la version principale ; encre ou fond sombre avec la version `-reversed`. Sur une photo chargée, utiliser l'icône sur tuile.
- **Le nom** est dessiné sur mesure, en tracés : ne pas le retaper dans une police.
- **Texte d'accompagnement :** Segoe UI Variable, la police du système.

## À ne pas faire

- Changer les couleurs, mettre la pastille dans une autre couleur que l'ambre ou le monochrome.
- Déplacer la pastille à gauche (« éteint »), l'agrandir ou la remplacer par un rond.
- Étirer, incliner, ajouter ombre, contour ou dégradé.
- Placer la version encre sur un fond sombre : elle disparaît.

## Notes

- La géométrie se régénère avec `gen_logo.py` (cadre de 208 × 136, épaisseur 22, version claire amincie à 20 pour compenser l'irradiation).
- Aucune vérification de marque n'a été faite : prévoir une recherche (INPI, EUIPO, recherche d'image inversée) avant une publication commerciale.
