Spécification Technique : Interface Utilisateur & Fonctionnalités (CroustyPatch)

1. Objectifs de l'Interface Utilisateur
Fournir une expérience d'administration claire, modulaire et ergonomique permettant de piloter l'ensemble du parc de machines (serveurs et postes utilisateurs) via CroustyPatch. L'interface s'adapte dynamiquement aux rôles des utilisateurs et aux modules activés.
2. Le Dashboard Personnalisable (Tableau de Bord Principal)

Principe : Page d'accueil de l'interface de management offrant une vue synthétique et modulaire de l'état global de l'infrastructure.

Widgets personnalisables (Drag-and-Drop) :
Widget État du Parc : Répartition des machines (en ligne, en maintenance, en alerte critique ou en quarantaine).Widget Conformité & Vulnérabilités : Score global OpenSCAP et nombre de CVE critiques non patchées.Widget Activité Récente : Dernières campagnes de déploiement et statuts en temps réel.Widget Portail Applicatif : Statistiques des installations d'applications par les utilisateurs.

Personnalisation par utilisateur : Chaque administrateur ou opérateur peut épingler, redimensionner ou masquer les widgets selon ses besoins métier.

3. Gestion des Fonctionnalités Backend par Composant

A. Partie Agent / Hôtes (Hosts & Agents)
Pour chaque hôte ou groupe d'hôtes, l'interface propose les sous-modules fondamentaux suivants :
Création et Gestion de Groupes :Permet de regrouper les machines par critères logiques (ex: Serveurs Production, Postes Dev, Bureaux Paris).
Les groupes servent de cible directe pour l'application des politiques de patch et des règles de sécurité.
Création de Règles (Policies) :Interface de configuration des règles d'automatisation (fenêtres de maintenance, mode d'analyse IA vs Déterministe, politiques de redémarrage et seuils de rollback).Association des règles à un ou plusieurs groupes d'hôtes.
Historique des Actions (Audit Trail Opérationnel) :Journal détaillé par machine et par groupe retraçant chronologiquement : les scans effectués, les patchs déployés, les reboots initiés, les alertes de santé et les déclenchements de rollback.
Gestion des Scans OpenSCAP :Configuration et planification des audits de conformité basés sur des profils standardisés (CIS Benchmarks, ANSSI, STIG).Visualisation des rapports de conformité par machine ou par groupe d'hôtes, avec mise en évidence des écarts de configuration et des contrôles échoués.

B. Partie Portail Applicatif (Centre d'Applications)
Principe : Module optionnel dédié à la gestion des applications autorisées sans droits root pour les utilisateurs.
Fonctionnalités de l'Interface de Management :
Gestion des applications autorisées : L'administrateur valide, ajoute (chemin d'accès pour applications internes ou sources externes / pré-paquetées) et configure les applications du catalogue.
Liaison avec les Groupes d'Hôtes : Une application du portail n'est visible et installable que pour les utilisateurs appartenant à des groupes d'hôtes spécifiques (ex: l'application de comptabilité est autorisée uniquement sur le groupe d'hôtes Postes Comptabilité).
Personnalisation visuelle : Ajout d'une description, d'une image/icône et classement par catégories pour l'expérience utilisateur finale (TUI ou GUI).

Suivi des installations/désinstallations : Visualisation des demandes en attente et de l'état du déploiement via le workflow (Portail $\rightarrow$ Serveur $\rightarrow$ Agent).