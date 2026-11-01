# MonitorMG

Gérez vos écrans sous Windows 10 et 11 sans passer par les Paramètres : activez ou désactivez un écran, placez-le, réglez luminosité, résolution, fréquence, orientation, mise à l'échelle et HDR. Pilotez aussi les commandes DDC/CI de vos écrans (contraste, volume, entrée, préréglage couleur, alimentation…) depuis un panneau ouvert d'un clic sur l'icône de notification. Enregistrez des profils d'affichage et rappelez-les en un clic, depuis la zone de notification, un raccourci clavier ou une icône sur le bureau.

## Installation

| Édition | Pour qui | Comment |
| --- | --- | --- |
| Installateur `MonitorMG-2.0.2-setup.exe` | La plupart des utilisateurs | Double-cliquez. Aucun droit administrateur n'est demandé ; MonitorMG s'installe pour votre compte et apparaît dans le menu Démarrer. |
| Portable `MonitorMG-2.0.2-portable.zip` | Clé USB, poste sans installation | Décompressez dans un dossier personnel et lancez `MonitorMG.exe`. Profils et journal restent dans le dossier `data`, à côté. |

Prérequis : Microsoft Edge WebView2, déjà présent sur Windows 11. L'installateur l'ajoute si besoin ; pour l'édition portable, [téléchargez-le ici](https://go.microsoft.com/fwlink/p/?LinkId=2124703).

Le fichier `SHA256SUMS.txt` publié avec chaque version permet de vérifier que le fichier téléchargé n'a pas été modifié.

## Utilisation

| Où | Quoi |
| --- | --- |
| Disposition | Glissez un écran pour le déplacer : il s'aimante comme dans Windows. Au clavier, sélectionnez-le et utilisez les flèches. |
| Profils | Cliquez un profil pour l'appliquer ; clic droit ou `⋯` pour le renommer, lui donner un raccourci clavier, créer son raccourci sur le bureau, l'appliquer au branchement de ses écrans (idéal avec un dock) ou choisir les réglages qu'il applique : mise à l'échelle, HDR et luminosité peuvent être laissés tels quels. |
| DDC/CI | Pour chaque écran : nom et présence dans le panneau, plage de luminosité (pour harmoniser des écrans différents), code VCP de la luminosité, blocage de DDC/CI. Puis les commandes annoncées par l'écran, à afficher dans le panneau : un curseur pour les réglages continus (contraste, volume, gains…), des choix pour les autres (entrée, préréglage couleur, sourdine), un bouton marche/arrêt. Un réglage continu peut suivre la luminosité. Un code absent de la liste peut être ajouté à la main. Enfin, le comportement de « Éteindre les écrans » : veille de Windows, signal DDC/CI, ou les deux. |
| Activité | L'historique de chaque changement, d'où qu'il vienne (fenêtre, menu, raccourci, branchement), avec **Annuler**. |
| Paramètres | Lancement avec Windows, icône de la zone de notification (cinq au choix), confirmation des changements et son délai, raccourcis pour éteindre les écrans, monter ou baisser la luminosité et le pas de luminosité, liste des raccourcis, dossier des données. |
| Zone de notification | Clic gauche : le panneau. Luminosité de chaque écran (molette sur un curseur), un seul curseur pour tous si vous le souhaitez, réglages DDC/CI choisis, bouton marche/arrêt, profils, extinction des écrans. Double clic : la fenêtre. Clic droit : profils, annulation, et pour chaque écran la luminosité (paliers, ou le panneau pour le réglage précis), la résolution, la fréquence, l'activation et l'écran principal ; projection, identification, extinction des écrans. |

Un changement risqué (résolution, fréquence, orientation, activation, projection) demande confirmation : sans réponse, l'affichage précédent revient au bout de 15 s. Le délai, de 5 à 60 s, se règle dans Paramètres.

### Raccourcis clavier

| Raccourci | Action |
| --- | --- |
| Celui de votre choix | Appliquer un profil, partout dans Windows |
| Celui de votre choix | Éteindre les écrans, partout dans Windows ; une touche ou la souris les rallume |
| Ceux de votre choix | Monter ou baisser la luminosité de tous les écrans |
| `Ctrl+S` | Enregistrer l'affichage actuel comme profil |
| `Ctrl+Z` | Annuler la dernière modification |
| `Ctrl+I` | Identifier les écrans |
| `Ctrl+1` à `Ctrl+9` | Appliquer le profil 1 à 9 |
| `1` à `9` | Choisir l'écran 1 à 9 |
| Flèches | Déplacer l'écran sélectionné |
| `F5` | Relire les écrans |

### Ligne de commande

```
MonitorMG.exe --profile <id ou nom>   applique un profil puis quitte
MonitorMG.exe --minimized             démarre dans la zone de notification
```

## Confidentialité

MonitorMG fonctionne entièrement sur votre ordinateur. Il ne se connecte jamais à Internet, ne collecte aucune donnée et n'a besoin d'aucun droit administrateur. Il écrit uniquement :

- ses profils, réglages et son journal dans `%APPDATA%\MonitorMG` (ou `data\` en édition portable) ;
- une entrée de démarrage dans le registre de votre compte, si vous activez « Lancer avec Windows ».

## Dépannage

- **Une action ne fait pas ce qui est attendu** : la vue Activité indique ce qui s'est passé et pourquoi. Le journal complet est dans `logs\activite.jsonl`, sous le dossier des données (Paramètres, « Ouvrir le dossier »). Joignez-le à votre signalement.
- **Écran noir après un changement** : attendez la fin du délai, l'affichage précédent revient seul. Sinon, `Ctrl+Z` dans la fenêtre ou « Annuler » dans le menu de notification.
- **Un écran n'apparaît pas dans le panneau, ou la vue DDC/CI dit « Commandes illisibles »** : activez DDC/CI dans le menu de l'écran. Certains adaptateurs et câbles (HDMI optique, stations d'accueil bon marché) ne le transmettent pas. Si le curseur de luminosité ne change rien, essayez un autre code de luminosité dans la vue DDC/CI.
- **Le PC ralentit quand le panneau s'ouvre** : bloquez DDC/CI pour l'écran en cause dans la vue DDC/CI.
- **Un écran éteint par DDC/CI ne se rallume pas depuis MonitorMG** : certains écrans disparaissent de Windows une fois éteints. Rallumez-le avec son bouton, ou choisissez le signal « Veille » dans la vue DDC/CI.
- **La fenêtre ne s'ouvre pas** : un message propose d'installer WebView2.
- **Avertissement SmartScreen à l'installation** : l'exécutable n'est pas encore signé. Cliquez sur « Informations complémentaires », puis « Exécuter quand même ».

## Désinstallation

Paramètres de Windows, Applications, MonitorMG, Désinstaller. Le lancement avec Windows est retiré ; cochez « Supprimer les données de l'application » pour effacer aussi vos profils et le journal. L'édition portable se supprime avec son dossier.

## Remerciements

Les commandes DDC/CI et le panneau de la version 2.0 s'inspirent de [Twinkle Tray](https://github.com/xanderfrangos/twinkle-tray), de Xander Frangos (licence MIT). Aucun code n'en est repris.

## Licence

MonitorMG est un logiciel libre, distribué sous [licence MIT](LICENSE) : vous pouvez l'utiliser, le modifier et le redistribuer, y compris à des fins commerciales, à condition de conserver la mention de copyright et le texte de la licence. Il est fourni sans aucune garantie. Le texte qui fait foi est en anglais, dans le fichier `LICENSE`.

## Développement

Voir [docs/DEVELOPPEMENT.md](docs/DEVELOPPEMENT.md), et [docs/AUDIT.md](docs/AUDIT.md) pour l'audit de la version 1.0.
