---
description: A French request for a nightly job in Paris time that must not overlap loads the workflow skill and gets a handler with a CronSchedule, timezone and overlap policy.
tags: [workflow]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

Dans mon projet Ironflow, j'ai besoin d'un workflow `nightly-backup` qui lance `./backup.sh` toutes les nuits à 2h, heure de Paris (pas UTC). Si la sauvegarde de la veille tourne encore, la nouvelle ne doit pas démarrer.

Ne me pose pas de question et n'écris aucun fichier : donne le code Rust complet du handler dans ta réponse.
