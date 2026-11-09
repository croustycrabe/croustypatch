Spécification Technique : Stack Technique & Architecture (CroustyPatch)
1. Introduction

Ce document formalise les choix technologiques et architecturaux retenus pour le développement de CroustyPatch, l'outil de gestion des correctifs de la gamme Croustylab (développé par Croustycrabe). L'objectif est d'assurer une architecture robuste, hautement performante, sécurisée et modulaire, tant pour les environnements de type Home Lab que pour les déploiements d'entreprise.
2. Backend (100% Rust)

Le backend de CroustyPatch repose entièrement sur l'écosystème Rust, garantissant une excellente sécurité mémoire, des performances de haut niveau et une faible empreinte système.

    API REST & WebSockets (Actix / Actix-web) :

        Utilisation d'Actix-web pour porter l'ensemble des endpoints de l'API de management.

        Gestion des WebSockets pour maintenir le tunnel permanent et sécurisé avec les agents (permettant notamment le mode Pull et le mode éco pour les postes nomades).

    ORM et Base de Données (Diesel & PostgreSQL) :

        PostgreSQL est utilisé comme base de données relationnelle interne pour stocker les inventaires, les politiques, l'historique des actions et les journaux d'audit.

        Diesel est l'ORM retenu pour interagir de manière typée, performante et sécurisée (protection contre les injections SQL) avec la base de données.

    Gestion des Secrets & Chiffrement :

        Utilisation des crates Rust de chiffrement natif (ex: aes-gcm) pour le chiffrement applicatif des données sensibles (Column-Level Encryption).

        Gestion rigoureuse des clés en RAM avec des structures sécurisées pour purger les secrets en mémoire après utilisation.

3. Authentification, Protocoles & Standards

    OIDC (OpenID Connect) & SSO :

        Intégration d'un fournisseur d'identité OIDC pour l'authentification des utilisateurs sur l'interface de management.

        Récupération automatique des groupes d'utilisateurs et mapping des rôles et des droits par l'administrateur.

    Model Context Protocol (MCP) :

        Intégration du protocole MCP pour standardiser les interactions de l'agent IA contextuel avec le reste du système (analyse d'impact, requêtes de remédiation, génération de scripts).

4. Frontend (React / Next.js)

    Choix Technologique :

        Utilisation de React ou Next.js pour concevoir l'interface utilisateur (Dashboard personnalisable, gestion des groupes, des règles de sécurité, des audits et du portail applicatif).

    Ergonomie :

        Interface modulaire s'adaptant dynamiquement aux rôles des utilisateurs (RBAC) et aux modules activés (Home Lab vs Enterprise).