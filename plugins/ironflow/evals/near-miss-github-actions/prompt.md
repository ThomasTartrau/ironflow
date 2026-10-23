---
description: A French request for a CI workflow with parallel jobs and a manual approval, unrelated to Ironflow, must not load any Ironflow skill.
tags: [trigger]
max_turns: 5
allowed_tools: [Read, Glob, Grep, Skill]
---

Écris un workflow GitHub Actions qui lance `cargo test` et `cargo clippy` en parallèle à chaque push, puis attend une approbation manuelle via un environnement protégé avant de déployer avec `./deploy.sh`. Donne le YAML dans ta réponse, n'écris aucun fichier.
