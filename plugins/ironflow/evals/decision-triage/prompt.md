---
description: A French request for a typed machine classification with human escalation loads the workflow skill and gets a decision step with derived answers and choices.
tags: [workflow]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

Projet Ironflow : écris le workflow `triage-ticket`. En entrée, le texte d'un ticket support. Une décision machine typée (pas un agent qui répond en texte libre) classe le ticket vers l'équipe billing, technical ou sales, et dit s'il est urgent. Si le modèle n'est pas assez sûr de lui (confiance sous 0,7), un humain doit valider avant de continuer. Ensuite, le workflow lance `./route.sh` avec l'équipe choisie.

Ne me pose pas de question et n'écris aucun fichier : donne le code Rust complet dans ta réponse.
