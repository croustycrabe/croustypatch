Spécification Technique : Communication de l'Agent et Mode "Éco" (CroustyPatch)
1. Modes de Communication Agent-Serveur

Pour s'adapter aux différents types d'infrastructures (serveurs permanents vs postes utilisateurs nomades), l'agent CroustyPatch propose deux modes de communication principaux avec le serveur central :

    Mode Push (Standard) :

        L'agent prend l'initiative d'envoyer régulièrement sa télémétrie, son inventaire et ses rapports de conformité au serveur central selon une périodicité fixe.

    Mode Pull via Tunnel WebSocket (Type Patchmon) :

        L'agent initie et maintient un tunnel WebSocket persistant et chiffré vers le serveur central.

        Ce tunnel permet au serveur de pousser des ordres de manière instantanée (ex: lancement d'un patch critique, scan immédiat) sans que l'agent n'ait besoin d'exposer de port réseau ou d'interroger en permanence le serveur (économie de bande passante et réactivité maximale).

2. Sécurité du Canal de Communication

    Chiffrement obligatoire : Indépendamment du mode choisi (Push ou Pull WebSocket), l'intégralité du trafic transite exclusivement sur des canaux sécurisés par TLS 1.3.

    Authentification forte : L'établissement du tunnel ou des requêtes s'appuie sur des jetons JWT et une paire de clés RSA unique générée pour chaque agent lors de son enrôlement, garantissant l'intégrité et l'authentification mutuelle.

3. Le Mode "Éco" pour les Laptops / Postes Nomades

Lorsque l'agent est déployé sur un ordinateur portable (laptop) fonctionnant sur batterie, un trafic réseau constant ou un maintien actif de WebSocket peut impacter l'autonomie de l'appareil. Le Mode "Éco" intègre les optimisations suivantes :

    Espacement des battements (Heartbeats adaptatifs) : En mode éco, la fréquence des transmissions de télémétrie et des vérifications de routine est réduite dynamiquement lorsque l'ordinateur est détecté sur batterie (ex: passage d'un ping toutes les minutes à un check toutes les 15 à 30 minutes).

    Suspension intelligente du tunnel WebSocket : Si le laptop passe en veille ou en mode économie d'énergie du système d'exploitation, le tunnel WebSocket est proprement suspendu et se reconnecte uniquement lors du retour en activité ou lors de la prochaine connexion réseau stable (ex: connexion Wi-Fi de bureau).

    Diffèrement des tâches lourdes : Les opérations non urgentes (comme les scans OpenSCAP complets ou le téléchargement anticipé d'images Docker) sont mises en pause ou reportées tant que l'appareil n'est pas branché sur secteur.