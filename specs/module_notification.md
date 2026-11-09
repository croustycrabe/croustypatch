Spécification Technique : Module de Notification Avancé et Granulaire
1. Objectifs du Module

Permettre aux utilisateurs et aux administrateurs de configurer des canaux de notification sur mesure (e-mail ou SMS), de définir précisément qui doit être notifié, quand (ex: en temps réel à chaque problème ou par récapitulatif périodique chaque matin pour les scans de la veille), et de quoi (filtrage par type d'événement). De plus, chaque type de notification peut s'appuyer sur un ou plusieurs canaux distincts selon la criticité (ex: alerte critique par e-mail et SMS, mais rapport de synthèse par e-mail uniquement).
2. Fonctionnalités Clés
A. Choix et Multiplicité des Canaux

    Canaux supportés : E-mail (SMTP / API Transactionnelle) et SMS (Passerelle SMS / API type Twilio, etc.).

    Dissociation par type de message : Un même événement ou rapport peut être routé vers des canaux différents. Par exemple :

        Alerte de vulnérabilité critique / échec de patch : E-mail + SMS.

        Rapport de scan quotidien / succès de déploiement : E-mail uniquement.

B. Granularité et Fréquence (Planification)

    Mode Instantané (Temps réel) : Notification déclenchée immédiatement lors d'un événement critique (ex: échec d'un déploiement, double échec de rollback, mise en quarantaine d'une machine).

    Mode Périodique / Agrégé (Digest) : Regroupement des événements sur une période donnée (ex: "Tous les matins à 08h00, envoyer le rapport des scans OpenSCAP et des patchs de la veille").

C. Ciblage et Rôles (Qui notifier ?)

    Destinataires dynamiques par périmètre : Possibilité de notifier des groupes d'utilisateurs spécifiques selon l'actif concerné (ex: les équipes DevOps pour les serveurs de staging, l'équipe Sécurité pour les alertes critiques, ou l'utilisateur final pour le redémarrage de son poste de travail).

3. Modélisation des Données (Exemple de Configuration JSON)

Voici comment une règle de notification granulaire peut être modélisée dans le système :
JSON

{
  "notification_policy_id": "notif_rule_critical_alerts",
  "target_audience": {
    "roles": ["secops_lead", "system_admin"],
    "specific_users": ["admin@entreprise.com"]
  },
  "triggers": {
    "events": [
      "patch_deployment_failed",
      "rollback_failed",
      "machine_quarantined"
    ],
    "frequency": "INSTANT"
  },
  "channels": {
    "sms": {
      "enabled": true,
      "min_severity": "CRITICAL"
    },
    "email": {
      "enabled": true,
      "min_severity": "HIGH",
      "include_full_report_attachment": true
    }
  }
}

Et pour un rapport périodique (digest) :
JSON

{
  "notification_policy_id": "notif_rule_daily_summary",
  "target_audience": {
    "roles": ["it_manager", "compliance_officer"]
  },
  "triggers": {
    "events": [
      "daily_openscap_summary",
      "yesterday_patch_remediation_report"
    ],
    "frequency": "SCHEDULED",
    "cron_expression": "0 8 * * *"
  },
  "channels": {
    "sms": {
      "enabled": false
    },
    "email": {
      "enabled": true,
      "template": "daily_summary_html"
    }
  }
}

