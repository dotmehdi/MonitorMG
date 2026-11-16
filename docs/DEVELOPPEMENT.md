# Développement

## Architecture

Rust + [Tauri 2](https://tauri.app). L'interface est en HTML, CSS et JavaScript, sans framework ni étape de build (`ui/`), rendue par WebView2. La webview n'existe que tant qu'une fenêtre est ouverte : en zone de notification, il ne reste que le processus Rust (environ 3 Mo de mémoire privée).

| Fichier | Rôle |
| --- | --- |
| `src/display.rs` | Moteur d'affichage. Topologie, disposition et profils par l'API CCD (`QueryDisplayConfig` / `SetDisplayConfig`) ; modes par `ChangeDisplaySettingsEx`, seule API qui accepte une fréquence précise ; échelle par `DisplayConfigSetDeviceInfo` (valeurs non documentées -3/-4, celles de la page Paramètres) ; HDR par `SET_HDR_STATE` (24H2) ou `SET_ADVANCED_COLOR_STATE` ; luminosité par DDC/CI (`dxva2`, code VCP 0x10 ou celui choisi, ramené à la plage de l'écran) pour les écrans externes et WMI pour l'écran intégré ; autres codes VCP et chaîne de capacités MCCS (`vcp_caps`, `parse_caps`). Toute écriture passe par un verrou unique ; tout échange DDC/CI par le verrou du bus. |
| `src/main.rs` | Commandes, pipeline des actions (`act` : sérialisation, confirmation, journal, annulation), `save_with` (modifier et enregistrer les réglages), démarrage et raccourcis bureau. |
| `src/views.rs` | Fenêtres : principale, panneau de la zone de notification, confirmation, identification. |
| `src/tray.rs` | Icône et menu de la zone de notification, reconstruits une fois le calme revenu et seulement si leur contenu change (`changed`). |
| `src/hotkeys.rs` | Raccourcis clavier globaux, extinction des écrans, raccourcis de luminosité, bascule d'entrée (`toggle_input`). |
| `src/watch.rs` | Branchements, changements d'affichage et de thème ; profils automatiques ; alimentation et sortie de veille, transmises à `auto`. |
| `src/auto.rs` | Automatismes : luminosité selon l'heure (`Schedule`), profils à heure fixe, au changement d'alimentation et au lancement d'un programme (`Triggers`). |
| `src/dim.rs` | Luminosité logicielle : voiles sombres (fenêtres en couches, traversées par les clics) posés par le thread de surveillance sur les écrans sans luminosité matérielle. |
| `src/placement.rs` | Place des fenêtres d'un profil : relevé (`EnumWindows`, `GetWindowPlacement`), association aux fenêtres ouvertes (`pair`), remise en place. |
| `src/cli.rs` | Ligne de commande : `--profile`, `--brightness`, `--input`, `--off`, `--json`, dans l'application en marche ou seule. |
| `src/update.rs` | Vérification des mises à jour sur option : une requête WinHTTP à l'API des releases de GitHub. |
| `src/store.rs` | Profils et réglages (`config.json`, réparés à la lecture), préférences DDC/CI par écran (`Screen`, `Feature`) et du panneau (`Panel`, `OffMode`, `PowerSignal`), mode portable, raccourcis bureau, couleur d'accent. |
| `src/journal.rs` | Journal d'activité : une ligne JSON par événement (`Level`), écrite en un seul appel, fichier tournant à 512 Ko. Il nourrit la vue Activité et sert au diagnostic. |
| `ui/` | Interface : `index.html`, `app.js`, `style.css`, `i18n.js` (traduction anglaise), `common.js` (outils partagés : `el`, envoi `latest`, aimantation `snap`), `vcp.js` (noms MCCS des codes et valeurs), le panneau de la zone de notification (`flyout.html`, `flyout.js`), plus les fenêtres de confirmation et d'identification. |
| `tests/` | Tests des fonctions pures de l'interface (`node --test`). |
| `.github/workflows/ci.yml` | Intégration continue : Clippy, tests Rust, tests de l'interface. |
| `capabilities/` | Permissions par fenêtre : la fenêtre principale appelle les commandes de l'application, le panneau celles qu'il affiche, la confirmation une seule, l'identification aucune. |
| `windows/hooks.nsh` | Désinstallation : retire le lancement avec Windows et, sur demande, les données. |

### Profils

Un profil stocke la configuration CCD brute, sérialisée en hexadécimal. Les LUID d'adaptateur changent à chaque démarrage : on mémorise le chemin de chaque adaptateur pour les retrouver. Si un écran manque, le reste est appliqué ; si Windows refuse la configuration exacte, on lui laisse choisir les timings, puis la seule topologie.

### DDC/CI

Les capacités d'un écran (1 à 3 s de lecture) sont lues une fois depuis la vue DDC/CI et gardées dans ses préférences. Un code dont l'écran liste des valeurs est un réglage à choix, envoyé brut ; les autres sont continus et passent en % par leur plage. `ddc_set` n'écrit que les codes affichés dans le panneau, avec une valeur annoncée par l'écran (`Screen::allows`), et `Screen::checked` refuse les codes de commande (remise à zéro…). Les préférences sont appliquées au moteur par `retune()` : code et plage de luminosité, blocage. Les erreurs DDC/CI sont typées (`DdcError`) et ne deviennent du texte qu'à la frontière avec l'interface.

Certains écrans renvoient la réponse précédente pour un code qu'ils ne savent pas lire (BenQ EW3270U) : `read_codes` vérifie une réponse identique à la précédente en relisant le code après un code témoin.

L'écran intégré d'un portable passe par WMI, avec une connexion gardée dans un thread COM dédié.

Le panneau (`flyout`) est créé à chaque ouverture et détruit dès qu'il perd le focus ; sa page demande sa hauteur à `flyout_fit`, qui l'ancre en bas à droite de son écran.

### Profils et raccourcis bureau

Un profil garde le niveau réel de luminosité de chaque écran, pas la position du curseur : changer la plage d'un écran ensuite ne change pas le profil. Un raccourci bureau (`--profile`) est transmis à l'instance déjà lancée par le greffon d'instance unique : elle applique le profil avec ses réglages DDC/CI, son journal et l'annulation. Si MonitorMG ne tourne pas, le processus du raccourci applique lui-même le profil, préférences DDC/CI comprises, puis quitte.

### Concurrence

Le thread principal de Tauri gère l'icône, les menus, les fenêtres et les raccourcis globaux, et il prend lui-même le verrou des réglages. Règle : **ne jamais tenir `store`, `undo` ou `pending` pendant un appel à Tauri** (création d'un élément de menu, `available_monitors`, opérations sur une fenêtre ou l'icône) : depuis un autre thread, ces appels attendent le thread principal, qui peut attendre ce verrou. Copier d'abord ce qu'il faut, relâcher, puis appeler. `save_with` applique cette règle aux modifications des réglages ; les lints Clippy `significant_drop_in_scrutinee` et `significant_drop_tightening` (dans `Cargo.toml`) signalent un verrou gardé dans un `match` ou trop longtemps, mais pas dans une fermeture : la règle reste à appliquer à la relecture.

### Automatismes

Un seul thread (`auto::start`) se réveille à chaque minute et sur `auto::wake` (réglages modifiés, sortie de veille, changement d'alimentation). La luminosité prévue vient de `Schedule::target`, fonction pure : paliers, ou interpolation jusqu'à l'étape suivante, minuit compris. À une étape, tous les écrans la suivent ; entre deux étapes, en transition douce, un écran dont la luminosité s'écarte de la dernière valeur envoyée a été réglé à la main : il est laissé tranquille jusqu'à l'étape suivante. Un tic en retard d'une ou deux minutes rattrape les échéances sautées (`due`) ; après une veille rien n'est rattrapé, mais la luminosité prévue est réappliquée, 8 s après le réveil, le temps que DDC/CI réponde.

Les déclencheurs d'un profil passent par `apply_profile`, donc par `act()` : journal, annulation, pas de boîte de dialogue. Le lancement d'un programme est détecté en comparant la liste des processus (`CreateToolhelp32Snapshot`) toutes les 5 s, seulement si un profil a ce déclencheur ; un programme déjà ouvert ne déclenche rien, sa fermeture non plus.

La bascule d'entrée (`@input|<clé d'écran>` parmi les propriétaires de raccourcis) lit le code 0x60, puis écrit l'autre entrée de la paire. Son raccourci ne s'enregistre que par `assign_hotkey` : `set_screen` garde celui du fichier, quoi qu'envoie l'interface.

### Luminosité logicielle

`display::monitors()` donne à chaque écran actif sa luminosité matérielle (`LEVELS`) ou, à défaut, logicielle (`SOFT`, 100 % par défaut) avec `soft: true`. Le repli n'existe que dans l'application en marche (`enable_soft`) et après une première lecture matérielle : avant, tout écran paraîtrait muet. `set_brightness` note alors le niveau et demande au thread de surveillance (`WM_VEILS`) de mettre les voiles à jour ; `dim::sync` crée, déplace ou retire une fenêtre par écran d'après `display::veils()`. Un écran qui répond enfin en DDC/CI quitte `SOFT` à la relecture. Les profils n'enregistrent que la luminosité matérielle.

### Place des fenêtres

`store::capture` relève toujours les fenêtres d'application (visibles, titrées, sans propriétaire, ni outil ni masquées) ; elles ne sont remises en place que si « windows » n'est pas dans `exclude`, ce qui est le cas par défaut. Après `display::restore`, on attend `WINDOWS_SETTLE` que Windows ait fini de déplacer les fenêtres, puis `placement::restore` applique `SetWindowPlacement` ; une fenêtre agrandie est d'abord remise à sa place normale, pour s'agrandir sur le bon écran.

### Ligne de commande

`cli::parse` lit les arguments ; `main` traite `--json` et les erreurs dans le processus lancé, exécute la commande seule (`cli::alone`) si MonitorMG ne tourne pas, et laisse sinon le greffon d'instance unique la transmettre (`cli::run`). Les deux chemins partagent `brightness`, `input`, `power_down` et `switch_input`.

### Actions et annulation

Chaque modification d'affichage passe par `act()` : capture de l'état d'avant, application (sous confirmation si elle est risquée), entrée dans le journal avec sa source, mémorisation de l'état d'avant pour « Annuler ». La fenêtre de confirmation rétablit l'état d'origine si personne ne répond.

### Données

`%APPDATA%\MonitorMG` : `config.json`, `logs\activite.jsonl`. En édition portable (fichier `portable` à côté de l'exécutable), tout va dans `data\` à côté, y compris les données WebView2.

## Commandes

```
cargo test                     # 35 tests ; le test réel lit la config et les écrans et fait valider chaque opération par Windows, sans rien appliquer
cargo test -- --ignored        # test réel de la luminosité, écran par écran (modifie puis rétablit)
cargo clippy --all-targets     # aucun avertissement attendu, lints de verrous compris
cargo audit                    # vulnérabilités connues des dépendances
npm ci && npm run test:ui      # tests de l'interface (node --test)
node dev/serve.js ui 4517      # aperçu de l'interface dans un navigateur, avec un faux backend (dev/mock.js) ; ?lang=en pour l'anglais
node dev/captures.js           # captures d'écran du README (docs/screenshots/) depuis cet aperçu ; nécessite Playwright et les polices Segoe
node scripts/release.mjs       # installateur, édition portable et SHA256SUMS.txt dans dist/, avec la version de Tauri CLI figée par package-lock.json
```

## Publier une version

1. Mettre à jour `version` dans `Cargo.toml` et `tauri.conf.json`, et compléter `CHANGELOG.md`.
2. `cargo test`, `cargo clippy --all-targets`, `cargo audit`, `npm run test:ui` (l'intégration continue les lance aussi à chaque envoi).
3. `node scripts/release.mjs`, puis tester l'installateur sur une session propre : installation, lancement avec Windows, mise à jour par-dessus la version précédente, désinstallation.
4. Pousser le tag `vX.Y.Z` : le workflow `release` construit `dist/` sur un runner Windows, fait signer les exécutables si la signature est configurée ([SIGNATURE.md](SIGNATURE.md)), et crée la release en brouillon avec les trois fichiers.
5. Coller les notes du `CHANGELOG.md` dans le brouillon et publier. (Sans CI : `node scripts/release.mjs` et publier les trois fichiers de `dist/` à la main.)

### Signature

Voir [SIGNATURE.md](SIGNATURE.md) : options de signature (SignPath Foundation, Certum), intégration dans le workflow de release, et winget en complément. Le workflow `.github/workflows/release.yml` fait signer les exécutables par SignPath dès que le dépôt est configuré (secret `SIGNPATH_API_TOKEN`, variables `SIGNPATH_*`) ; sans configuration, il produit une release non signée.

## Conventions

- Interface en français ou en anglais selon la langue de Windows (`GetUserDefaultUILanguage`, forçable par la variable `MONITORMG_LANG=en` ou `fr`). Le texte français sert de clé : `t()` et le dictionnaire `EN` de `ui/i18n.js`, `store::t` et `tr!` côté Rust. Tout nouveau texte reçoit sa traduction.
- Phrases courtes, pas de tiret long, ponctuation française en français ; règles « craft » (pas de dégradé, de halo, de `transition: all`, espacements sur une base de 4 px).
- Tout nouvel appel exposé à l'interface doit être ajouté à `COMMANDS` dans `build.rs` et à la capacité de la fenêtre qui l'utilise.
- Une modification d'affichage passe par `act()` pour être journalisée et annulable.
- Les réglages se modifient par `save_with`, jamais en gardant le verrou pendant un appel à Tauri (voir « Concurrence »).
- Les valeurs à choix sont des `enum` (`OffMode`, `PowerSignal`, `Level`) ; une valeur inconnue dans le fichier prend la valeur par défaut au lieu de rendre tout le fichier illisible.
- Tauri CLI se met à jour volontairement : changer la version dans `package.json`, puis `npm install` pour régénérer `package-lock.json`.
