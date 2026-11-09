# Spécification Technique : Sécurité, Réseau, Chiffrement et Communication (CroustyPatch)

## 1. Introduction
Ce document détaille les exigences de sécurité, les protocoles réseau, le chiffrement des données ainsi que les modes de communication de l'agent pour l'écosystème **CroustyPatch** (développé au sein de la gamme **Croustylab** par **Croustycrabe**). L'objectif est de garantir une étanchéité totale des flux, une protection renforcée des données sensibles au repos et une gestion intelligente de la connectivité des agents nomades.

---

## 2. Protocoles de Communication & Authentification Réseau

Les échanges au sein de l'écosystème CroustyPatch (entre les agents, le serveur central et l'interface de management) garantissent l'intégrité, la confidentialité et l'authentification mutuelle pour empêcher toute interception ou injection de commandes.

*   **Canaux de transport chiffrés (TLS 1.3) :**
    *   Toutes les communications réseau (API REST, WebSockets, flux de télémétrie) sont obligatoirement chiffrées en transit à l'aide du protocole **TLS 1.3** (avec repli strict interdit sur des versions obsolètes de TLS).
*   **Authentification Agent-Serveur (JWT & Clés RSA Custom) :**
    *   L'authentification des requêtes de l'agent auprès du serveur repose sur des jetons **JSON Web Tokens (JWT)** signés cryptographiquement.
    *   Chaque agent dispose d'une paire de clés **RSA** unique générée lors de son enrôlement initial, garantissant qu'un agent ne peut pas usurper l'identité d'un autre.
*   **Intégrité des Webhooks (Signatures HMAC) :**
    *   Pour les communications sortantes vers des SIEM (comme Wazuh) ou des plateformes de notification, chaque payload est signé via une empreinte cryptographique **HMAC-SHA256** pour attester de son authenticité.

---

## 3. Support du Reverse Proxy & Gestion SSL/TLS Contextuelle

L'application est conçue pour être intégrée de manière transparente derrière un reverse proxy (Nginx, Traefik, HAProxy, Cloudflare Tunnel) ou gérée de façon autonome.

*   **Modes de Transport Frontal :**
    *   **Mode Direct (Autonome) :** Le serveur gère lui-même les connexions HTTPS en embarquant les certificats (intégration Let's Encrypt automatisée ou certificats custom).
    *   **Mode Derrière un Reverse Proxy (Délégué) :** Le serveur CroustyPatch écoute en HTTP pur ou en interne sur le réseau privé du cluster/machine, considérant que la terminaison SSL/TLS est prise en charge par le reverse proxy amont.
*   **Prise en compte des En-Têtes de Proxy (Forwarded Headers) :**
    *   Le serveur interprète les en-têtes standardisés pour conserver le contexte réel de l'utilisateur :
        *   `X-Forwarded-For` : Récupération de l'adresse IP source réelle.
        *   `X-Forwarded-Proto` : Détection du protocole d'origine (`http` ou `https`) pour adapter les redirections et la sécurité des cookies de session.
        *   `X-Forwarded-Host` / `X-Forwarded-Port` : Conservation du domaine public traversé.

---

## 4. Gestion et Chiffrement des Données Sensibles en Base de Données

La base de données centrale de CroustyPatch (ex: PostgreSQL) stocke des informations critiques : mots de passe hachés, jetons d'accès API, secrets de configuration et rapports d'audit.

*   **Chiffrement au Repos (Encryption at Rest) :**
    *   Le stockage physique de la base de données est chiffré au niveau du système de fichiers ou du volume (ex: LUKS sous Linux, chiffrement natif cloud/hyperviseur) pour prémunir le système contre le vol de disques.
*   **Chiffrement Applicatif (Column-Level Encryption) :**
    *   Les données hautement sensibles (jetons d'accès, secrets d'intégration) subissent un chiffrement applicatif symétrique fort (**AES-256-GCM**) avant l'insertion en base de données.
*   **Gestion des Clés de Chiffrement (Key Management) :**
    *   Les clés maîtresses (Master Keys) sont externalisées via des variables d'environnement sécurisées, des fichiers de configuration aux permissions strictes (`chmod 600`), ou intégrées avec un gestionnaire de secrets externe (type HashiCorp Vault).
*   **Hachage des Mots de Passe Utilisateurs :**
    *   Les mots de passe des comptes de l'interface de management sont salés et hachés à l'aide de l'algorithme robuste **Argon2id** (recommandé par l'OWASP).

---

## 5. Modes de Communication de l'Agent et Mode "Éco"

Pour s'adapter aux différents types d'infrastructures (serveurs permanents vs postes utilisateurs nomades), l'agent CroustyPatch propose des modes de communication flexibles et économes en énergie.

### 5.1. Modes de Communication Agent-Serveur
*   **Mode Push (Standard) :** L'agent prend l'initiative d'envoyer régulièrement sa télémétrie, son inventaire et ses rapports de conformité au serveur central selon une périodicité fixe.
*   **Mode Pull via Tunnel WebSocket (Type Patchmon) :**
    *   L'agent initie et maintient un **tunnel WebSocket persistant et chiffré** vers le serveur central.
    *   Ce tunnel permet au serveur de pousser des ordres de manière instantanée (ex: lancement d'un patch critique, scan immédiat) sans que l'agent n'ait besoin d'exposer de port réseau ou d'interroger en permanence le serveur (bande passante optimisée).

### 5.2. Le Mode "Éco" pour Laptops / Postes Nomades et Statut de Supervision
Lorsque l'agent est déployé sur un ordinateur portable fonctionnant sur batterie, un trafic réseau constant ou un maintien actif de WebSocket peut impacter l'autonomie. Le **Mode "Éco"** intègre les optimisations suivantes :
*   **Espacement des battements (Heartbeats adaptatifs) :** En mode éco, la fréquence des transmissions de télémétrie est réduite dynamiquement sur batterie (ex: passage d'un ping minute à un check toutes les 15 à 30 minutes).
*   **Suspension intelligente du tunnel WebSocket :** Si le laptop passe en veille ou en économie d'énergie, le tunnel WebSocket est proprement suspendu et se reconnecte lors du retour en activité ou sur une connexion réseau stable.
*   **Notification et Statut "Éco" pour la Supervision :**
    *   Lorsque l'agent bascule en mode éco (détection d'un fonctionnement sur batterie), **il prévient immédiatement le serveur central**.
    *   Le serveur bascule automatiquement le statut de l'hôte à l'état **"Éco"** dans l'interface et auprès des outils de supervision connectés. Cela permet d'**éviter les alertes intempestives** (du type "fréquence de check dépassée" ou "ralentissement de télémétrie") et de calibrer les seuils d'alerte en conséquence.
*   **Diffèrement des tâches lourdes :** Les opérations non urgentes (scans OpenSCAP complets, téléchargements d'images Docker) sont reportées tant que l'appareil n'est pas branché sur secteur.