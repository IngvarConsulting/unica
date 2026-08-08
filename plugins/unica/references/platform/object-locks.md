# Object Locks: Pessimistic And Optimistic

Use this reference when the concurrency in question is between *users editing the
same object*, not between transactions contending for data.

Transactional locking is a different mechanism with a different lifetime and a
different failure mode; it is owned by `transactions-locks.md`.

Two source classes back this document, and they are not interchangeable. The
behavior below — what each lock prevents, when it is released, how the two
interact with transactions — is platform behavior, taken from the 8.3.27
Developer Guide, chapter 9.2.2 `Object locking`. The single normative rule about
when to take an object lock from code is a development standard, `#std490`.
Verify a standard's wording with `unica.standards.explain`; verify behavior
against the platform documentation for the target version, because the release
notes for object locking have changed between versions.

## Two Locks, Two Jobs

The platform provides two kinds of object locking so that several users can make
holistic changes to the same objects:

- **Pessimistic** stops the object being *edited* elsewhere while someone holds
  it. It is set and released around editing.
- **Optimistic** stops a *write* whose in-memory object is stale. It is a check
  performed at write time.

They solve different halves of the same problem, and a configuration normally
relies on both.

## Pessimistic Locking

- The platform applies it through applied object form extensions. When the user
  starts modifying an object in a form the extension sets the lock; when the
  form closes the extension removes it. Another user — or the same user in
  another session — attempting to edit is notified.
- What the notification offers depends on the mode. In file mode without the
  collaboration system, the blocking user's details are shown. With the
  collaboration system available, editing can be started anyway, losing the other
  session's changes, and a message can be sent to that user. In client/server
  mode editing can likewise proceed with loss of the other side's changes.
- For a non-standard object form, reproduce the standard behavior with the form
  methods `ЗаблокироватьДанныеФормыДляРедактирования()` and
  `РазблокироватьДанныеФормыДляРедактирования()`. From code, use the object's
  `Заблокировать()` or the global `ЗаблокироватьДанныеДляРедактирования()`, and
  release with `РазблокироватьДанныеДляРедактирования()`.

**The lock is cooperative, and this is the fact everything else depends on.** It
does not prevent the object in the database from being modified or deleted. It
only prevents *another lock* being taken on the same data. An object is
effectively protected only because every path that modifies it tries to lock it
first. A single path that writes without locking makes the whole scheme
decorative — which is exactly what `#std490` requires: before modifying an
existing object from code, set the object lock.

### Lock Lifetime Depends On The Form Id

With a form id, the lock lives for the form's lifetime. It is released when the
form closes — immediately on a normal connection, after a delay on a slow one —
and also when the session ends, one minute after the form's modification flag is
cleared, when other locks are set on behalf of that form, when the form that
started background jobs closes, or through
`РазблокироватьДанныеДляРедактирования()` called with the same form id.

Without a form id, the lock is bound to nothing. It is released at session end,
when control returns from the server, at the end of the transaction if it was set
inside one, or through `РазблокироватьДанныеДляРедактирования()` with no form id.

**The two forms are incompatible.** Locking an object without a form id and then
locking the same object with one raises an exception.

### Pessimistic Locks And Transactions

- Object locking operations affect only other object locking operations. They do
  not affect data operations and they do not affect the transaction flow.
- Locking an already-locked object throws an exception that can be handled, and
  it does **not** necessarily require rolling the transaction back. An exception
  raised by `ЗаблокироватьДанныеДляРедактирования()` inside a transaction may be
  caught with `Попытка ... Исключение ... КонецПопытки` and the transaction
  continued. This is a genuine exception to the reflex of rolling back on any
  exception; see `transactions-locks.md` for the rule it qualifies.
- A lock set inside a transaction without a form id is released when the
  transaction ends.

### Two Shapes For Taking The Lock

`#std490` gives both:

- **Fail loudly.** Take the lock, and let the exception reach the user, who is
  told which session holds the object. This is the interactive shape.
- **Skip and retry later.** Take the lock inside `Попытка`, and on failure skip
  this object, write a warning to the event log, and let the next run pick it up.
  This is the shape for background and scheduled jobs, where blocking a user is
  not an option and the work is repeatable.

## Optimistic Locking

Optimistic locking prevents an object from being written when it was modified in
the database after it was read.

When a language object reads data from the database it also reads the object's
stored version. If the data in the database changes before the user starts
editing — before the pessimistic lock is taken — the stored version changes too.
At write time the platform compares the in-memory version with the stored one;
because they differ, it reports that the object version has changed or the object
was deleted.

What this guarantees is narrow and worth stating exactly: a user's changes will
not silently overwrite changes made by other sessions, **or by other software
objects within the same session**. It does not merge anything, and it does not
tell the user what changed. Re-reading and re-applying is the caller's job.

Because it costs nothing to have and cannot be switched off per write, the design
question is never whether to use it — it is whether the code handles its error
sensibly instead of surfacing a raw platform message.

## Stop Rules

- Do not treat a pessimistic lock as protection against writing. It stops other
  locks, not other writes.
- Do not modify an existing object from code without taking the object lock
  first (`#std490`).
- Do not mix locking with and without a form id for the same object.
- Do not roll a transaction back merely because taking an object lock failed.
- Do not present a version-conflict failure to the user as a raw platform error;
  say what was changed elsewhere and what they can do.
- If public MCP `unica` lacks the operation needed to inspect the editing path or
  reproduce the conflict, report a Unica MCP contract gap.
