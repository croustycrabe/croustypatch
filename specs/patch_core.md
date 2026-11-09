# Spécification Technique : Module de Remédiation et Gestion Automatisée des Correctifs (Patch Management)

## 1. Introduction et Objectifs du Module

Ce document décrit l'architecture fonctionnelle et technique du module d'automatisation des correctifs. L'objectif est de transformer un processus de patch traditionnel en un pipeline intelligent, gouverné par des politiques, capable de fonctionner dans divers environnements (connectés ou isolés/air-gapped) avec ou sans intelligence artificielle, tout en garantissant un niveau maximal de sécurité et de traçabilité (ISO 27001, SOC 2).

---

## 2. Architecture des Modes de Connectivité et Sources de Données

Le module s'adapte aux contraintes de sécurité des infrastructures d'entreprise via deux modes de déploiement réseau :

```
                        ┌───────────────────────────────┐
                        │   Sources de Vulnérabilités   │
                        └───────────────┬───────────────┘
                                        │
                 ┌──────────────────────┴──────────────────────┐
                 ▼                                             ▼
       [ Mode Connecté Internet ]                    [ Mode Full Local (Air-Gapped) ]
       - Flux CVE mondiaux en temps réel             - Flux OVAL / CVE synchronisés en local
       - Enrichissement éditeurs externes            - Base de données de vulnérabilités embarquée
```

### 2.1. Mode Full Local (Air-Gapped / Hors-Ligne)
* **Principe :** L'outil ne possède aucune dépendance sortante vers Internet.
* **Fonctionnement :** Les flux de vulnérabilités (bases OVAL, flux CVE normalisés, bulletins constructeurs) sont téléchargés périodiquement sur un bastion sécurisé, puis injectés dans la base de données locale de l'outil de patch.

### 2.2. Mode Connecté Internet
* **Principe :** L'outil interroge dynamiquement les sources ouvertes et les APIs des éditeurs pour récupérer les métadonnées de vulnérabilité dès leur publication.

---

## 3. Ingestion des Vulnérabilités et Intégration SIEM (Wazuh)

L'outil centralise la détection des failles en se connectant aux agents de sécurité du parc.

### 3.1. Intégration Native avec Wazuh (SIEM / XDR)
* **Collecte :** Récupération des rapports de vulnérabilités générés par le module *Vulnerability Detector* de Wazuh via l'API Wazuh ou par réception d'alertes en temps réel via des webhooks configurés sur Wazuh.
* **Corrélation :** L'agent associe chaque CVE détectée par Wazuh à l'inventaire matériel et logiciel de la CMDB interne.

---

## 4. Moteur d'Analyse d'Impact : Mode IA vs. Mode Déterministe (Sans IA)

L'administrateur peut choisir le mode d'évaluation du risque contextuel selon la politique de gouvernance de l'entreprise.

```
                  ┌─────────────────────────────────────────┐
                  │ Réception de l'Alerte / CVE & Cible     │
                  └────────────────────┬────────────────────┘
                                       │
                      [ Le paramètre IA est-il actif ? ]
                                       │
                 ┌─────────────────────┴─────────────────────┐
                 ▼                                           ▼
      { Mode IA Activé }                          { Mode Sans IA (Déterministe) }
      - Analyse contextuelle par LLM              - Évaluation par arbre de décision
      - Évaluation sémantique des configs        - Correspondance stricte de tags/règles
                 │                                           │
                 └─────────────────────┬─────────────────────┘
                                       ▼
                       [ Indice de Risque Contextuel ]
```

### 4.1. Option A : Mode IA (Contextual LLM Assessment)
* L'agent analyse sémantiquement la configuration de l'actif, l'exposition réseau et l'exploitabilité de la faille pour générer un score de risque contextuel nuancé.

### 4.2. Option B : Mode Sans IA (Règles Déterministes / Logique Métier)
* **Principe :** Désactivation de toute couche d'IA générative/analytique. L'évaluation repose sur un **moteur de règles booléennes strictes** défini par l'administrateur.
* **Exemple de règle déterministe :**
  ```json
  {
    "rule_id": "rule_no_ai_01",
    "condition": "cvss_score >= 8.0 AND asset_environment == 'production' AND port_exposed_to_internet == true",
    "fallback_contextual_risk": "Critical"
  }
  ```

---

## 5. Évaluation des Risques et Stratégies de Test

Avant d'appliquer un correctif, le module évalue le risque de non-régression selon la typologie de l'actif :
1. **Environnement de Staging / Digital Twin :** Clonage éphémère ou conteneurisé pour rejeu de trafic et tests de fumée automatisés.
2. **Matériel Complexe / Mainframe (Sans Staging) :** 
   * Déploiement par vagues (*Ring-Based Deployment*).
   * Exigence obligatoire d'un snapshot matériel préalable (via iLO/iDRAC ou bas niveau).
   * Respect strict de fenêtres de maintenance courtes (*Timeboxing* avec point de non-retour et rollback d'anticipation).

---

## 6. Moteur de Décision et Politiques (Policy Engine)

Le moteur croise le risque contextuel et le risque de stabilité pour décider de l'action à mener selon la configuration choisie :

```json
{
  "policy_id": "pol_enterprise_default",
  "ai_mode_enabled": true,
  "target_scope": "environment == 'production'",
  "conditions": {
    "contextual_risk": "Critical",
    "stability_risk": "Low"
  },
  "action": "AUTO_DEPLOY"
}
```

* **Actions possibles :** `AUTO_DEPLOY`, `REQUIRE_APPROVAL` (génération d'un ticket ITSM / interface web), ou `BLOCK`.

---

## 7. Surveillance Post-Déploiement et Rollback Automatique

* **Fenêtre d'Observation (Soak Period) :** Période de surveillance active après déploiement (ex: 30 minutes).
* **Déclenchement du Rollback :** Si les sondes de santé applicatives échouent ou si le taux d'erreur dépasse le seuil critique, restauration automatique de la version précédente ou du snapshot.
* **Protocole de Double Échec (Fail-Safe d'Urgence) :** Si le rollback échoue à son tour :
  1. Isolation immédiate de la machine (Quarantaine réseau).
  2. Paging d'urgence des équipes d'astreinte (intégration PagerDuty / Slack).
  3. Gel des preuves (*Forensics Freeze*).

---

## 8. Traçabilité, Conformité et Export SIEM

* **Inaltérabilité (WORM) :** Journaux d'audit chiffrés et chaînés cryptographiquement pour répondre aux exigences **ISO 27001** (Gestion des changements) et **SOC 2**.
* **Export vers le SIEM (Wazuh / Splunk) :** Envoi en temps réel des événements de remédiation via des webhooks HTTPS signés (HMAC) ou publication dans l'index de logs Wazuh pour centraliser la supervision de la sécurité.

---

## 9. Gestion des Dérogations et Exceptions (Waiver Management)

* Permet aux équipes métier de soumettre une demande de report de patch justifiée.
* **Contraintes :** Durée de validité limitée obligatoirement (*time-boxed*), obligation de renseigner des contrôles compensatoires (ex: règles WAF temporaires), et validation croisée requise (RSSI + Responsable Métier) avant enregistrement dans le registre d'audit.