# Spécification Technique : Module de Gestion de Conformité OpenSCAP

## 1. Objectif du Module
Intégrer l'évaluation de la conformité de sécurité (Security Compliance) au sein de l'agent de patch Patchmon en s'appuyant sur l'outil standard **OpenSCAP** (`oscap`). Ce module permet à l'agent de lancer périodiquement des audits de configuration et de vulnérabilités sur la machine hôte en utilisant des profils de conformité normalisés (ex: ANSSI, CIS Benchmarks, STIG), puis de remonter les résultats de manière sécurisée au serveur central.

---

## 2. Architecture et Fonctionnement Général

```
                      ┌───────────────────────────────┐
                      │    Serveur Central / GRC      │
                      └───────────────▲───────────────┘
                                      │ Remontée des Rapports XML/ARF
                                      │ & Synchronisation des Profils
                                      ▼
                      ┌───────────────────────────────┐
                      │    Agent Patchmon (Core)      │
                      └───────────────┬───────────────┘
                                      │
                      ┌───────────────▼───────────────┐
                      │    Module OpenSCAP (Option)   │
                      └───────────────┬───────────────┘
                                      │
         ┌────────────────────────────┴────────────────────────────┐
         ▼                                                         ▼
[ Exécution Périodique (Cron/Timer) ]            [ Analyse XCCDF / OVAL ]
- Exécution de l'outil `oscap xccdf eval`        - Évaluation locale des règles
- Utilisation des profils définis               - Génération du rapport chiffré
```

1. **Planification :** Le serveur central déploie une politique de conformité contenant les plannings et les profils OpenSCAP à exécuter sur l'actif.
2. **Exécution Locale :** L'agent (s'exécutant avec les privilèges `root` requis pour inspecter l'ensemble du système de fichiers et des configurations) déclenche périodiquement l'outil OpenSCAP.
3. **Collecte et Remontée :** L'agent analyse le rapport généré, extrait les indicateurs clés (score de conformité, règles échouées/réussies) et transmet le rapport brut (format ARF/XML) et synthétique au serveur central.

---

## 3. Spécifications Détaillées des Composants

### 3.1. Gestion des Profils et des Datastreams
* **Sources des profils :** Les flux SCAP (fichiers XCCDF / OVAL) peuvent être fournis :
  * Soit par les paquets natifs de la distribution (ex: `scap-security-guide`).
  * Soit par des flux personnalisés poussés par le serveur central (ex: profil de durcissement interne à l'entreprise).
* **Sélection du profil :** Le profil est référencé par son identifiant unique (ID) dans le profil de conformité de l'agent (ex: `xccdf_org.ssgproject.content_profile_cis`).

### 3.2. Exécution Périodique et Planification (Scheduler)
* L'agent intègre un planificateur interne (ou s'appuie sur le timer système) pour lancer les scans à intervalle régulier (ex: quotidien, hebdomadaire) afin de ne pas impacter les performances de la machine pendant les heures de forte charge.
* **Isolation des ressources :** L'exécution de l'audit OpenSCAP est limitée en ressources CPU/I-O (via `nice` et `ionice`) pour garantir la stabilité de la production.

---

## 4. Modélisation de la Politique de Conformité (Exemple JSON)

La configuration de la conformité OpenSCAP est intégrée dans le modèle de politique global de l'agent :

```json
{
  "policy_id": "pol_compliance_anssi_high",
  "target_scope": "environment == 'production' AND asset_type == 'linux_server'",
  "openscap_config": {
    "enabled": true,
    "schedule_cron": "0 2 * * 0",
    "scap_content_path": "/usr/share/xml/scap/ssg/content/ssg-rhel9-ds.xml",
    "profile_id": "xccdf_org.ssgproject.content_profile_anssi_bp28_enhanced",
    "reporting": {
      "send_full_arf": true,
      "alert_on_score_below": 85.0
    }
  }
}
```

---

## 5. Contrat d'Interface (Payload de Remontée des Résultats)

Une fois le scan terminé, l'agent envoie le résultat de l'audit au serveur central via une requête API sécurisée :

* **POST /api/v1/compliance/reports**
```json
{
  "scan_id": "scan_osc_987654",
  "timestamp": "2026-11-05T04:00:00Z",
  "asset_id": "srv-app-prod-01",
  "profile_used": "xccdf_org.ssgproject.content_profile_anssi_bp28_enhanced",
  "compliance_summary": {
    "total_rules_checked": 340,
    "passed": 310,
    "failed": 25,
    "not_applicable": 5,
    "compliance_score_percent": 91.17
  },
  "failed_rules_preview": [
    {
      "rule_id": "xccdf_org.ssgproject.content_rule_disable_root_login",
      "severity": "high",
      "description": "Ensure root login is restricted."
    }
  ],
  "cryptographic_proof": {
    "arf_report_hash_sha256": "f5d967...c41a",
    "signature": "hmac_sha256_sig..."
  }
}
```

---

## 6. Traçabilité et Intégration GRC / SIEM
* **Conformité ISO 27001 / SOC 2 :** Les rapports de conformité OpenSCAP sont indexés dans le stockage inaltérable (WORM) pour attester en continu du niveau de sécurité du parc auprès des auditeurs.
* **Alerting SIEM (Wazuh / Splunk) :** Si un scan OpenSCAP détecte une baisse du score de conformité en deçà du seuil acceptable ou l'apparition de nouvelles failles de configuration critiques, l'agent émet une alerte immédiate vers le SIEM.