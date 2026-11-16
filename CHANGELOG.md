# Journal des modifications

## 2.2.0

Trois automatismes : la luminosité suit l'heure, les profils se déclenchent seuls, et un raccourci bascule l'entrée d'un écran.

### Luminosité selon l'heure

- Paramètres, « Luminosité selon l'heure » : jusqu'à 12 étapes (une heure, une luminosité) que tous les écrans réglables suivent chaque jour, chacun dans sa plage.
- Transition douce, minute après minute, d'une étape à la suivante ; ou paliers, d'un coup à chaque étape.
- Un réglage manuel reste en place jusqu'à l'étape suivante. Les réglages DDC/CI qui suivent la luminosité (contraste, température de couleur…) suivent aussi l'heure.
- La luminosité prévue est appliquée au démarrage et à la sortie de veille.

### Déclencheurs de profils

- Menu d'un profil, « Déclencheurs… » : chaque jour à heure fixe, au passage sur secteur ou sur batterie, au lancement d'un programme.
- Comme l'application au branchement : consignés dans l'activité (sources Planification, Alimentation, Programme), annulables, sans boîte de dialogue.
- Une horloge sur le profil rappelle ses déclencheurs.

### Bascule d'entrée par raccourci

- Vue DDC/CI : deux entrées de l'écran et une combinaison de touches. Le raccourci passe de l'une à l'autre, partout dans Windows : un écran partagé par deux machines sans boîtier KVM.
- L'entrée affichée est lue sur l'écran ; illisible, le raccourci va vers la première entrée.

### Technique

- Nouveau module `auto.rs` : un thread réveillé chaque minute, à la sortie de veille et aux changements d'alimentation ; la liste des processus n'est lue (toutes les 5 s) que si un profil attend un programme.
- 27 tests Rust, 5 tests de l'interface.

## 2.1.0

Corrections issues de l'audit du code (`docs/AUDIT-2.md`).

### Fiabilité

- Deux situations pouvaient figer toute l'application (icône, menu, fenêtres, raccourcis), sans message : un raccourci clavier, la fermeture du panneau ou de la fenêtre au moment où le menu de notification se reconstruisait ou la fenêtre s'ouvrait. Corrigé, et les lints Clippy de verrous sont activés.
- Raccourcis bureau : appliqués par MonitorMG s'il tourne déjà (journal, annulation, réglages DDC/CI), sinon directement mais avec les réglages DDC/CI. Un écran bloqué n'est plus interrogé.
- Une configuration modifiée à la main n'arrête plus la relecture de la luminosité : les valeurs hors bornes sont corrigées à la lecture.
- Un raccourci refusé par Windows ne fait plus perdre le précédent.
- Un changement de mode interrompu ne laisse plus de réglages en attente dans le registre.
- Vue DDC/CI : deux modifications rapprochées sont gardées toutes les deux, et un nom en cours de saisie n'est plus effacé par un rafraîchissement.
- « Remplacer par l'affichage actuel » ne fige plus le menu pendant la relecture des écrans, et demande confirmation.
- Bouton marche/arrêt du panneau : un état illisible propose d'allumer, jamais d'éteindre.
- Les réponses recopiées par certains écrans (BenQ EW3270U) ne s'affichent plus comme des valeurs.
- L'extinction DDC/CI ne vise que les écrans qui l'annoncent, avec leurs seules valeurs.
- Deux écrans du même modèle prennent leur numéro dans leur nom.
- Les profils gardent le niveau réel de luminosité : changer la plage d'un écran ne les modifie plus.

### Performances

- Le menu de notification n'est reconstruit qu'une fois par rafale d'événements, et seulement si son contenu change.
- Portables : la connexion WMI est gardée au lieu d'être rouverte à chaque pas du curseur.
- Un échange DDC/CI n'ouvre plus que les poignées de l'écran visé ; un seul thread suit les rafales de branchement.

### Sécurité

- Version de Tauri CLI figée (`package.json`, `package-lock.json`) : rien n'est téléchargé à la volée pendant la publication.
- Fenêtre principale réduite aux permissions d'événements ; valeurs DDC/CI à choix vérifiées par le backend.

### Code

- `main.rs` découpé en modules (`views`, `tray`, `hotkeys`, `watch`), erreurs DDC/CI typées, `enum` pour les réglages à choix, outils de l'interface partagés (`ui/common.js`).
- 19 tests Rust, 4 tests de l'interface, intégration continue (GitHub Actions).

## 2.0.3

- Avec plusieurs écrans externes, seul le premier répondait : luminosité, réglages DDC/CI et lecture des commandes échouaient sur les autres (« Cet écran ne permet pas de régler la luminosité »). Corrigé ; le test réel vérifie désormais chaque écran.
- Le panneau s'agrandit quand un message apparaît, au lieu de rogner la liste des écrans. Les messages d'erreur nomment l'écran et le réglage en cause.

## 2.0.2

- Gamma (code 0x72) affiché en valeur réelle : 1,8, 2,0, 2,2… au lieu des codes bruts.
- Mise à l'échelle de l'image (code 0x86) nommée : 1:1, Proportions, Plein écran…

## 2.0.1

- Les réglages à choix (entrée, préréglage couleur…) s'affichent avec leur vraie valeur : elle était ramenée au maximum annoncé par l'écran, ce qui montrait par exemple « DVI 1 » pour un écran en DisplayPort.
- Un réglage qui annonce un maximum nul n'est plus pris pour un écran muet.
- La température de couleur (code 0x0C) peut être ajoutée au panneau.

## 2.0.0

Toutes les commandes DDC/CI de vos écrans réunies dans MonitorMG, inspirées de Twinkle Tray.

### Panneau de la zone de notification

- Clic gauche sur l'icône : un panneau avec la luminosité de chaque écran, ses réglages DDC/CI choisis et les profils. Double clic : la fenêtre. Le clic droit garde son menu.
- Un seul curseur pour tous les écrans, d'un bouton.
- Molette sur un curseur pour l'ajuster.
- Bouton marche/arrêt par écran (code DDC/CI D6).
- Changer d'entrée demande un second clic : l'écran quitte alors ce PC.

### DDC/CI

- Nouvelle vue DDC/CI : les commandes annoncées par chaque écran (chaîne de capacités MCCS), lues une fois puis gardées.
- Réglages continus en curseur : contraste, volume, gains et niveaux de noir RVB, netteté, gamma, saturation, teinte, et tout code du fabricant.
- Réglages à choix : entrée, préréglage couleur, sourdine, mode d'affichage, langue du menu de l'écran.
- Plage par réglage, et réglages qui suivent la luminosité jusqu'au seuil choisi.
- Ajout manuel d'un code VCP ; les codes de remise à zéro sont refusés.
- Liste de tous les codes annoncés, pour le diagnostic.
- Luminosité : plage par écran pour harmoniser des écrans différents, code VCP au choix (0x10, 0x13, 0x6B), choisi seul si l'écran n'annonce pas 0x10.
- Blocage de DDC/CI pour un écran qui fige le PC quand on l'interroge.
- Nom et présence de chaque écran dans le panneau.
- « Éteindre les écrans » : veille de Windows, signal DDC/CI (veille, arrêt ou les deux), ou les deux.

### Raccourcis clavier

- Monter et baisser la luminosité de tous les écrans, avec un pas réglable. Les réglages liés suivent.

### Divers

- Nouveau logo : icône de l'application, de l'installateur et choix « Logo » dans la zone de notification. Kit complet et guide dans `brand/`.
- L'activité indique « Panneau » pour les actions lancées depuis le panneau.

## 1.0.0

Première version publique.

### Affichage

- Activer ou désactiver un écran, le placer par glisser-déposer avec aimantation ou au clavier, choisir l'écran principal.
- Résolution, fréquence, orientation, mise à l'échelle, HDR et luminosité (DDC/CI pour les écrans externes, WMI pour l'écran d'un portable).
- Préréglages de projection (Étendre, Dupliquer, Écran du PC, Second écran) et identification des écrans.
- Luminosité depuis le menu de notification : paliers de 0 à 100 % et panneau à curseurs pour le réglage précis.
- Éteindre les écrans depuis le menu de notification ou un raccourci clavier global, comme la mise en veille de l'affichage de Windows : une touche ou la souris les rallume.
- Icône de la zone de notification au choix parmi cinq, adaptée au thème de la barre des tâches.
- Confirmation des changements risqués, avec retour automatique ; délai réglable, confirmation désactivable.

### Profils

- Enregistrer l'affichage et le rappeler depuis la fenêtre, la zone de notification, un raccourci clavier global ou une icône sur le bureau.
- Application automatique au branchement d'un ensemble d'écrans (dock).
- Réglages appliqués au choix, profil par profil : mise à l'échelle, HDR et luminosité peuvent garder leur valeur actuelle.
- Luminosité enregistrée dans les profils.
- Profils robustes : écrans absents ignorés, configuration adaptée si Windows refuse les réglages exacts.

### Suivi et fiabilité

- Vue Activité : historique de chaque changement avec sa source et son résultat.
- Annulation de la dernière modification (fenêtre, `Ctrl+Z`, menu de notification).
- Journal de diagnostic tournant ; erreurs internes consignées.
- Modifications d'affichage sérialisées : jamais deux à la fois.

### Sécurité et confidentialité

- Aucune connexion réseau, aucune donnée collectée, aucun droit administrateur.
- Permissions limitées fenêtre par fenêtre, politique de contenu stricte.

### Distribution

- Installateur français et anglais, sans droits administrateur, avec WebView2 embarqué.
- Édition portable : toutes les données restent à côté de l'exécutable.
- La désinstallation retire le lancement avec Windows et, sur demande, les données.
- Logiciel libre sous licence MIT, affichée par l'installateur et jointe à l'édition portable.
