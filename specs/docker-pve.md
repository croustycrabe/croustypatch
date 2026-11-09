Extension de la Spécification Technique : Gestion des Environnements Spéciaux (Docker & Proxmox Helper Scripts)
1. Architecture des Modules Optionnels (Plugins)

Pour maintenir un agent léger, sécurisé et modulaire (rappel : exécuté en tant que root avec une surface d'attaque minimale), ces fonctionnalités ne sont pas intégrées au binaire core de l'agent par défaut. Ce sont des modules externes optionnels que l'agent peut charger et administrer dynamiquement.
Options d'installation et de cycle de vie :

    À l'installation initiale : L'agent peut être déployé avec des drapeaux (flags) d'activation spécifiques (par exemple via Ansible ou en ligne de commande) :
    Bash

    patchmon-agent-installer --docker --helper-script

    En cours de cycle de vie (Post-installation) : L'administrateur peut activer, désactiver ou mettre à jour ces modules dynamiquement depuis l'interface de management, qui transmet l'ordre sécurisé à l'agent :
    Bash

    patchmon-agent module add docker
    patchmon-agent module remove helper-script

2. Module 1 : Proxmox VE Helper Scripts (Applications LXC/VM)

    Principe : Ce module cible les machines (généralement des conteneurs LXC ou des machines virtuelles sous Proxmox) dont les applications ont été initialement installées ou configurées via les scripts communautaires de type Proxmox VE Helper-Scripts.

    Fonctionnement & Monitoring :

        L'agent analyse l'inventaire local pour détecter la présence de l'indicateur ou du fichier de métadonnées laissé par le helper script.

        Il répertorie l'application et ses dépendances spécifiques.

        Alignement sur le process classique : Une fois l'application découverte, l'agent la traite exactement comme un paquet logiciel standard : remontée des versions, vérification des CVE associées, passage par le moteur de décision (politiques, évaluation du risque) et application de la mise à jour via le gestionnaire sous-jacent ou le script de mise à jour propre à l'application.

3. Module 2 : Conteneurs Docker & Images OCI

    Principe : Ce module supervise le daemon Docker local pour auditer, monitorer et mettre à jour les conteneurs et les images associées.

    Fonctionnement & Monitoring :

        Découverte dynamique : L'agent se connecte au socket Docker (/var/run/docker.sock) avec les privilèges stricts requis. Il dresse l'inventaire en temps réel des conteneurs actifs, des images utilisées, et détecte automatiquement les nouveaux ajouts de conteneurs au fil de l'eau.

        Veille sur les versions d'images : Pour chaque image monitorée, l'agent interroge (en mode local ou via le serveur central selon la connectivité) le registre distant pour savoir si une nouvelle version (tag ou digest) est disponible.

        Processus de mise à jour unifié :

            Le besoin de mise à jour passe par le processus de patch classique (évaluation du risque contextuel, règles de gouvernance, fenêtres de maintenance, et validation par l'IA ou les règles déterministes).

            Actions d'exécution spécifiques à Docker : Contrairement à un paquet système classique (apt upgrade), l'action de remédiation pour Docker suit un cycle conteneurisé spécifique :

                docker pull <image:new_version> (téléchargement de la nouvelle image).

                Arrêt gracieux du conteneur existant (docker stop).

                Suppression ou renommage de l'ancien conteneur.

                Recréation et lancement du nouveau conteneur en réappliquant rigoureusement les mêmes paramètres d'origine (volumes persistants -v, variables d'environnement -e, réseaux --net, ports -p, etc.).

                Lancement de la phase de test de santé post-déploiement (Health Check) et déclenchement éventuel d'un rollback (retour à l'ancienne image) en cas d'échec.

4. Intégration dans le Modèle de Politique (Exemple JSON)

La politique de remédiation globale s'applique également à ces environnements spéciaux, en y intégrant les spécificités d'exécution :
JSON

{
  "policy_id": "pol_special_environments",
  "target_scope": "environment == 'production' AND (has_docker_module == true OR has_helper_script_module == true)",
  "special_modules_rules": {
    "docker": {
      "auto_pull_images": true,
      "preserve_volumes": true,
      "rollback_on_health_check_fail": true
    },
    "proxmox_helper_scripts": {
      "follow_standard_package_pipeline": true
    }
  },
  "conditions": {
    "contextual_risk": "Critical",
    "stability_risk": "Low"
  },
  "action": "AUTO_DEPLOY"
}