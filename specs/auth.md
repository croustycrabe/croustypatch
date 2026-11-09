# Spécification Technique : Interface Utilisateur & Gestion des Accès (CroustyPatch)

## 1. Objectifs de l'Interface Utilisateur
Fournir une expérience d'administration claire, modulaire, sécurisée et ergonomique permettant de piloter l'ensemble du parc de machines (serveurs et postes utilisateurs) via **CroustyPatch**. L'interface s'adapte dynamiquement aux rôles des utilisateurs, aux permissions RBAC et aux modules activés.

---

## 2. Authentification, SSO et Gestion des Accès

### A. Gestion du Compte Super Administrateur (Bootstrap & Sécurité CLI)
* **Création au Premier Démarrage (Bootstrap) :** Lors de la toute première initialisation du serveur CroustyPatch, un compte **Super Admin** de secours est généré automatiquement. Le mot de passe temporaire est exigé ou affiché de manière sécurisée.
* **Désactivation / Verrouillage par la CLI :** Une fois les administrateurs configurés (notamment via SSO), le compte Super Admin initial peut être désactivé ou restreint directement depuis la ligne de commande du serveur :
  ```bash
  croustypatch-server admin disable --super-admin-fallback
  ```

### B. Authentification SSO & Mappage des Groupes
* **Connexion SSO :** Intégration d'un fournisseur d'identité externe (SAML 2.0 / OIDC / LDAP / Keycloak / Azure AD) pour l'authentification des utilisateurs.
* **Récupération Automatique des Groupes :** Lors de la connexion d'un utilisateur via le SSO, l'application récupère automatiquement les groupes d'appartenance de l'utilisateur depuis le jeton ou l'annuaire.
* **Mappage des Droits (Mapping SSO Group -> Roles/Permissions) :** 
  * Un panneau d'administration dédié permet à l'administrateur de lier (mapper) un groupe SSO externe à un **rôle** interne de CroustyPatch (ex: le groupe Active Directory `SecOps-Team` est mappé sur le rôle personnalisé "Auditeur / SecOps").
  * À chaque connexion, les droits de l'utilisateur sont dynamiquement synchronisés selon son appartenance aux groupes SSO.

### C. Contrôle d'Accès Basé sur les Rôles (RBAC)
* **Permissions Élémentaires :** `read`, `create`, `write`, `delete`.
* **Création de Rôles Personnalisés :** Regroupement des permissions par modules et actions pour s'adapter aux profils métiers.

---

## 3. Le Dashboard Personnalisable (Tableau de Bord Principal)
* **Principe :** Page d'accueil de l'interface de management offrant une vue synthétique et modulaire de l'état global de l'infrastructure.
* **Widgets personnalisables (Drag-and-Drop) :**
  * *Widget État du Parc :* Répartition des machines (en ligne, en maintenance, en alerte critique ou en quarantaine).
  * *Widget Conformité & Vulnérabilités :* Score global OpenSCAP et nombre de CVE critiques non patchées.
  * *Widget Activité Récente :* Dernières campagnes de déploiement et statuts en temps réel.
  * *Widget Portail Applicatif :* Statistiques des installations d'applications par les utilisateurs.
* **Personnalisation par utilisateur :** Chaque administrateur ou opérateur peut épingler, redimensionner ou masquer les widgets selon ses besoins métier.

---

## 4. Gestion des Fonctionnalités Backend par Composant

### A. Partie Agent / Hôtes (Hosts & Agents)
Pour chaque hôte ou groupe d'hôtes, l'interface propose les sous-modules fondamentaux :
1. **Création et Gestion de Groupes d'Hôtes :** Regroupement logique des machines (ex: *Serveurs Production*, *Postes Dev*).
2. **Création de Règles (Policies) :** Configuration des règles d'automatisation (fenêtres de maintenance, mode d'analyse IA vs Déterministe, politiques de redémarrage, seuils de rollback).
3. **Historique des Actions (Audit Trail Opérationnel) :** Journal détaillé par machine et par groupe retraçant chronologiquement les scans, patchs, reboots et rollbacks.
4. **Gestion de la Conformité OpenSCAP :** Pilotage et planification des scans de conformité (CIS Benchmarks, ANSSI, STIG) avec remontée des résultats et indicateurs visuels.

### B. Partie Portail Applicatif (Centre d'Applications)
* **Principe :** Module optionnel dédié à la gestion des applications autorisées sans droits root pour les utilisateurs.
* **Fonctionnalités de l'Interface de Management :**
  * **Gestion des applications autorisées :** Validation, ajout (chemins d'accès ou sources pré-paquetées) et configuration.
  * **Liaison avec les Groupes d'Hôtes :** Une application du portail n'est visible et installable que pour les utilisateurs appartenant à des groupes d'hôtes spécifiques.
  * **Personnalisation visuelle :** Description, icône/image et classement par catégories pour l'expérience utilisateur (TUI ou GUI).
  * **Suivi des installations/désinstallations :** Visualisation des demandes et de l'état du déploiement via le workflow (Portail $\rightarrow$ Serveur $\rightarrow$ Agent).

---

## 5. Module de Notification Avancé
* **Canaux :** E-mail et SMS configurables.
* **Granularité :** Dissociation des canaux selon la criticité ou le type d'événement (urgences en temps réel vs rapports de synthèse périodiques).