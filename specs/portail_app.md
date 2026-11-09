# Spécification Technique : Module Portail Applicatif (App Center / Self-Service Portal)

## 1. Introduction et Objectif du Module
Le **Portail Applicatif** est un module optionnel couplé à notre agent de gestion de correctifs (Patchmon). Son objectif est de fournir un catalogue d'applications autogéré par les administrateurs et accessible aux utilisateurs finaux (sur postes locaux) ou aux administrateurs système (sur serveurs headless), permettant d'installer et de désinstaller des logiciels **sans nécessiter de privilèges root** pour l'utilisateur, grâce à une délégation sécurisée via le serveur et l'agent.

---

## 2. Architecture et Flux d'Exécution (Installation / Désinstallation)
Le mécanisme repose sur une architecture découplée où l'utilisateur initie l'action via le portail, qui transmet l'ordre au serveur central, lequel valide les droits et pilote l'agent (qui possède les privilèges root nécessaires).

```
[ Utilisateur (GUI / TUI) ] 
       │ (Clic sur "Installer" / "Désinstaller")
       ▼
[ Serveur Central ] 
       │ (Vérification des habilitations de l'application & de l'utilisateur)
       ▼
[ Agent (Exécuté en root) ] 
       │ (Exécution locale isolée du paquet ou du script d'installation)
       ▼
[ Système / Application opérationnelle ]
```

### 3.1. Séparation des Privilèges et Sécurité
* **Principe du Moindre Privilège :** L'utilisateur final n'a aucun droit d'administration sur sa machine. Le portail ne fait qu'émettre une intention d'action.
* **Validation Serveur :** Le serveur vérifie que l'application demandée fait bien partie du catalogue autorisé pour cet utilisateur ou ce groupe d'actifs avant de signer et de transmettre l'ordre à l'agent.
* **Exécution Agent :** L'agent (tournant en root) réceptionne l'ordre, vérifie la signature de la commande, et déploie le paquet selon le profil défini (ex: extraction dans un répertoire sécurisé, exécution de scripts packagés, ou appel au gestionnaire de paquets natif de manière contrôlée).

---

## 3. Le Catalogue Administrateur (Management Interface)
Depuis l'interface de management centrale, l'administrateur dispose d'un espace de gestion du catalogue d'applications :

* **Sources des Applications :**
  * **Applications externes :** Liens vers des dépôts officiels, paquets globaux, ou sources externes validées.
  * **Applications internes (Pré-packagées) :** Indication du chemin d'accès (ex: partage réseau interne, archive binaire, script d'installation personnalisé ou recette de déploiement interne).
* **Métadonnées de l'Application :**
  * Nom et version de l'application.
  * **Catégorie :** Regroupement logique (ex: *Bureautique, Développement, Sécurité, Outils Métier*).
  * **Description :** Texte explicatif enrichi (Markdown ou texte brut) pour l'utilisateur.
  * **Image / Icône :** Logo ou illustration graphique pour l'affichage dans les portails.
* **Ciblage (Scopes) :** Attribution des applications à des groupes d'utilisateurs ou de machines spécifiques (ex: l'application de dev n'apparaît que pour l'équipe de développement).

---

## 4. Déclinaisons du Portail Client (Interface Utilisateur)
Le module propose deux versions distinctes de l'interface client, déployables et configurables selon la nature de la cible :

### 4.1. Version GUI (Graphical User Interface)
* **Cible :** Postes de travail locaux des utilisateurs sous Linux (GNOME, KDE, etc.) ou environnements graphiques.
* **Expérience Utilisateur (UX) :**
  * Une application de type "App Store" / "Centre de logiciels" épuré.
  * Affichage des applications par **catégories** sous forme de grilles avec icônes, titres et descriptions.
  * Boutons d'action contextuels clairs : **"Installer"**, **"Installé"**, ou **"Désinstaller"**.
  * Indicateur de progression en temps réel lors des opérations.

### 4.2. Version TUI (Text User Interface)
* **Cible :** Serveurs distants, environnements sans interface graphique (*headless*), ou sessions SSH.
* **Expérience Utilisateur (UX) :**
  * Une interface textuelle interactive (basée sur des bibliothèques type `textual` ou `ncurses`).
  * Navigation au clavier intuitive par menus et catégories.
  * Commandes textuelles rapides et affichage des logs d'installation/désinstallation en direct dans le terminal.

---

## 5. Cycle de Vie d'une Application (Installation & Désinstallation)

1. **Parcours d'Installation :**
   * L'utilisateur ouvre le portail (GUI ou TUI).
   * Il navigue dans les catégories et choisit une application.
   * Il clique sur **Installer**.
   * Le portail envoie un payload JSON au serveur (`POST /api/v1/portal/install`).
   * Le serveur relaie la tâche à l'agent de la machine concernée.
   * L'agent procède à l'installation silencieuse/encadrée et retourne le statut au serveur, qui met à jour l'interface.

2. **Parcours de Désinstallation :**
   * Les applications déjà installées et gérées par le portail affichent un bouton **Désinstaller**.
   * L'utilisateur clique sur Désinstaller.
   * Le même processus sécurisé (`Portail -> Serveur -> Agent`) est enclenché pour purger l'application ou supprimer les binaires associés sans impacter les autres utilisateurs ni nécessiter de droits root manuels.

---

## 6. Contrat d'Interface (API REST - Exemples)

### Requête d'Installation
```json
{
  "request_id": "req_portal_001",
  "asset_id": "ws-user-042",
  "app_id": "app_internal_ide_v2",
  "action": "INSTALL",
  "initiated_by": "j.dupont"
}
```

### Réponse du Serveur
```json
{
  "status": "queued",
  "task_id": "task_exec_98765",
  "message": "Deployment command successfully sent to agent."
}
```