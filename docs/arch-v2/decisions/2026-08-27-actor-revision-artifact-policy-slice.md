---
id: DEC.2026-08-27.ACTOR-REVISION-ARTIFACT-POLICY-SLICE
---

# Закрытая выдача полномочий и кодирование видов файлов ревизии

Actor выдаёт одну неподделываемую authority, которая связывает retained root,
state scope и доказанные kind, format и profile выбранного source set.
Из неё одновременно строятся закрытый `RevisionArtifactPolicy` и scoped
revision service; raw production-конструкторов этих частей нет.

Типизированные manifest kinds сохраняют старые значения directory/content
и добавляют presence. Алгоритм, record schema и wire shape не меняются.
