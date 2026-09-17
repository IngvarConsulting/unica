---
id: INV.APP.DAEMON-ACTOR-AUTHORITY
---

# Передача вызова обработчику через связанное право актора

После дешёвой schema-проверки daemon связывает canonical call с opaque
`ActorBoundInvocation`: retained exact actor, named physical provider root
и identity digest, полученный от того же actor. Handler не получает raw
`InvocationRequest` или `workspaceHint`.
