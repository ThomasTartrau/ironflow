---
description: A French request for a workflow with parallel checks and an approval gate loads the workflow skill and gets a typed WorkflowHandler.
tags: [workflow]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

Je bosse sur un projet Ironflow. Écris-moi le workflow `deploy` : en entrée un `git_ref` (obligatoire) et un `environment` ("staging" par défaut). Il build avec `cargo build --release`, lance `cargo test` et `cargo clippy` en parallèle, demande une validation humaine avant la prod, puis exécute `./deploy.sh` avec l'environnement.

Ne me pose pas de question et n'écris aucun fichier : donne le fichier Rust complet du handler dans ta réponse, puis la ligne à ajouter dans `handlers()`.
