# Signature des exécutables

Comment faire disparaître « Éditeur inconnu » et réduire les avertissements SmartScreen au téléchargement des releases. État des lieux de novembre 2026 ; les offres citées évoluent, vérifier les liens avant d'agir.

## Le problème

Les exécutables publiés ne sont pas signés. Conséquences :

- SmartScreen (navigateur Edge et lancement du fichier téléchargé) affiche « Éditeur inconnu » et demande une confirmation en deux clics (« Informations complémentaires », « Exécuter quand même »).
- Certains antivirus se méfient davantage d'un binaire non signé.

À savoir avant de dépenser quoi que ce soit : **la signature ne supprime pas l'avertissement SmartScreen du jour au lendemain**. Elle remplace « Éditeur inconnu » par le nom de l'éditeur et, surtout, elle donne au certificat une réputation qui se construit avec les téléchargements et **survit aux versions suivantes** signées du même certificat. Sans signature, chaque nouveau fichier repart de zéro.

## Voie recommandée : SignPath Foundation (gratuit pour l'open source)

[SignPath Foundation](https://signpath.org) signe gratuitement les projets open source. MonitorMG remplit les conditions de fond ([conditions](https://signpath.org/terms.html)) :

- licence approuvée OSI sans double licence commerciale (MIT ✓), code entièrement open source ✓, dépôt public ✓, téléchargements gratuits ✓ ;
- MFA obligatoire pour les mainteneurs, sur GitHub et sur SignPath ;
- les binaires signés doivent être **construits par la CI du dépôt** (« origin verification ») : c'est le rôle du workflow `.github/workflows/release.yml` ajouté à cet effet ;
- chaque release doit porter la mention « Free code signing provided by SignPath.io, certificate by SignPath Foundation » (le workflow l'ajoute aux notes de release) ;
- l'éditeur affiché par Windows sera « SignPath Foundation », pas votre nom — c'est le principe de leur certificat mutualisé ;
- la fondation vérifie la réputation du projet avant d'accepter : un refus est possible pour un projet encore peu visible ; dans ce cas, re-candidater plus tard ou passer à l'option Certum ci-dessous.

### Étapes (à faire par un humain, dans l'ordre)

1. Activer la MFA sur le compte GitHub.
2. Candidater sur [signpath.org/apply](https://signpath.org/apply) : nom du projet, lien du dépôt, licence MIT, description (gestionnaire d'écrans Windows), lien des releases.
3. À l'acceptation, dans le tableau de bord SignPath :
   - créer le projet et coller la configuration d'artefact fournie dans [`dev/signpath-artifact-configuration.xml`](../dev/signpath-artifact-configuration.xml) (la valider dans leur éditeur) ;
   - créer une signing policy « release-signing » ;
   - relier le dépôt GitHub comme *trusted build system* (c'est ce qui active la vérification d'origine) ;
   - générer un jeton d'API.
4. Dans GitHub, Settings → Secrets and variables → Actions :
   - secret `SIGNPATH_API_TOKEN` ;
   - variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`.
5. Publier en poussant un tag `vX.Y.Z` : le workflow construit, envoie `setup.exe` et le `MonitorMG.exe` portable à SignPath, attend la signature (l'approbation se fait à la main dans leur tableau de bord), reconstruit le zip portable avec l'exécutable signé, régénère `SHA256SUMS.txt` et crée la release en **brouillon** ; il ne reste qu'à coller les notes du CHANGELOG et publier.

Tant que les variables ne sont pas configurées, le workflow saute la signature et produit une release non signée : rien n'est cassé en attendant.

### Limite de ce flux

Seuls les deux fichiers téléchargés par les utilisateurs sont signés. Le désinstallateur et les DLL embarqués dans l'installateur NSIS ne le sont pas. C'est là que SmartScreen agit (les fichiers portant la marque du web), donc l'essentiel est couvert ; une signature complète est possible plus tard via `bundle.windows.signCommand` (voir ci-dessous), si SignPath confirme la soumission fichier par fichier depuis le build.

## Option payante : certificat Certum « Open Source »

Si SignPath refuse ou si vous préférez votre nom comme éditeur : [Certum](https://shop.certum.eu/code-signing.html) vend un certificat de signature de code « Open Source » accessible aux particuliers (validation d'identité individuelle ; l'éditeur affiché est de la forme « Open Source Developer, Prénom Nom »). Ordre de grandeur constaté : ~70 à 110 € selon support et durée ; la clé privée vit dans leur cloud SimplySign (ou une carte), non exportable — c'est une exigence générale des certificats de signature depuis 2023.

Avantage sur le flux SignPath : la signature se fait en local pendant `node scripts/release.mjs`, donc **tout** est signé (exécutable, désinstallateur, installateur), via `bundle.windows.signCommand` dans `tauri.conf.json` :

```json
"signCommand": "signtool sign /fd SHA256 /tr http://time.certum.pl /td SHA256 /n \"Open Source Developer\" %1"
```

(`signtool` avec SimplySign installé ; `%1` est remplacé par le fichier à signer. Tauri signe alors chaque binaire puis l'installateur pendant la construction.)

## Azure Artifact Signing (ex-Trusted Signing) : pas accessible

L'offre de Microsoft (~10 $/mois) serait idéale, mais la validation d'identité **individuelle** est limitée aux États-Unis et au Canada ([documentation](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart), [refus d'un développeur français, mars 2026](https://learn.microsoft.com/en-us/answers/questions/5810735/cant-create-a-new-trusted-signing-individual-ident)). Les organisations de l'UE sont éligibles : l'option ne redevient pertinente que si le projet est un jour porté par une entité enregistrée (micro-entreprise avec SIRET, association…).

## En complément : winget

`winget install MonitorMG` installe sans navigateur ni SmartScreen (winget vérifie l'empreinte SHA-256 du manifeste). C'est le canal « sans encombre » le plus simple pour les utilisateurs techniques, signature ou pas.

Des manifestes prêts pour la version 2.2.4, avec les vraies empreintes, sont dans [`dev/winget/`](../dev/winget/). Soumission :

1. `wingetcreate submit dev/winget` (ou un fork de [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) avec les fichiers sous `manifests/d/dotmehdi/MonitorMG/2.2.4/`, puis une pull request).
2. Les modérateurs peuvent demander des ajustements (identifiant du paquet, champs ARP) ; `wingetcreate` les calcule au besoin.
3. Pour chaque version suivante : `wingetcreate update dotmehdi.MonitorMG --urls <url du setup.exe> --version X.Y.Z --submit` (automatisable dans le workflow de release plus tard).

## Après la première release signée

- Adapter l'entrée « Avertissement SmartScreen à l'installation » du dépannage de `README.md` et `README.en.md` (l'avertissement devient rare puis disparaît avec la réputation ; « Éditeur inconnu » disparaît tout de suite).
- Conserver la mention SignPath dans les notes de chaque release (exigence de leurs conditions) — le workflow s'en charge.
- Vérifier la signature d'un fichier téléchargé : clic droit → Propriétés → Signatures numériques, ou `signtool verify /pa MonitorMG-X.Y.Z-setup.exe`.
