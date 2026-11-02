# Audit du code de MonitorMG 2.0.3

Audit réalisé le 2 novembre 2026, après la correction du bug « seul le premier écran répond » (2.0.3). Périmètre : tout le code livré, soit environ 8 000 lignes (Rust 3 600, JavaScript 2 100, CSS 2 000, HTML 280), plus la configuration Tauri, les permissions, l'installateur et le script de publication.

## Méthode

| Axe | Référentiel | Preuves |
| --- | --- | --- |
| Fonctionnement | comportement attendu, cas limites, données invalides | lecture intégrale du code, essais réels sur l'AORUS FO32U2 et le BenQ EW3270U |
| Concurrence | verrous, threads, thread principal de Tauri | lecture du code de Tauri 2.12.5 et de ses greffons, programme de vérification, lints Clippy dédiés |
| Performances | coût des opérations fréquentes, travail sur le thread principal | lecture du code, mesures des essais réels |
| Sécurité | guide de sécurité Tauri 2, moindre privilège, chaîne de publication, RustSec | `cargo audit`, revue des capacités, de la CSP et des entrées |
| Code | duplication, typage, découpage, tests | `cargo clippy`, `cargo test`, revue |

Gravité : **Critique** (peut figer l'application), **Élevée** (comportement faux dans un usage normal), **Moyenne** (risque réel, cas moins fréquent), **Faible** (finition, risque limité), **Info** (constat sans action urgente).

## Synthèse

Le moteur d'affichage reste solide et la surface d'attaque est petite : pas de réseau, CSP stricte, aucune insertion de HTML, capacités limitées fenêtre par fenêtre, aucune vulnérabilité connue dans les dépendances compilées pour Windows.

Le bug de la 2.0.3 n'était pas isolé, mais la « forêt » est surtout ailleurs : **la concurrence**. Deux endroits gardent le verrou des réglages pendant qu'ils attendent le thread principal de Tauri, alors que ce thread prend lui-même ce verrou (raccourcis globaux, fermeture du panneau, fermeture de la fenêtre). Le résultat possible est un gel complet de l'application, sans erreur ni trace dans le journal. C'est rare mais réel, et cela touche le cœur de l'usage quotidien.

Viennent ensuite un chemin oublié par la 2.0 (les raccourcis bureau ignorent les réglages DDC/CI, y compris le blocage d'un écran qui fige le PC), une configuration lue sans validation, et quelques pertes de données d'interface.

| Axe | État | Points prioritaires |
| --- | --- | --- |
| Concurrence | À corriger | C1, C2 |
| Fonctionnement | Bon, quatre défauts réels | F1, F2, F3, F4 |
| Performances | Correct, travail inutile sur le thread principal | P1, P2 |
| Sécurité | Bon, chaîne de publication à verrouiller | S1 |
| Code | Lisible, duplications et typage par chaînes | M1, M3 |

## Tableau des constats

| Id | Gravité | Constat | Où |
| --- | --- | --- | --- |
| C1 | Critique | Verrou des réglages tenu pendant la création du menu de notification | `src/main.rs:1244` |
| C2 | Élevée | Verrou des réglages tenu pendant une requête au thread principal à l'ouverture de la fenêtre | `src/main.rs:912` |
| F1 | Élevée | Les raccourcis bureau (`--profile`) ignorent les réglages DDC/CI et le blocage | `src/main.rs:1403` |
| F2 | Moyenne | `config.json` lu sans validation : une plage invalide fait paniquer la lecture de luminosité, qui ne reprend plus | `src/store.rs:244`, `src/display.rs:847` |
| F3 | Moyenne | Un raccourci refusé efface le raccourci précédent en mémoire | `src/main.rs:1097` |
| F4 | Moyenne | Un échec en cours de changement de mode laisse des réglages en attente dans le registre | `src/display.rs:781` |
| F5 | Moyenne | Vue DDC/CI : enregistrement à partir d'un état périmé, saisie effacée par un rafraîchissement | `ui/app.js:623`, `ui/app.js:589` |
| C3 | Moyenne | « Remplacer par l'affichage actuel » garde le verrou pendant des lectures DDC/CI | `src/main.rs:443` |
| P1 | Moyenne | Le menu de notification est reconstruit en entier après presque chaque événement | `src/main.rs:764` |
| P2 | Moyenne | Portables : une connexion WMI complète à chaque pas du curseur | `src/display.rs:994` |
| S1 | Moyenne | Le script de publication télécharge une version non figée de l'outil de build | `scripts/release.mjs:12` |
| M1 | Moyenne | Erreurs en chaînes de caractères, décisions prises en comparant des messages | `src/display.rs:1119` |
| F6 | Faible | Bouton marche/arrêt : un état illisible est toujours traité comme « allumé » | `src/main.rs:317` |
| F7 | Faible | Écrans qui renvoient la réponse précédente : valeurs fausses dans le panneau | `src/display.rs:1199` |
| F8 | Faible | Extinction DDC/CI envoyée à tous les écrans, avec des valeurs qu'ils n'annoncent pas | `src/display.rs:1225` |
| F9 | Faible | Écrans de même modèle impossibles à distinguer dans le panneau et la vue DDC/CI | `ui/flyout.js`, `ui/app.js` |
| F10 | Faible | Journal : une ligne écrite en deux fois, mélange possible entre threads | `src/journal.rs:49` |
| F11 | Faible | Boucle de relecture de la configuration sans limite | `src/display.rs:270` |
| F12 | Faible | « Remplacer par l'affichage actuel » écrase un profil sans confirmation | `ui/app.js:848` |
| P3 | Faible | Chaque échange DDC/CI réénumère tous les écrans | `src/display.rs:928` |
| P4 | Faible | Un thread par message système pendant les rafales de branchement | `src/main.rs:1356` |
| S2 | Faible | Fenêtre principale : `core:default` plus large que nécessaire | `capabilities/main.json:6` |
| S3 | Faible | Valeurs DDC/CI à choix non vérifiées côté backend | `src/main.rs:303` |
| M2 | Faible | Valeurs codées en chaînes et en nombres magiques | `src/store.rs`, `src/main.rs:353` |
| M3 | Faible | Règles dupliquées entre Rust et JavaScript | `ui/app.js:555`, `ui/app.js:579` |
| M4 | Faible | Code dupliqué entre `app.js` et `flyout.js` | `ui/app.js:12`, `ui/flyout.js:13` |
| M5 | Faible | `main.rs` mêle commandes, menu, fenêtres, raccourcis et surveillance | `src/main.rs` |
| M6 | Faible | Pas de test JavaScript, pas d'intégration continue | dépôt |
| F13 | Info | La luminosité d'un profil dépend de la plage réglée au moment de l'enregistrement | `src/display.rs:1256` |
| S4 | Info | Serveur d'aperçu : contrôle de chemin par préfixe | `dev/serve.js` |

## Concurrence

### C1. Verrou des réglages tenu pendant la création du menu (Critique)

**Constat.** Dans `tray_menu`, l'élément « Éteindre les écrans » est construit ainsi :

```rust
let off = match store(app).off_hotkey.clone() {
    Some(h) => off().accelerator(&h).build(app)...,
    None => off().build(app)?,
};
```

En Rust, le verrou renvoyé par `store(app)` est un temporaire de l'expression du `match` : il reste tenu jusqu'à la fin du `match`, donc pendant `build(app)`. Or, appelé hors du thread principal, `MenuItem::new` envoie la création au thread principal et attend sa réponse (`run_main_thread!`, Tauri 2.12.5, `src/menu/normal.rs` et `src/lib.rs:1112`).

**Impact.** `changed()`, qui reconstruit ce menu, est appelé depuis des threads secondaires : relecture de la luminosité, actions de la fenêtre, branchements, raccourcis de luminosité. Pendant ce temps, le thread principal prend le même verrou dans trois cas courants :
- un raccourci clavier global (le gestionnaire tourne sur le thread principal, `on_hotkey`) ;
- la fermeture du panneau (`WindowEvent::Destroyed` appelle `changed()`, `main.rs:1007`) ;
- la fermeture de la fenêtre principale (`save_place`, `main.rs:938`).

Si les deux se croisent, chacun attend l'autre : l'icône, le menu, les fenêtres et les raccourcis ne répondent plus, sans message ni entrée de journal. Il faut tuer le processus.

**Preuve.** Programme de vérification (édition 2021) : dans un `match` et dans un `.filter(|..| ..)`, le verrou est encore tenu dans le bras ou la fermeture ; copié dans une variable d'abord, il ne l'est plus. Le lint Clippy `significant_drop_in_scrutinee` signale aussi cette ligne.

**Action.** Copier la valeur avant le `match` (`let off_key = store(app).off_hotkey.clone();`). Adopter une règle écrite : ne jamais tenir `store`, `undo` ou `pending` pendant un appel à Tauri (fenêtre, menu, icône, écrans). Activer `clippy::significant_drop_in_scrutinee` et `clippy::significant_drop_tightening` dans `Cargo.toml` (`[lints.clippy]`).

### C2. Verrou tenu pendant `available_monitors` à l'ouverture de la fenêtre (Élevée)

**Constat.** `let place = store(&app).window.filter(|p| on_screen(&app, p));` garde le verrou pendant `on_screen`, qui appelle `app.available_monitors()`. Hors du thread principal, cet appel attend lui aussi le thread principal (`event_loop_window_getter!`, tauri-runtime-wry 2.12.3).

**Impact.** Même gel que C1 si un raccourci global, la fermeture du panneau ou une autre fermeture tombe pendant l'ouverture de la fenêtre. Clippy ne le détecte pas (fermeture dans `filter`).

**Action.** `let saved = store(&app).window;` puis `saved.filter(|p| on_screen(&app, p))`.

### C3. Lectures DDC/CI sous verrou dans « Remplacer par l'affichage actuel » (Moyenne)

**Constat.** `profile_update` prend le verrou puis appelle `store::capture`, qui relit la luminosité de tous les écrans en DDC/CI. Cette relecture attend le bus si une lecture de capacités est en cours (jusqu'à 3,7 s mesurées sur le BenQ). `store::capture` appelle aussi `display::read_brightness` directement, sans le garde-fou `READING` utilisé ailleurs, d'où deux relectures complètes possibles en même temps.

**Impact.** Pendant ce temps, tout ce qui demande le verrou sur le thread principal est figé (raccourcis, menu). Pas de blocage définitif, mais un gel visible de plusieurs secondes dans le pire cas.

**Action.** Capturer avant de prendre le verrou, comme le fait déjà `profile_save`. Passer par une fonction de relecture unique qui respecte `READING`.

## Fonctionnement

### F1. Les raccourcis bureau ignorent les réglages DDC/CI (Élevée)

**Constat.** `MonitorMG.exe --profile <id>` lance un second processus qui applique le profil puis quitte (`apply_from_desktop`). Ce processus charge la configuration mais n'appelle jamais `display::tune` : le moteur y garde ses valeurs par défaut.

**Impact.**
- Un écran **bloqué** dans la vue DDC/CI est interrogé quand même, alors que le blocage sert précisément aux écrans qui figent le PC.
- La plage de luminosité et le code VCP choisis sont ignorés : la luminosité appliquée n'est pas celle du profil.
- Ce processus et l'instance principale parlent au bus DDC/CI en même temps, sans verrou commun.

**Action.** Appeler `display::tune` avec les préférences chargées avant `restore`. Mieux : quand l'instance principale tourne, lui transmettre la demande par le greffon d'instance unique (il reçoit déjà les arguments) pour qu'elle applique le profil elle-même, avec journal, annulation et verrou du bus.

### F2. Configuration lue sans validation (Moyenne)

**Constat.** `Store::load` accepte tout `config.json` bien formé. Les contrôles de `Screen::checked` ne s'appliquent qu'aux réglages venus de l'interface.

**Impact.**
- Une plage inversée (par exemple `[80, 20]`, après une édition à la main ou une évolution future du format) fait paniquer `Range::to_slider` : `u32::clamp` panique si le minimum dépasse le maximum. La panique survient dans le thread de relecture, avant que `READING` ne repasse à faux (`main.rs:789`) : la luminosité n'est plus jamais relue jusqu'au redémarrage. Dans `ddc_values`, le panneau reste sur « … ».
- `confirm.secs` à 0 fait revenir l'affichage précédent immédiatement après chaque changement risqué.

**Action.** Valider à la lecture : passer chaque `Screen` par `checked` (en écartant ce qui est invalide), borner `confirm.secs`, `panel.step` et `panel.power`. Rendre `to_slider` sans panique (`min`/`max` au lieu de `clamp`). Remettre `READING` à faux même en cas de panique (garde avec `Drop`).

### F3. Un raccourci refusé efface le précédent (Moyenne)

**Constat.** `assign_hotkey` écrit le nouveau raccourci, tente de l'enregistrer auprès de Windows et, en cas de refus, met `None` à la place, pas l'ancienne valeur. Le fichier n'est pas réécrit.

**Impact.** Après « Cette combinaison est déjà prise par une autre application », le profil n'a plus de raccourci en mémoire et l'ancien ne fonctionne plus, alors que le fichier le garde : il revient au redémarrage, ou disparaît pour de bon au prochain enregistrement d'un autre réglage.

**Action.** Garder l'ancienne valeur et la remettre en cas d'échec.

### F4. Changement de mode interrompu à mi-chemin (Moyenne)

**Constat.** `change()` prépare chaque écran avec `CDS_UPDATEREGISTRY | CDS_NORESET`, puis applique tout d'un coup. Si le déplacement d'un écran voisin échoue (`?` ligne 781), la fonction s'arrête sans appliquer ni annuler ce qui a déjà été préparé.

**Impact.** Des réglages restent en attente dans le registre et s'appliquent au prochain changement d'affichage, d'où qu'il vienne : un mode inattendu peut apparaître plus tard, sans lien apparent.

**Action.** En cas d'échec après la première préparation, restaurer les modes de départ (déjà lus) avec `CDS_UPDATEREGISTRY | CDS_NORESET`, puis appliquer.

### F5. Vue DDC/CI : enregistrement périmé et saisie effacée (Moyenne)

**Constat.**
- Chaque contrôle enregistre `{ ...sc, ...patch }`, où `sc` est l'état au moment du dernier affichage. Deux modifications rapprochées (deux interrupteurs en moins de 200 ms environ) : la seconde renvoie l'état d'avant la première, qui est perdue.
- `render()` reconstruit toute la vue à chaque événement `changed`. Une relecture de luminosité qui détecte un écart (par exemple quand Twinkle Tray règle aussi les écrans) reconstruit les champs : un nom ou une plage en cours de saisie est effacé avant d'être enregistré.

**Action.** Envoyer au backend des modifications ciblées (« activer le code 0x12 », « plage = … ») plutôt que l'objet entier, ou relire l'état juste avant d'enregistrer. Ne pas reconstruire la vue quand un champ de saisie a le focus.

### F6. Bouton marche/arrêt sur un état illisible (Faible)

`ddc_power` traite une lecture ratée du code D6 comme « allumé » et envoie donc l'extinction. Un écran déjà en veille qui ne répond plus ne peut pas être rallumé par ce bouton. Le panneau affiche ensuite 4 (veille) quelle que soit la valeur réellement envoyée. **Action :** proposer explicitement « Allumer » quand l'état est inconnu, et afficher la valeur envoyée.

### F7. Réponses recopiées par certains écrans (Faible)

Le BenQ EW3270U renvoie la réponse précédente pour un code qu'il ne sait pas lire (constaté pour 0xD8, 0xE2 à 0xF9). Lus à la suite, ces codes affichent dans le panneau la valeur d'un autre réglage. **Action :** quand une réponse est identique à la précédente pour un code différent, relire un code de référence entre les deux pour confirmer, sinon afficher « Sans réponse ».

### F8. Extinction DDC/CI envoyée à tous les écrans (Faible)

`power_off_all` envoie le code D6 à tous les écrans externes, même à ceux qui ne l'annoncent pas, et envoie 4 (veille) au BenQ qui n'accepte que 1 et 5. Sans danger connu, mais c'est une écriture non demandée. **Action :** ne l'envoyer qu'aux écrans qui annoncent D6, avec les seules valeurs annoncées.

### F9. Écrans identiques indiscernables (Faible)

Deux écrans du même modèle (cas du dock HP G4) portent le même nom dans le panneau et la vue DDC/CI. Le nom personnalisé règle le problème, mais seulement après coup. **Action :** suffixer automatiquement les doublons par leur numéro d'écran, comme Windows (« 1 | HP E24 »).

### F10. Journal écrit en deux fois (Faible)

`writeln!` écrit la ligne puis le saut de ligne : deux écritures. Deux threads (ou le processus `--profile`) peuvent mélanger leurs lignes ; la vue Activité ignore alors les lignes abîmées. **Action :** une seule écriture `write_all(format!("{line}\n"))`.

### F11. Boucle sans limite (Faible)

`query()` recommence tant que Windows répond « tampon trop petit ». Sous une rafale de changements, la boucle n'a pas de plafond. **Action :** limiter à quelques essais puis renvoyer l'erreur.

### F12. Remplacement de profil sans confirmation (Faible)

« Remplacer par l'affichage actuel » écrase le profil en un clic, sans annulation possible. **Action :** demander confirmation, comme pour la suppression.

### F13. Luminosité des profils et plage (Info)

Un profil enregistre la valeur du curseur, relative à la plage de l'écran à ce moment. Si la plage change ensuite, le même profil donne une autre luminosité réelle. À documenter, ou enregistrer le niveau réel de l'écran.

## Performances

### P1. Reconstruction complète du menu de notification (Moyenne)

`changed()` reconstruit tout le menu : environ 40 éléments par écran actif (20 résolutions, les fréquences, 12 paliers de luminosité, les commandes). Depuis un thread secondaire, chaque élément demande un aller-retour au thread principal. Ce travail est déclenché après chaque action, chaque relecture de luminosité qui détecte un écart, chaque fermeture du panneau et chaque appui sur un raccourci de luminosité. **Action :** regrouper les appels rapprochés (une reconstruction au plus toutes les 300 ms) et ne reconstruire que si ce qui est affiché a changé (empreinte des données du menu).

### P2. WMI à chaque pas du curseur sur un portable (Moyenne)

Pour l'écran intégré, chaque écriture crée un thread, initialise COM et ouvre une connexion WMI, puis la referme. Pendant un glissement, cela se répète à chaque valeur. **Action :** un thread COM dédié et une connexion WMI gardée tant que l'application tourne. À mesurer sur un portable avant et après.

### P3. Réénumération à chaque échange DDC/CI (Faible)

Chaque lecture ou écriture relance `QueryDisplayConfig`, `EnumDisplayMonitors`, les noms de périphériques, et ouvre les poignées physiques de tous les écrans. Négligeable avec deux ou trois écrans ; à mettre en cache par topologie si le nombre d'écrans grandit.

### P4. Un thread par message système (Faible)

La fenêtre de surveillance crée un thread pour chaque message pendant les rafales (`WM_DEVICECHANGE` au branchement d'un dock). Ils se terminent vite, mais un seul minuteur suffirait.

## Sécurité

### S1. Outil de build non figé (Moyenne)

`scripts/release.mjs` exécute `npx --yes @tauri-apps/cli@2 build` : la dernière version 2.x publiée sur npm est téléchargée et exécutée à chaque publication. Une version compromise ou boguée de l'outil produirait l'installateur signé de MonitorMG, et deux publications du même code peuvent donner des binaires différents. **Action :** figer la version exacte (`package.json` + `package-lock.json`, ou `@tauri-apps/cli@2.x.y`) et la mettre à jour volontairement.

### S2. Permissions de la fenêtre principale (Faible)

`core:default` accorde à la fenêtre principale l'ensemble des permissions de base de Tauri (fenêtres, menus, icône, chemins, ressources…), alors qu'elle n'utilise que les événements et ses propres commandes. Le panneau, lui, est déjà au strict nécessaire. **Action :** remplacer par `core:event:default` et les quelques permissions `core:window` utilisées.

### S3. Valeurs DDC/CI à choix non vérifiées (Faible)

`ddc_set` vérifie que le code est affiché dans le panneau, mais pas que la valeur fait partie de celles annoncées par l'écran. L'interface n'envoie que des valeurs annoncées ; c'est une défense en profondeur si la page était un jour compromise. **Action :** refuser une valeur absente de la liste annoncée pour un réglage à choix.

### S4. Serveur d'aperçu (Info)

`dev/serve.js` vérifie le chemin avec `startsWith(root)` sans séparateur. Outil de développement, lié à 127.0.0.1 : sans conséquence, à corriger par principe (`root + path.sep`).

### Ce qui est bien en place

- CSP stricte (`script-src 'self'`, pas de script en ligne), aucun `innerHTML` ni `eval` : tout texte venant d'un écran ou d'un profil passe par `textContent`.
- Capacités par fenêtre ; la fenêtre d'identification n'a aucune permission.
- Les scripts d'initialisation injectés (`START_VIEW`, `ID`) n'acceptent qu'une liste blanche ou du JSON sérialisé.
- Les codes VCP de commande (remise à zéro…) sont refusés côté backend.
- `cargo audit` : aucune vulnérabilité ; les six avertissements (glib, zbus, proc-macro-error…) concernent des dépendances Linux qui ne sont pas compilées pour Windows (vérifié avec `cargo tree --target x86_64-pc-windows-msvc`).
- Aucune connexion réseau, aucun droit administrateur.

## Code et maintenabilité

### M1. Erreurs en chaînes de caractères (Moyenne)

`Res<T> = Result<T, String>` partout. `set_brightness` décide de son message en comparant le texte d'erreur (`e == refused()`) : une traduction ou une reformulation change le comportement. **Action :** un `enum` d'erreur (`NoDdc`, `Refused`, `Blocked`, `Windows(String)`…) converti en texte seulement à la frontière avec l'interface.

### M2. Chaînes et nombres magiques (Faible)

`panel.off` vaut `"soft"`, `"hard"` ou `"both"` ; `panel.power` 4, 5 ou 6 ; les niveaux du journal `"ok"`, `"warn"`… Ils sont vérifiés à la main (`set_panel`). **Action :** des `enum` avec `serde`, qui rendent les valeurs invalides impossibles à la lecture du fichier.

### M3. Règles dupliquées entre Rust et JavaScript (Faible)

Le choix automatique du code de luminosité (`autoBrightness` et `Screen::brightness_code`), les valeurs par défaut d'un écran (`DEFAULT_SCREEN` et `Screen::default`) et la règle « à choix ou continu » existent en double. Une évolution d'un seul côté créerait un écart silencieux. **Action :** envoyer dans `state` les valeurs calculées par le backend (code retenu, type de chaque réglage).

### M4. Code dupliqué dans l'interface (Faible)

`el()`, `$`, `NNBSP` et la file d'envoi « une requête à la fois, la dernière valeur gagne » sont écrits deux fois (`app.js` et `flyout.js`). **Action :** un petit fichier partagé, comme `vcp.js`.

### M5. Organisation de `main.rs` (Faible)

1 574 lignes qui mêlent commandes, logique d'action, fenêtres, menu, raccourcis et surveillance des écrans. Le schéma « verrou, modification, enregistrement, `drop`, `changed` » est répété treize fois, et le libellé d'un changement de mode trois fois. **Action :** découper en modules (`tray.rs`, `hotkeys.rs`, `windows.rs`) et une fonction `edit_store(app, |s| …)` qui enregistre et notifie.

### M6. Tests (Faible)

12 tests unitaires, un test réel en lecture (lancé avec `cargo test`) et un test réel d'écriture (manuel, `--ignored`) couvrent le moteur et les analyseurs. Rien ne couvre le JavaScript (aimantation de la disposition, plages, traductions) ni les règles de concurrence, et aucune intégration continue ne lance `cargo test` et `cargo clippy` à chaque envoi. **Action :** un workflow GitHub Actions (Windows) avec `cargo test`, `cargo clippy` et les lints de C1, plus quelques tests `node --test` pour les fonctions pures de l'interface.

## Plan d'action proposé

| Lot | Contenu | Effort |
| --- | --- | --- |
| 1, urgent | C1, C2, F1 | court, quelques lignes chacun |
| 2, fiabilité | F2, F3, F4, F5, C3, F10, F11 | moyen |
| 3, performances et sécurité | P1, P2, S1, S2, S3 | moyen |
| 4, qualité | M1 à M6, F6 à F9, F12 | au fil des versions |

Le lot 1 mérite une version 2.0.4 dès que possible : C1 et C2 peuvent figer l'application sans laisser de trace, et F1 contourne une protection que l'utilisateur a activée exprès.

## Corrections (version 2.1.0)

Tous les constats ont été traités. Ce qui a été fait, et comment c'est vérifié :

| Id | Correction | Vérification |
| --- | --- | --- |
| C1, C2 | Valeurs copiées avant tout appel à Tauri ; le menu lit ses données (`MenuData`) puis se construit sans verrou ; règle écrite dans `docs/DEVELOPPEMENT.md` | programme de vérification, lints Clippy activés dans `Cargo.toml` (aucun avertissement) |
| C3 | Capture hors verrou dans `profile_update` ; une seule relecture complète à la fois (`READ`) | revue |
| F1 | `--profile` transmis à l'instance lancée par le greffon d'instance unique ; sinon `display::tune` avant d'appliquer | essai réel sur la copie de test |
| F2 | `Store::sanitized` à la lecture, `Range` sans panique, drapeau de relecture remis par une garde | tests `bad_values_in_the_file_are_repaired`, `invalid_ranges_never_panic` |
| F3 | L'ancien raccourci reprend sa place | revue |
| F4 | Modes d'origine remis si une étape échoue | revue (échec non reproductible sans matériel défaillant) |
| F5 | Enregistrements en file à partir de l'état renvoyé par le backend ; vue non reconstruite pendant une saisie | aperçu : deux activations rapprochées gardées, nom conservé pendant un rafraîchissement |
| F6 | `ddc_power` reçoit l'état voulu ; un état illisible propose d'allumer | aperçu |
| F7 | `read_codes` : réponse identique vérifiée après un code témoin | test `echoed_replies_are_dropped`, BenQ réel : 0xE2 « sans réponse » au lieu d'une valeur recopiée |
| F8 | `power_targets` : écrans qui annoncent D6, valeurs annoncées | test `only_announced_values_are_allowed` |
| F9 | Numéro ajouté aux noms en double | test `twin_screens_get_their_number` |
| F10 | Une seule écriture par ligne | revue |
| F11 | Cinq essais au plus | revue |
| F12 | Confirmation avant remplacement | aperçu |
| F13 | Les profils gardent le niveau réel | revue |
| P1 | Menu reconstruit une fois par rafale (`debounce`), seulement si son empreinte change | revue |
| P2 | Thread COM unique, connexion WMI gardée et rouverte après un échec | test réel (chemin exercé, pas de portable pour mesurer) |
| P3 | `on_screen` n'ouvre que les poignées du moniteur Windows visé | tests réels sur les deux écrans |
| P4 | Un seul thread suit les rafales | revue |
| S1 | `package.json` + `package-lock.json` (Tauri CLI 2.12.4), `npm ci` dans le script | `npx --no-install tauri --version` |
| S2 | `core:default` remplacé par `core:event:default` | essai réel de la fenêtre |
| S3 | `Screen::allows` vérifie les valeurs à choix et d'alimentation | test `only_announced_values_are_allowed` |
| S4 | Contrôle de chemin avec séparateur | revue |
| M1 | `DdcError` ; plus aucune décision sur un texte | revue |
| M2 | `OffMode`, `PowerSignal`, `Level`, lecture indulgente des valeurs inconnues | test `power_signal_keeps_its_file_format` |
| M3 | `state` renvoie les préférences de chaque écran et le code de luminosité retenu (`auto_vcp`) ; plus de copie des règles en JavaScript | aperçu |
| M4 | `ui/common.js` | tests de l'interface |
| M5 | Modules `views`, `tray`, `hotkeys`, `watch` ; `save_with`, `mode_label` | compilation, tests |
| M6 | `tests/ui.test.mjs`, `.github/workflows/ci.yml` (actions figées par empreinte) | `npm run test:ui` |

Reste ouvert, par nature : la signature du code (décision du propriétaire) et la mesure de P2 sur un portable.
