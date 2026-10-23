---
description: A French request to test a handler that stops on an approval gate loads the test skill and gets a TestEngine test that asserts the suspension, resumes the run and asserts the final steps.
tags: [test]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

J'ai un handler Ironflow `Release`, enregistré sous le nom `release` : une étape shell `build` (`cargo build`), une validation humaine `ship`, puis une étape shell `publish` (`./publish.sh`). Écris un test qui vérifie que le run s'arrête sur la validation, puis qu'une fois la validation accordée il se termine avec ses trois étapes, sans lancer de vrai processus.

Ne me pose pas de question et n'écris aucun fichier : donne le fichier de test complet dans ta réponse.
