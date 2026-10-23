---
description: A French request to wait for an external CI event keyed by commit loads the workflow skill and gets a typed Signal, wait_for_signal with a timeout, and the timeout branch.
tags: [workflow]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

Projet Ironflow : le workflow `deploy-after-ci` reçoit un `sha` de commit en entrée. Il doit attendre l'événement externe « pipeline CI terminé » pour ce commit, que notre CI envoie avec un statut `success` ou `failed`, pendant une heure au plus. Il lance `./deploy.sh` seulement si le statut est `success`, et échoue si la CI a échoué ou si l'attente expire.

Ne me pose pas de question et n'écris aucun fichier : donne le code Rust complet dans ta réponse.
