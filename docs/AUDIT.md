# Audit de MonitorMG avant publication

Audit réalisé le 27 octobre 2026 sur la version 0.1.0 (environ 4 500 lignes : Rust, HTML, CSS, JavaScript).

## Méthode

Un audit compare le logiciel à un référentiel, relève les écarts avec des preuves, les classe par gravité et propose une action pour chacun. Le référentiel retenu couvre les cinq axes demandés, plus le code et la documentation :

| Axe | Référentiel | Outils et preuves |
| --- | --- | --- |
| Interface | WCAG 2.1 AA, règles « craft » de finition, conventions Windows 11 | axe-core sur l'interface rendue, revue écran par écran |
| Fiabilité et suivi | gestion d'erreurs, concurrence, diagnostic, retour d'information | lecture intégrale du code, essais réels (activation, retour automatique) |
| Performances | mémoire au repos, temps d'ouverture, latence | mesures sur l'exécutable release |
| Sécurité | guide de sécurité Tauri 2 (CSP, capacités, moindre privilège), avis RustSec | `cargo audit`, revue des permissions et des entrées |
| Publication | installation, désinstallation, signature, version portable, documentation | revue de la configuration NSIS et des métadonnées |
| Code | lints Rust, cohérence, tests | `cargo clippy` (normal et strict), `cargo test` |

Gravité : **Élevée** (bloque une publication sereine), **Moyenne** (visible par l'utilisateur ou risque réel), **Faible** (finition, risque limité), **Info** (constat sans action).

## Synthèse

Le cœur est solide. Le moteur d'affichage gère les cas difficiles (changement de LUID au redémarrage, écrans absents, trois niveaux de repli), l'application est très légère au repos, sans aucune vulnérabilité connue ni aucun avertissement de lint. Les manques sont ceux d'un prototype : pas de journal, pas de trace des actions faites hors de la fenêtre, pas d'annulation, des réglages rangés au mauvais endroit, pas de version portable, un exécutable sans métadonnées et non signé.

| Axe | État | Points bloquants |
| --- | --- | --- |
| Interface | Bon, 0 violation WCAG détectée | aucun |
| Fiabilité et suivi | Insuffisant pour le public | F1, F2 |
| Performances | Très bon | aucun |
| Sécurité | Bon, durcissement possible | aucun |
| Publication | Incomplet | P1 (décision du propriétaire), P2 |

## Mesures (version 0.1.0, release)

| Mesure | Valeur |
| --- | --- |
| Exécutable / installateur | 7,3 Mo / 1,7 Mo |
| Au repos, zone de notification seule | 1 processus, 2,6 Mo privés, 13,9 Mo en mémoire de travail |
| Fenêtre ouverte | 7 processus (WebView2), 215 Mo privés |
| Après fermeture de la fenêtre | 1 processus, 5,8 Mo privés : la webview est bien libérée |
| Ouverture de la fenêtre (page chargée) | 0,48 s à chaud, 0,51 s au premier affichage |
| Commande `state` (10 appels) | médiane 62 ms, maximum 92 ms |
| `cargo audit` | 0 vulnérabilité ; 5 avertissements sur des dépendances Linux (gtk, zbus) non compilées pour Windows |
| `cargo clippy` | 0 avertissement ; en mode strict, uniquement du style autour des appels Windows |
| axe-core (WCAG 2.1 A et AA) | 38 règles passées, 0 violation |

## Constats et actions

### Publication

| N° | Gravité | Constat | Action |
| --- | --- | --- | --- |
| P1 | Élevée | Exécutable et installateur non signés : Windows SmartScreen affiche « Éditeur inconnu » et demande deux clics pour lancer. C'est la première impression du public. | **Décision du propriétaire.** Certificat OV (150 à 300 $ par an) ou Azure Artifact Signing (environ 10 $ par mois, réservé aux particuliers des États-Unis et du Canada). La signature s'ajoute ensuite par `bundle.windows.signCommand`, sans toucher au code. |
| P2 | Élevée | Pas de version portable. | Archive portable : un fichier `portable` à côté de l'exécutable garde profils, journaux et données WebView2 dans `data\`. |
| P3 | Moyenne | Version 0.1.0, aucun éditeur, copyright ni description : les propriétés du fichier et « Applications installées » sont vides. | Version 1.0.0, métadonnées complètes. |
| P4 | Moyenne | La désinstallation laisse l'entrée de démarrage automatique : Windows tente ensuite de lancer un fichier disparu. | Script NSIS de désinstallation qui retire l'entrée et, sur demande, les données. |
| P5 | Moyenne | Installateur en français seulement ; WebView2 téléchargé pendant l'installation, donc échec hors ligne sur un Windows 10 sans WebView2. | Installateur français et anglais, programme d'installation WebView2 embarqué (+1,8 Mo). |
| P6 | Moyenne | Ni journal des modifications, ni déclaration de confidentialité, ni licence. | CHANGELOG et confidentialité ajoutés. **La licence est une décision du propriétaire** (libre comme MIT, ou propriétaire). |
| P7 | Faible | Pas de dépôt git ni `.gitignore` ; dossier généré `gen/` au milieu des sources. | `.gitignore` ajouté ; créer le dépôt avant la première publication. |
| P8 | Faible | Interface uniquement en français. | Recommandé pour une diffusion internationale : traduction anglaise (environ 200 chaînes). |
| P9 | Faible | Pas de mise à jour automatique. | Recommandé une fois l'hébergement choisi : `tauri-plugin-updater` avec mises à jour signées. |

### Fiabilité et suivi des tâches

| N° | Gravité | Constat | Action |
| --- | --- | --- | --- |
| F1 | Élevée | Aucun journal : impossible de diagnostiquer un problème chez un utilisateur. Les erreurs internes (panic) disparaissent sans trace. | Journal tournant (512 Ko) dans le dossier de données, capture des erreurs internes, informations système au démarrage. |
| F2 | Élevée | Les modifications d'affichage ne sont pas sérialisées : fenêtre, menu de notification, raccourci clavier et branchement automatique peuvent appliquer deux configurations en même temps. | Verrou global autour de toute écriture d'affichage. |
| F3 | Moyenne | 17 `lock().unwrap()` : une erreur interne sous verrou empoisonne le verrou et fait échouer toutes les commandes suivantes. | Verrous tolérants. |
| F4 | Moyenne | En release, un échec au démarrage ou l'absence de WebView2 ne produit rien : l'icône ne s'ouvre pas, sans explication. | Message clair et lien de téléchargement de WebView2, entrée dans le journal. |
| F5 | Moyenne | Les actions faites hors de la fenêtre (menu, raccourci, branchement, raccourci bureau) ne laissent aucune trace ; les notifications disparaissent en 2,6 s. | Vue **Activité** : historique horodaté de chaque action, avec sa source et son résultat. Infobulle de l'icône avec la dernière action. |
| F6 | Moyenne | Aucune annulation après coup, hors de la fenêtre de confirmation. | **Annuler** le dernier changement depuis la notification, la vue Activité, le menu de notification et `Ctrl+Z`. |
| F7 | Moyenne | Pendant une opération (jusqu'à 5 s pour activer un écran), seule une barre de 2 px l'indique et les commandes restent actives : risque de double action. | Statut « en cours » explicite, commandes suspendues pendant l'opération. |
| F8 | Faible | Les erreurs des actions automatiques s'affichent dans une boîte modale bloquante. | Actions automatiques consignées dans Activité ; la boîte modale reste réservée aux actions explicites hors fenêtre. |
| F9 | Faible | Un fichier de configuration abîmé est mis de côté sous un nom fixe, écrasé à la corruption suivante. | Copie horodatée. |
| F10 | Faible | Tests limités au moteur (3 tests). | Tests unitaires de la logique pure (configuration, noms de fichiers, nouveaux réglages). |

### Performances

| N° | Gravité | Constat | Action |
| --- | --- | --- | --- |
| R1 | Moyenne | Chaque rafraîchissement de l'interface relance une lecture DDC/CI de tous les écrans : du trafic sur le bus de l'écran à chaque clic, ce qui provoque sur certains modèles un affichage parasite ou des ralentissements. | Lecture de la luminosité limitée à une toutes les 5 s, sauf au branchement d'un écran. |
| R2 | Faible | `WM_SETTINGCHANGE`, très fréquent, reconstruit le menu et relit toute la configuration. | Ne réagir qu'aux changements de thème et de couleur d'accent. |
| R3 | Info | Mémoire au repos (2,6 Mo), libération de WebView2 à la fermeture, ouverture en 0,5 s : conformes à l'objectif d'une application de zone de notification. | Aucune. |

### Sécurité

| N° | Gravité | Constat | Action |
| --- | --- | --- | --- |
| S1 | Moyenne | Toutes les fenêtres (confirmation, identification) peuvent appeler toutes les commandes, alors qu'elles n'en utilisent qu'une ou aucune. | Moindre privilège : manifeste des commandes et permissions par fenêtre. |
| S2 | Faible | La politique de contenu (CSP) autorise les styles en ligne, inutiles ici, et ne bloque pas `object` ni `base`. | CSP durcie. |
| S3 | Faible | Le nom du raccourci bureau vient du nom du profil : un raccourci existant du même nom est écrasé sans prévenir. | Nom normalisé et suffixé : `Bureau (MonitorMG).lnk`. |
| S4 | Info | Aucun accès réseau, aucune télémétrie, aucune élévation de privilèges, outils de développement désactivés en release, aucun contenu distant chargé. | À mettre en avant dans la déclaration de confidentialité. |
| S5 | Faible | Version portable lancée depuis un dossier partagé ou de téléchargement : Windows charge certaines DLL (`dxva2.dll`) depuis le dossier de l'exécutable en priorité. | Risque résiduel documenté : lancer la version portable depuis un dossier personnel. |

### Interface

| N° | Gravité | Constat | Action |
| --- | --- | --- | --- |
| U1 | Moyenne | Les réglages sont entassés sous la liste des profils ; ni page Paramètres, ni « À propos » (version, dossier des données, raccourcis, confidentialité). | Navigation à la Windows 11 : **Disposition**, **Activité**, **Paramètres**. |
| U2 | Moyenne | La disposition ne se règle qu'à la souris (critère WCAG 2.1.1). | Flèches du clavier pour déplacer l'écran sélectionné, avec aimantation. |
| U3 | Faible | Le mode Contraste élevé de Windows n'est pas pris en charge. | Styles `forced-colors`. |
| U4 | Faible | Notifications trop brèves, non suspendues au survol (critère WCAG 2.2.1). | Durée allongée, pause au survol, erreurs à fermer soi-même. |
| U5 | Faible | Taille et position de la fenêtre oubliées à chaque ouverture. | Mémorisation de l'état de la fenêtre. |
| U6 | Faible | Après une action, l'interface est reconstruite et le focus clavier est perdu. | Focus rétabli sur le même contrôle. |

### Code et documentation

| N° | Gravité | Constat | Action |
| --- | --- | --- | --- |
| C1 | Info | Code clair, commenté au bon niveau, sans avertissement de lint. | Aucune. |
| C2 | Faible | Le README mêle guide utilisateur et notes techniques. | README orienté utilisateur ; documentation technique dans `docs/DEVELOPPEMENT.md`. |

## Décisions qui restent au propriétaire

1. **Signature du code** (P1) : choisir un certificat. Sans signature, l'avertissement SmartScreen restera.
2. **Licence** (P6) : libre ou propriétaire. Sans licence explicite, personne n'a légalement le droit de redistribuer le logiciel.
3. **Éditeur affiché** : la valeur « MonitorMG » est utilisée ; à remplacer par votre nom ou celui de votre structure si vous le souhaitez (`bundle.publisher`).
4. **Hébergement des versions** (P9) : il conditionne la mise à jour automatique.
5. **Anglais** (P8) : recommandé pour une diffusion hors des pays francophones.

## Résultats après mise à niveau (version 1.0.0)

Toutes les actions ci-dessus ont été appliquées, sauf les décisions qui reviennent au propriétaire (P1, P6 pour la licence, P8, P9). Chaque point a été vérifié sur la machine de test (AORUS FO32U2 et BenQ EW3270U, GPU AMD, Windows 11 25H2).

### Mesures avant et après

| Mesure | 0.1.0 | 1.0.0 |
| --- | --- | --- |
| Commande `state` (médiane sur 10 appels) | 62 ms | **10 ms**, grâce au cache des modes du pilote (énumération de 50 à 80 ms) |
| Au repos, zone de notification seule | 2,6 Mo privés | 2,7 Mo privés |
| Ouverture de la fenêtre (page chargée) | 0,48 s | 0,51 à 0,64 s |
| Lectures DDC/CI | à chaque rafraîchissement | au plus une toutes les 5 s, plus une au branchement |
| Erreurs dans la console au chargement | 12 (IPC bloqué par la CSP, voir ci-dessous) | **0** |
| Tests automatisés | 3 | 8, plus 1 test réel optionnel |
| `cargo clippy` | 0 avertissement | 0 avertissement |
| axe-core, WCAG 2.1 AA, thèmes clair et sombre | 0 violation (1 vue) | 0 violation sur les 3 vues, après correction d'un contraste de 4,16:1 en thème clair |
| Installateur / portable | 1,7 Mo / absent | 3,5 Mo (WebView2 embarqué) / 2,3 Mo |

### Constat découvert pendant la vérification

**S6 (Moyenne)** : la politique de contenu, déjà en place dans la version 0.1.0, bloquait le canal IPC rapide de Tauri (`http://ipc.localhost`). Chaque appel retombait sur un canal de secours plus lent et la console comptait 12 erreurs au chargement. Corrigé par `connect-src ipc: http://ipc.localhost`, comme le recommande la documentation de Tauri.

### Vérifications réelles

| Vérification | Résultat |
| --- | --- |
| Édition portable | Profils, journal et données WebView2 écrits dans `data\` à côté de l'exécutable, rien dans `%APPDATA%` |
| Permissions par fenêtre | La fenêtre principale se voit refuser `confirm` (« not allowed by ACL ») |
| Modification puis annulation | Disposition appliquée puis annulée, les deux étapes consignées dans l'Activité avec leur source |
| Installateur silencieux | Installation par utilisateur, entrée « MonitorMG 1.0.0 » avec son éditeur, raccourci dans le menu Démarrer |
| Désinstallation | Lancement avec Windows retiré, dossier, raccourci et entrée supprimés, données conservées quand la case n'est pas cochée |
| Propriétés de l'exécutable | Version 1.0.0, éditeur et copyright renseignés |
| Journal de diagnostic | Version, édition, version de Windows et liste des écrans consignées au démarrage |

### Reste à faire avant la publication

1. ~~Choisir et ajouter une licence~~ : licence MIT ajoutée (`LICENSE`, page de l'installateur, édition portable).
2. Obtenir un **certificat de signature** et renseigner `signCommand` (voir `docs/DEVELOPPEMENT.md`).
3. Créer le **dépôt git** et y publier les fichiers de `dist/`.
4. Facultatif : traduction anglaise, mise à jour automatique.
