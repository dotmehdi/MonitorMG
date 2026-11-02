# Journal des modifications

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
