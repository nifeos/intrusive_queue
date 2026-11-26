# Changelog

Dates are UTC. One line per release, what changed and why, so a reader of the diff has the
sentence beside it.

## Unreleased

- First version.
- `Node::link` now projects to an opaque `Link<T>` field around an `UnsafeCell`, so the queue
  writes a link with no reference to the node and a shared reference to a queued node stays sound
  under both Miri borrow models.
- `Iter` is `Debug`, `Clone` and `ExactSizeIterator`, with an exact `size_hint`.
- `Queue` implements `Extend` and `FromIterator` of `Unqueued` tokens, and `&Queue` is
  `IntoIterator`, so a queue collects and loops like a standard collection.
- `Queue::front` names the head without taking it, and `Queue::append` joins two queues in
  constant time.
- `Queue<T>` is `Sync` when `T` is, as `LinkedList<T>` is.
- The examples return errors with `?` instead of unwrapping.
- The `T: Node` bound moves from the structs to the impl blocks that need it.

## BUGS

- The template's lag check guards the CI pin, not this file; a change that matters to a user of
  the crate belongs in a release line here.
