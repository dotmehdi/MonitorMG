# Développement

## Architecture

Rust + [Tauri 2](https://tauri.app). L'interface est en HTML, CSS et JavaScript, sans framework ni étape de build (`ui/`), rendue par WebView2. La webview n'existe que tant qu'une fenêtre est ouverte : en zone de notification, il ne reste que le processus Rust (environ 3 Mo de mémoire privée).

| Fichier | Rôle |
| --- | --- |
| `src/display.rs` | Moteur d'affichage. Topologie, disposition et profils par l'API CCD (`QueryDisplayConfig` / `SetDisplayConfig`) ; modes par `ChangeDisplaySettingsEx`, seule API qui accepte une fréquence précise ; échelle par `DisplayConfigSetDeviceInfo` (valeurs non documentées -3/-4, celles de la page Paramètres) ; HDR par `SET_HDR_STATE` (24H2) ou `SET_ADVANCED_COLOR_STATE` ; luminosité par DDC/CI (`dxva2`, code VCP 0x10 ou celui choisi, ramené à la plage de l'écran) pour les écrans externes et WMI pour l'écran intégré ; autres codes VCP et chaîne de capacités MCCS (`vcp_caps`, `parse_caps`). Toute écriture passe par un verrou unique ; tout échange DDC/CI par le verrou du bus. |
| `src/main.rs` | Commandes, pipeline des actions (`act` : sérialisation, confirmation, journal, annulation), zone de notification, raccourcis globaux, détection des branchements, fenêtres. |
| `src/store.rs` | Profils et réglages (`config.json`), préférences DDC/CI par écran (`Screen`, `Feature`) et du panneau (`Panel`), mode portable, raccourcis bureau, couleur d'accent. |
| `src/journal.rs` | Journal d'activité : une ligne JSON par événement, fichier tournant à 512 Ko. Il nourrit la vue Activité et sert au diagnostic. |
| `ui/` | Interface : `index.html`, `app.js`, `style.css`, `i18n.js` (traduction anglaise), `vcp.js` (noms MCCS des codes et valeurs), le panneau de la zone de notification (`flyout.html`, `flyout.js`), plus les fenêtres de confirmation et d'identification. |
| `capabilities/` | Permissions par fenêtre : la fenêtre principale appelle les commandes de l'application, le panneau celles qu'il affiche, la confirmation une seule, l'identification aucune. |
| `windows/hooks.nsh` | Désinstallation : retire le lancement avec Windows et, sur demande, les données. |

### Profils

Un profil stocke la configuration CCD brute, sérialisée en hexadécimal. Les LUID d'adaptateur changent à chaque démarrage : on mémorise le chemin de chaque adaptateur pour les retrouver. Si un écran manque, le reste est appliqué ; si Windows refuse la configuration exacte, on lui laisse choisir les timings, puis la seule topologie.

### DDC/CI

Les capacités d'un écran (1 à 3 s de lecture) sont lues une fois depuis la vue DDC/CI et gardées dans ses préférences. Un code dont l'écran liste des valeurs est un réglage à choix, envoyé brut ; les autres sont continus et passent en % par leur plage. `ddc_set` n'écrit que les codes affichés dans le panneau, et `Screen::checked` refuse les codes de commande (remise à zéro…). Les préférences sont appliquées au moteur par `retune()` : code et plage de luminosité, blocage.

Le panneau (`flyout`) est créé à chaque ouverture et détruit dès qu'il perd le focus ; sa page demande sa hauteur à `flyout_fit`, qui l'ancre en bas à droite de son écran.

### Actions et annulation

Chaque modification d'affichage passe par `act()` : capture de l'état d'avant, application (sous confirmation si elle est risquée), entrée dans le journal avec sa source, mémorisation de l'état d'avant pour « Annuler ». La fenêtre de confirmation rétablit l'état d'origine si personne ne répond.

### Données

`%APPDATA%\MonitorMG` : `config.json`, `logs\activite.jsonl`. En édition portable (fichier `portable` à côté de l'exécutable), tout va dans `data\` à côté, y compris les données WebView2.

## Commandes

```
cargo test                     # 13 tests ; lit la config réelle et fait valider chaque opération par Windows, sans rien appliquer
cargo test -- --ignored        # test réel de la luminosité (modifie puis rétablit)
cargo clippy --all-targets     # aucun avertissement attendu
cargo audit                    # vulnérabilités connues des dépendances
node dev/serve.js ui 4517      # aperçu de l'interface dans un navigateur, avec un faux backend (dev/mock.js) ; ?lang=en pour l'anglais
node scripts/release.mjs       # installateur, édition portable et SHA256SUMS.txt dans dist/
```

## Publier une version

1. Mettre à jour `version` dans `Cargo.toml` et `tauri.conf.json`, et compléter `CHANGELOG.md`.
2. `cargo test`, `cargo clippy --all-targets`, `cargo audit`.
3. `node scripts/release.mjs`.
4. Tester l'installateur sur une session propre : installation, lancement avec Windows, mise à jour par-dessus la version précédente, désinstallation.
5. Publier les trois fichiers de `dist/`.

### Signature

Sans signature, SmartScreen affiche « Éditeur inconnu ». Une fois un certificat obtenu (OV, ou Azure Artifact Signing), il suffit de renseigner `bundle.windows.signCommand` dans `tauri.conf.json`, par exemple avec `signtool` :

```json
"signCommand": "signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /sha1 <empreinte du certificat> %1"
```

Tauri signe alors l'exécutable et l'installateur pendant `node scripts/release.mjs`.

## Conventions

- Interface en français ou en anglais selon la langue de Windows (`GetUserDefaultUILanguage`, forçable par la variable `MONITORMG_LANG=en` ou `fr`). Le texte français sert de clé : `t()` et le dictionnaire `EN` de `ui/i18n.js`, `store::t` et `tr!` côté Rust. Tout nouveau texte reçoit sa traduction.
- Phrases courtes, pas de tiret long, ponctuation française en français ; règles « craft » (pas de dégradé, de halo, de `transition: all`, espacements sur une base de 4 px).
- Tout nouvel appel exposé à l'interface doit être ajouté à `COMMANDS` dans `build.rs` et à la capacité de la fenêtre qui l'utilise.
- Une modification d'affichage passe par `act()` pour être journalisée et annulable.
