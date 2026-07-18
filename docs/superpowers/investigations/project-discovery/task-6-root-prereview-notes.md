# Task 6 root pre-review notes

Status: blocking notes for a future fresh review. This ignored file does not
authorize implementation.

Primary grammar checked:

- `https://github.com/1c-syntax/bsl-parser/blob/develop/src/main/antlr/BSLLexer.g4`
- `https://github.com/1c-syntax/bsl-parser/blob/develop/src/main/antlr/BSLParser.g4`

The proposed `Async? Procedure|Function` order and the core declaration grammar
match the referenced grammar. The following contradictions and missing
conservative dependencies still need resolution.

## P0: conditional compilation can affect code outside its own extent

The current design says definitions and calls inside `#If` are not promoted and
unconditional facts outside the extent remain usable. That is insufficient for
name resolution. A conditional branch can declare or assign a module/local
identifier that shadows a registered CommonModule receiver, or conditionally
define a same-module callee used by an unconditional caller. In at least one
build the outside call then resolves differently.

Required REDs:

- conditional module variable named as a CommonModule, followed by an
  unconditional qualified call;
- conditional local `Var`, implicit assignment, `For`, and `For Each` receiver,
  followed by an unconditional qualified call in the same method;
- conditional same-module definition followed by an unconditional direct call;
- conditional exported CommonModule definition used by another module;
- nested conditional branches with one shadowing path.

Every case must remain Dynamic/Ambiguous/Unknown with no runtime edge. The pure
analysis needs explicit maybe-defined/maybe-shadowed symbol sets propagated to
the containing module/method, not only a gap attached to tokens inside the
branch.

## P1: shadow analysis omits BSL binders

The draft names parameters, `Var` and a simple assignment as shadow sources.
The primary grammar also binds identifiers through `For` and `For Each`; an
lvalue can contain access/index forms, while module-body assignments can affect
all procedures. The closed v1 extractor must enumerate every accepted binder
shape or mark the caller/module unsupported. It must not resolve a qualified
CommonModule call after a binder it chose not to model.

## P1: search comparison rules contradict the token contract

`BslSignificantToken.comparison_text` is specified as Unicode-lowercase for
identifier, keyword, boolean and undefined/null tokens. Section 8 later says
boolean and undefined/null matching is exact. The primary lexer is
case-insensitive for those tokens. Choose the former rule, document it once,
and add case metamorphic tests for `TRUE/True`, `FALSE/False`,
`UNDEFINED/Undefined` and `NULL/Null`.

## P1: event and scheduled-job definition joins are now prerequisites

Task 6 cannot freeze `DefinitionShape` without the corrected Task 5A/5B
contracts:

- EventSubscription runtime context is `SameAsSourceEvent`, not `AtServer`.
- EventSubscription requires an exported synchronous Procedure whose exact
  parameter count/signature class is derived from the event plus every selected
  source family.
- ScheduledJob runtime context is Server but the BSL declaration may be module
  default; the handler may be an exported Procedure or Function in a non-global
  CommonModule callable on server. Predefined metadata jobs have no parameters;
  other jobs require exact parameter authority and cannot be guessed.
- HTTP binding server context likewise must not be compared directly to a BSL
  `&AtServer` directive.

Task 6 design must consume separate `BindingRuntimeContextV1` and
`BslExecutionContext`; it may not restore their earlier conflation.

## P1: async mode and unsupported tokens must fail closed by capability

The primary lexer enters a distinct ASYNC mode until the matching procedure or
function terminator. The handwritten lexer/parser must prove correct mode exit
for malformed and mixed-language terminators, nested delimiter failures and
preprocessor boundaries. Every source token that the subset does not classify
must create an exact capability gap; it cannot follow the reference lexer's
`UNKNOWN -> HIDDEN` behavior, because discovery needs conservative evidence.

## Review gate

Before Task 6 code, a fresh reviewer must:

1. resolve the issues above in `task-6-design.md`;
2. rebase the design onto accepted Task 5A/5B/5C SHAs and shared catalog;
3. verify all positive grammar claims against the primary grammar and official
   platform requirements;
4. produce exact REDs and an immutable reviewed SHA;
5. keep Task 6B cache optional and non-authoritative.
