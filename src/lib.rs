//! An intrusive first in, first out queue: the link lives inside the node, so push and pop cannot allocate or
//! fail, and a pop hands back the node itself. Nothing here checks rule 2 below; the README holds
//! the contract's fine print, the examples and what the proofs cover.
//!
//! # The caller's contract
//!
//! 1. A node is on at most one queue until popped; a push consumes its [`Unqueued`] token.
//! 2. A node outlives its time on the queue; stated once, at [`Unqueued::new`].
//! 3. The caller serializes all access to a queue and its links (`&mut self`, for the queue), and
//!    mints each token from the node's owning pointer, never from a reference; stated at
//!    [`Unqueued::new`].

#![cfg_attr(not(test), no_std)]

use core::cell::UnsafeCell;
use core::ptr::NonNull;

/// The README's examples, compiled and run as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExample;

/// A type that carries its own queue link, a [`Link`] field the queue reaches through
/// [`link`](Self::link).
///
/// The queue never forms a reference to a node. It projects from the node's pointer to the
/// [`Link`], and the link keeps its pointer in an `UnsafeCell`, so writing it is not a write to
/// the rest of the node. Rule 3 still asks that no reference to a queued node be live across a
/// queue operation; this shape is what lets a later release narrow that rule to the link alone
/// without breaking a caller.
///
/// # Safety
///
/// `link` must return a pointer to a `Link<Self>` field of `*this`, the same field on every call,
/// derived from `this` without forming a reference to the node (`&raw mut`, as below), and must
/// do nothing else. The queue threads its structure through that field, so any other
/// implementation is a corrupted queue.
///
/// # Examples
///
/// The implementation every node writes, and the only correct one:
///
/// ```
/// use core::ptr::NonNull;
/// use intrusive_queue::{Link, Node};
///
/// struct Thread {
///     tid: u32,
///     link: Link<Thread>,
/// }
///
/// // SAFETY: `link` projects to the `link` field by `&raw mut`, forms no reference and does
/// // nothing else, which is the whole of the `Node` contract.
/// unsafe impl Node for Thread {
///     unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>> {
///         // SAFETY: `this` is a live node (the caller's promise), and a field of a non-null
///         // pointer to a live value is non-null.
///         unsafe { NonNull::new_unchecked(&raw mut (*this.as_ptr()).link) }
///     }
/// }
/// ```
pub unsafe trait Node: Sized {
    /// The node's link field, projected from `this` without forming a reference to the node.
    ///
    /// # Safety
    ///
    /// `this` points to a live node of this type.
    unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>>;
}

/// The queue link a [`Node`] carries: one pointer, to the node behind it, that only the queue
/// reads or writes.
///
/// Its field is private and sits in an `UnsafeCell`, so the queue may write it while the caller
/// holds a shared reference elsewhere in the node, and no caller can write it by mistake. A node
/// gets one from [`Link::new`] and otherwise leaves it alone. The link is one pointer wide.
pub struct Link<T> {
    next: UnsafeCell<Option<NonNull<T>>>,
}

impl<T> Link<T> {
    /// A link for a node on no queue.
    pub const fn new() -> Self {
        Self {
            next: UnsafeCell::new(None),
        }
    }
}

impl<T> Default for Link<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> core::fmt::Debug for Link<T> {
    /// Prints no pointer: reading the link is the queue's alone (rule 3).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Link").finish_non_exhaustive()
    }
}

// SAFETY: a link holds a pointer to another `T` and offers no access to it; only a queue reads or
// writes it, under rule 3. Moving one moves that pointer, which is sound when `T` may move between
// threads, as for `LinkedList<T>`.
unsafe impl<T: Send> Send for Link<T> {}

// SAFETY: a shared `Link` offers no method that reads or writes the cell, so sharing one shares
// nothing; the queue reaches it through raw pointers under rule 3.
unsafe impl<T: Sync> Sync for Link<T> {}

/// The link stored after `node`, read through the node's projection.
///
/// # Safety
///
/// `node` points to a live node, and nothing writes its link during the call (rule 3).
#[inline(always)]
unsafe fn read_link<T: Node>(node: NonNull<T>) -> Option<NonNull<T>> {
    // SAFETY: `node` is live (the caller's promise), so `link` returns a pointer to its live
    // `Link`; `raw_get` reaches the cell's contents without forming a reference to the node, and
    // nothing writes them during the read.
    unsafe { *UnsafeCell::raw_get(&raw const (*T::link(node).as_ptr()).next) }
}

/// Store `next` as the link after `node`, through the node's projection.
///
/// # Safety
///
/// `node` points to a live node, and nothing else reads or writes its link during the call
/// (rule 3).
#[inline(always)]
unsafe fn write_link<T: Node>(node: NonNull<T>, next: Option<NonNull<T>>) {
    // SAFETY: as in `read_link`; the queue is the only writer of a link (rule 3).
    unsafe { *UnsafeCell::raw_get(&raw mut (*T::link(node).as_ptr()).next) = next }
}

/// Proof that one node is on no queue, and the only thing a queue will take.
///
/// Neither `Clone` nor `Copy`: [`Queue::push_back`] consumes it and [`Queue::pop_front`] hands it
/// back, so a node cannot be on two queues without `unsafe`. `#[repr(transparent)]` over its
/// [`NonNull`], so `Option<Unqueued<T>>` is one pointer wide. It cannot carry lifetime (rule 2),
/// which [`Unqueued::new`] discharges once. A dropped token strands its node.
#[repr(transparent)]
#[must_use = "a dropped token strands its node: nothing can queue it again without minting another"]
pub struct Unqueued<T>(NonNull<T>);

impl<T> Unqueued<T> {
    /// Mint the token for `node`. The one `unsafe` step in queuing.
    ///
    /// # Safety
    ///
    /// - `node` is valid to read and write, and stays valid for as long as this token, or any
    ///   queue it is pushed onto, can still reach it (rule 2).
    /// - `node` is on no queue, and no other `Unqueued` for it exists or will be minted while this
    ///   one, or the queue that consumes it, is alive (rule 1: this is what makes the token unique).
    /// - While a queue holds `node`, nothing but the queue writes its link, and no reference to
    ///   `node` is live across a queue operation (rule 3).
    /// - `node` is derived from the pointer that every later access to the node is derived from:
    ///   the one `Box::into_raw` returned, the pointer a table stores, or `&raw mut` on a local
    ///   that is not named again. It is never derived from a reference, such as
    ///   `NonNull::from(&mut *owner)` (rule 3). Under Rust's aliasing rules, a later `&mut`
    ///   reborrow of the node from its owner invalidates every pointer derived from an earlier
    ///   reference, so the queue's next use of a token minted that way is undefined behavior,
    ///   while a token derived from the owning pointer stays valid across such reborrows.
    #[inline(always)]
    pub const unsafe fn new(node: NonNull<T>) -> Self {
        Self(node)
    }

    /// The node, as a pointer. Safe to take: holding a pointer grants nothing, and dereferencing it
    /// is still the caller's `unsafe`.
    #[inline(always)]
    pub const fn as_non_null(&self) -> NonNull<T> {
        self.0
    }

    /// The node, as a raw pointer. See [`as_non_null`](Self::as_non_null).
    #[inline(always)]
    pub const fn as_ptr(&self) -> *mut T {
        self.0.as_ptr()
    }
}

impl<T> PartialEq<NonNull<T>> for Unqueued<T> {
    fn eq(&self, other: &NonNull<T>) -> bool {
        self.0 == *other
    }
}

impl<T> PartialEq for Unqueued<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T> Eq for Unqueued<T> {}

impl<T> PartialEq<Unqueued<T>> for NonNull<T> {
    fn eq(&self, other: &Unqueued<T>) -> bool {
        *self == other.0
    }
}

impl<T> core::fmt::Debug for Unqueued<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Unqueued").field(&self.0).finish()
    }
}

/// First in, first out: the queue is two pointers into nodes it does not own, plus a count.
///
/// Dropping a queue that still holds nodes strands them as dropping a token does: they stay linked,
/// no token for them exists, and queuing one again needs a fresh [`Unqueued::new`]. `Queue` has no
/// `Drop`, so pop every node before the queue goes.
///
/// # Examples
///
/// A run queue: threads leave in the order they arrived.
///
/// ```
/// # use core::ptr::NonNull;
/// # use intrusive_queue::{Link, Node, Queue, Unqueued};
/// # struct Thread { tid: u32, link: Link<Thread> }
/// # // SAFETY: plain field storage, which is the whole of the `Node` contract.
/// # unsafe impl Node for Thread {
/// #     unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>> {
/// #         // SAFETY: `this` is a live node (the caller's promise); this projects to its field.
/// #         unsafe { NonNull::new_unchecked(&raw mut (*this.as_ptr()).link) }
/// #     }
/// # }
/// let a = NonNull::new(Box::into_raw(Box::new(Thread { tid: 1, link: Link::new() }))).unwrap();
/// let b = NonNull::new(Box::into_raw(Box::new(Thread { tid: 2, link: Link::new() }))).unwrap();
/// let mut q: Queue<Thread> = Queue::new();
/// // SAFETY: live boxed nodes on no queue, minted from their owning pointers, freed below.
/// let (ta, tb) = unsafe { (Unqueued::new(a), Unqueued::new(b)) };
/// q.push_back(ta);
/// q.push_back(tb);
/// assert_eq!(q.len(), 2);
///
/// // Drained then refilled: where the classic tail-pointer bug lives.
/// let first = q.pop_front().unwrap();
/// q.push_back(first);
/// let order: Vec<u32> = core::iter::from_fn(|| q.pop_front())
///     // SAFETY: each token's node is one of the live locals above.
///     .map(|t| unsafe { (*t.as_ptr()).tid })
///     .collect();
/// assert_eq!(order, vec![2, 1]);
/// // SAFETY: both came from `Box::into_raw`, and `q` is empty, so nothing reaches them (rule 2).
/// unsafe { drop((Box::from_raw(a.as_ptr()), Box::from_raw(b.as_ptr()))) };
/// ```
#[doc(alias = "fifo")]
pub struct Queue<T> {
    head: Option<NonNull<T>>,
    tail: Option<NonNull<T>>,
    /// How many nodes the queue holds (`len`, the standard library's `LinkedList::len`).
    #[doc(alias = "length")]
    len: usize,
}

/// A `Queue<T>` is [`Send`] only when `T` is, as `LinkedList<T>` is. An `Rc` in a node keeps the
/// queue on its own thread:
///
/// ```compile_fail,E0277
/// # use core::ptr::NonNull;
/// # use std::rc::Rc;
/// # use intrusive_queue::{Link, Node, Queue};
/// struct Logged { log: Rc<u32>, link: Link<Logged> }
/// // SAFETY: plain field storage, which is the whole of the `Node` contract.
/// unsafe impl Node for Logged {
///     unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>> {
///         // SAFETY: `this` is a live node (the caller's promise); this projects to its field.
///         unsafe { NonNull::new_unchecked(&raw mut (*this.as_ptr()).link) }
///     }
/// }
/// let queue: Queue<Logged> = Queue::new();
/// std::thread::spawn(move || drop(queue));
/// ```
// SAFETY: the queue owns no data; it holds nodes only under the caller's contract (rule 3: all
// access serialized by the caller). Moving the queue moves access to its nodes with it, so the
// nodes must be `Send`, as for `LinkedList`.
unsafe impl<T: Send> Send for Queue<T> {}

/// A `Queue<T>` is [`Sync`] only when `T` is, as `LinkedList<T>` is. A shared queue offers
/// `is_empty`, `len`, `front` and `iter`, which read the queue and its links and write nothing. A
/// `Cell` in a node keeps a shared queue on its own thread:
///
/// ```compile_fail,E0277
/// # use core::cell::Cell;
/// # use core::ptr::NonNull;
/// # use intrusive_queue::{Link, Node, Queue};
/// struct Counted { hits: Cell<u32>, link: Link<Counted> }
/// // SAFETY: a projection to the link field, which is the whole of the `Node` contract.
/// unsafe impl Node for Counted {
///     unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>> {
///         // SAFETY: `this` is a live node (the caller's promise); this projects to its field.
///         unsafe { NonNull::new_unchecked(&raw mut (*this.as_ptr()).link) }
///     }
/// }
/// let queue: Queue<Counted> = Queue::new();
/// std::thread::scope(|s| {
///     s.spawn(|| queue.len());
/// });
/// ```
// SAFETY: through a shared reference the queue only reads: its own fields, and the links of its
// nodes in a walk. Two threads that read at once race on nothing, and no writer can run while the
// shared borrow lives, since every write takes `&mut self` and the caller writes no link (rule 3).
// The walk yields pointers to the nodes, which another thread may then read, so the nodes must be
// `Sync`, as for `LinkedList`.
unsafe impl<T: Sync> Sync for Queue<T> {}

impl<T> Queue<T> {
    /// An empty queue.
    pub const fn new() -> Self {
        Self {
            head: None,
            tail: None,
            len: 0,
        }
    }

    /// Whether the queue currently holds no nodes.
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    /// The number of nodes currently queued. O(1): maintained on every push and pop.
    pub fn len(&self) -> usize {
        self.len
    }

    /// The oldest node, as a pointer, left on the queue: the one the next
    /// [`pop_front`](Queue::pop_front) would take. Dereferencing it is the caller's `unsafe`, as
    /// for a pointer [`iter`](Queue::iter) yields.
    pub fn front(&self) -> Option<NonNull<T>> {
        self.head
    }
}

impl<T: Node> Queue<T> {
    /// Append a node, consuming its token. Safe: the token is the proof that the node is valid
    /// and on no queue, and taking it by value is what stops the same node being pushed twice.
    ///
    /// # Examples
    ///
    /// A token is not `Clone`, so pushing one node onto two queues does not compile:
    ///
    /// ```compile_fail,E0382
    /// # use core::ptr::NonNull;
    /// # use intrusive_queue::{Link, Node, Queue, Unqueued};
    /// # struct T { link: Link<T> }
    /// # unsafe impl Node for T {
    /// #     unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>> {
    /// #         // SAFETY: `this` is a live node (the caller's promise); this projects to its field.
    /// #         unsafe { NonNull::new_unchecked(&raw mut (*this.as_ptr()).link) }
    /// #     }
    /// # }
    /// # let mut b = T { link: Link::new() };
    /// # let (mut ready, mut inbox) = (Queue::<T>::new(), Queue::<T>::new());
    /// // SAFETY: `b` is live, on no queue, has no token yet, and is not named again.
    /// let token = unsafe { Unqueued::new(NonNull::new(&raw mut b).unwrap()) };
    /// ready.push_back(token);
    /// inbox.push_back(token); // error[E0382]: use of moved value: `token`
    /// ```
    pub fn push_back(&mut self, node: Unqueued<T>) {
        let node = node.0;
        // SAFETY: valid and on no queue, which is what `Unqueued::new`'s caller promised for this
        // token; and the token is consumed, so no other push can link this node until a pop.
        unsafe { write_link(node, None) };

        match self.tail {
            // SAFETY: `tail` was pushed earlier with its token and not yet popped, so it is valid.
            Some(tail) => unsafe { write_link(tail, Some(node)) },
            None => self.head = Some(node),
        }
        self.tail = Some(node);
        // Wrapping: a plain `+` carries an overflow check in any profile that enables them, on the
        // path every push takes. `len` counts distinct live linked nodes of nonzero size, so it
        // never reaches `usize::MAX`, and that check could never fire.
        self.len = self.len.wrapping_add(1);
    }

    /// Move every node of `other` to the back of this queue, keeping their order, and leave
    /// `other` empty. O(1): it writes the one link that joins the two, whatever their lengths.
    ///
    /// # Examples
    ///
    /// ```
    /// # use core::ptr::NonNull;
    /// # use intrusive_queue::{Link, Node, Queue, Unqueued};
    /// # struct Thread { id: u32, link: Link<Thread> }
    /// # // SAFETY: a projection to the link field, which is the whole of the `Node` contract.
    /// # unsafe impl Node for Thread {
    /// #     unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>> {
    /// #         // SAFETY: `this` is a live node (the caller's promise); this projects to its field.
    /// #         unsafe { NonNull::new_unchecked(&raw mut (*this.as_ptr()).link) }
    /// #     }
    /// # }
    /// let [a, b, c] = [1, 2, 3].map(|id| Box::into_raw(Box::new(Thread { id, link: Link::new() })));
    /// let [a, b, c] = [a, b, c].map(|p| NonNull::new(p).ok_or("null box"));
    /// let (a, b, c) = (a?, b?, c?);
    /// // SAFETY: live boxed nodes on no queue, minted from their owning pointers, freed below.
    /// let mut ready: Queue<Thread> = unsafe { [Unqueued::new(a)] }.into_iter().collect();
    /// // SAFETY: as above.
    /// let mut woken: Queue<Thread> = unsafe { [Unqueued::new(b), Unqueued::new(c)] }.into_iter().collect();
    ///
    /// // The threads a wake-up made runnable join the run queue behind those already there.
    /// ready.append(&mut woken);
    /// assert!(woken.is_empty() && ready.len() == 3);
    /// // SAFETY: the front is one of the live boxed nodes, read between queue operations.
    /// assert_eq!(unsafe { ready.front().ok_or("empty")?.as_ref() }.id, 1);
    /// while ready.pop_front().is_some() {}
    /// // SAFETY: all three came from `Box::into_raw`, and both queues are empty.
    /// unsafe { [a, b, c].map(|t| drop(Box::from_raw(t.as_ptr()))) };
    /// # Ok::<(), &str>(())
    /// ```
    pub fn append(&mut self, other: &mut Queue<T>) {
        let Some(other_head) = other.head else {
            return;
        };
        match self.tail {
            // SAFETY: `tail` was pushed with its token and not yet popped, so it is valid, and we
            // are the only accessor (rule 3).
            Some(tail) => unsafe { write_link(tail, Some(other_head)) },
            None => self.head = Some(other_head),
        }
        self.tail = other.tail;
        // Wrapping: the two counts are of distinct live nodes, so their sum is a count of distinct
        // live nodes too, which never reaches `usize::MAX`, as in `push_back`.
        self.len = self.len.wrapping_add(other.len);
        other.head = None;
        other.tail = None;
        other.len = 0;
    }

    /// Detach and return the oldest node, with its token. The returned node's link is cleared: it
    /// leaves the queue carrying no dangling reference into it, and the token says it may be
    /// queued again.
    pub fn pop_front(&mut self) -> Option<Unqueued<T>> {
        let node = self.head?;
        // SAFETY: every node between head and tail was pushed with a token and is still valid
        // (rule 2, the token's minting contract); we are the only accessor (rule 3).
        unsafe {
            self.head = read_link(node);
            if self.head.is_none() {
                self.tail = None;
            }
            write_link(node, None);
        }
        // Wrapping: `head` was `Some`, so `len >= 1`. The Kani proof below holds `len` to the
        // model's count after every step, so a wrapped decrement would fail it.
        self.len = self.len.wrapping_sub(1);
        Some(Unqueued(node))
    }

    /// Take `victim` out of the queue wherever it stands, with its token, or `None` if the queue
    /// does not hold it. The nodes around it keep their order.
    ///
    /// O(n) from the head: the walk reads each node's link until it meets `victim`, then writes
    /// the one link that bridges the gap. `victim` is only compared, never read, unless the queue
    /// holds it, so any pointer is safe to pass.
    ///
    /// # Examples
    ///
    /// ```
    /// # use core::ptr::NonNull;
    /// # use intrusive_queue::{Link, Node, Queue, Unqueued};
    /// # struct Thread { link: Link<Thread> }
    /// # // SAFETY: plain field storage, which is the whole of the `Node` contract.
    /// # unsafe impl Node for Thread {
    /// #     unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>> {
    /// #         // SAFETY: `this` is a live node (the caller's promise); this projects to its field.
    /// #         unsafe { NonNull::new_unchecked(&raw mut (*this.as_ptr()).link) }
    /// #     }
    /// # }
    /// let [a, b, c] = [(); 3].map(|()| NonNull::new(Box::into_raw(Box::new(Thread { link: Link::new() }))).unwrap());
    /// let mut waiting: Queue<Thread> = Queue::new();
    /// // SAFETY: live boxed nodes on no queue, minted from their owning pointers, freed below.
    /// unsafe { [a, b, c].map(|t| waiting.push_back(Unqueued::new(t))) };
    ///
    /// // A thread whose wait ends early leaves from the middle.
    /// assert!(waiting.remove(b).is_some_and(|token| token == b));
    /// assert!(waiting.remove(b).is_none());
    /// assert!(waiting.pop_front().is_some_and(|token| token == a));
    /// assert!(waiting.pop_front().is_some_and(|token| token == c));
    /// // SAFETY: all three came from `Box::into_raw`, and the queue is empty.
    /// unsafe { [a, b, c].map(|t| drop(Box::from_raw(t.as_ptr()))) };
    /// ```
    pub fn remove(&mut self, victim: NonNull<T>) -> Option<Unqueued<T>> {
        let mut previous = None;
        let mut cursor = self.head;
        while let Some(node) = cursor {
            if node == victim {
                // SAFETY: `node` is on this queue, reached from the head, and `previous` is the
                // node the walk passed just before it.
                return Some(unsafe { self.unlink(previous, node) });
            }
            previous = Some(node);
            // SAFETY: every node between head and tail was pushed with a token and is still valid
            // (rule 2); we are the only accessor (rule 3).
            cursor = unsafe { read_link(node) };
        }
        None
    }

    /// Take every node `filter` picks out of the queue, oldest first, and hand them back as a
    /// queue of their own. The nodes `filter` passes over stay, in their order.
    ///
    /// `filter` sees each node once, in queue order, as a pointer, so it can count as well as test.
    /// Reading the node is the caller's `unsafe`; replacing its `Link` breaks rule 3. If
    /// `filter` panics, the nodes already picked are stranded, as a dropped queue strands its own.
    ///
    /// Unlike `Vec::extract_if`, which returns a lazy iterator borrowing the vector, this walks the
    /// whole queue at once and returns a `Queue`, so the caller holds no borrow of this one.
    ///
    /// # Examples
    ///
    /// ```
    /// # use core::ptr::NonNull;
    /// # use intrusive_queue::{Link, Node, Queue, Unqueued};
    /// # struct Waiter { key: u32, link: Link<Waiter> }
    /// # // SAFETY: plain field storage, which is the whole of the `Node` contract.
    /// # unsafe impl Node for Waiter {
    /// #     unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>> {
    /// #         // SAFETY: `this` is a live node (the caller's promise); this projects to its field.
    /// #         unsafe { NonNull::new_unchecked(&raw mut (*this.as_ptr()).link) }
    /// #     }
    /// # }
    /// let waiters = [7, 9, 7, 7].map(|key| NonNull::new(Box::into_raw(Box::new(Waiter { key, link: Link::new() }))).unwrap());
    /// let mut bucket: Queue<Waiter> = Queue::new();
    /// // SAFETY: live boxed nodes on no queue, minted from their owning pointers, freed below.
    /// unsafe { waiters.map(|w| bucket.push_back(Unqueued::new(w))) };
    ///
    /// // Wake at most two of the waiters under key 7, first parked first.
    /// let mut left = 2;
    /// // SAFETY: each pointer names one of the live boxed nodes, read between queue operations.
    /// let mut woken = bucket.extract_if(|w| left > 0 && unsafe { (*w.as_ptr()).key } == 7 && { left -= 1; true });
    /// assert!(woken.pop_front().is_some_and(|t| t == waiters[0]));
    /// assert!(woken.pop_front().is_some_and(|t| t == waiters[2]));
    /// assert!(woken.is_empty());
    /// assert!(bucket.pop_front().is_some_and(|t| t == waiters[1]));
    /// assert!(bucket.pop_front().is_some_and(|t| t == waiters[3]));
    /// // SAFETY: all four came from `Box::into_raw`, and both queues are empty.
    /// unsafe { waiters.map(|w| drop(Box::from_raw(w.as_ptr()))) };
    /// ```
    pub fn extract_if(&mut self, mut filter: impl FnMut(NonNull<T>) -> bool) -> Queue<T> {
        let mut extracted = Queue::new();
        let mut previous = None;
        let mut cursor = self.head;
        while let Some(node) = cursor {
            // SAFETY: as in `remove`. The link is read before `node` can leave, because leaving
            // clears it.
            cursor = unsafe { read_link(node) };
            if filter(node) {
                // SAFETY: `node` is on this queue, and `previous` is the last node the walk kept,
                // which is the node just before it.
                extracted.push_back(unsafe { self.unlink(previous, node) });
            } else {
                previous = Some(node);
            }
        }
        extracted
    }

    /// Unlink `node` and hand back its token, with its link cleared as a pop clears it.
    ///
    /// # Safety
    ///
    /// `node` is on this queue, and `previous` is the node just before it, or `None` when `node`
    /// is the head.
    unsafe fn unlink(&mut self, previous: Option<NonNull<T>>, node: NonNull<T>) -> Unqueued<T> {
        // SAFETY: both are on this queue (the caller's promise), so both are valid (rule 2), and
        // we are the only accessor (rule 3).
        unsafe {
            let next = read_link(node);
            match previous {
                Some(previous) => write_link(previous, next),
                None => self.head = next,
            }
            if next.is_none() {
                self.tail = previous;
            }
            write_link(node, None);
        }
        // Wrapping: `node` was on the queue, so `len >= 1`, as in `pop_front`.
        self.len = self.len.wrapping_sub(1);
        Unqueued(node)
    }

    /// The queued nodes, oldest first, as pointers. The walk reads each node's link and nothing
    /// else, and the shared borrow of the queue stops a push or a pop while it runs, so a caller
    /// can count or find nodes without taking them off. Dereferencing a pointer it yields is still
    /// the caller's `unsafe`, under the same rules as a token's.
    ///
    /// # Examples
    ///
    /// Counting the queued threads that match a test, with the queue left as it was:
    ///
    /// ```
    /// # use core::ptr::NonNull;
    /// # use intrusive_queue::{Link, Node, Queue, Unqueued};
    /// # struct Thread { id: u32, link: Link<Thread> }
    /// # // SAFETY: plain field storage, which is the whole of the `Node` contract.
    /// # unsafe impl Node for Thread {
    /// #     unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>> {
    /// #         // SAFETY: `this` is a live node (the caller's promise); this projects to its field.
    /// #         unsafe { NonNull::new_unchecked(&raw mut (*this.as_ptr()).link) }
    /// #     }
    /// # }
    /// let a = NonNull::new(Box::into_raw(Box::new(Thread { id: 1, link: Link::new() }))).unwrap();
    /// let b = NonNull::new(Box::into_raw(Box::new(Thread { id: 2, link: Link::new() }))).unwrap();
    /// let mut ready: Queue<Thread> = Queue::new();
    /// // SAFETY: live boxed nodes on no queue, minted from their owning pointers, freed below.
    /// let (token_a, token_b) = unsafe { (Unqueued::new(a), Unqueued::new(b)) };
    /// ready.push_back(token_a);
    /// ready.push_back(token_b);
    /// // SAFETY: each pointer names one of the two live boxed nodes.
    /// let even = ready.iter().filter(|node| unsafe { node.as_ref() }.id % 2 == 0).count();
    /// assert_eq!((even, ready.len()), (1, 2));
    /// while ready.pop_front().is_some() {}
    /// // SAFETY: both came from `Box::into_raw`, and the queue is empty, so nothing reaches them.
    /// unsafe { drop((Box::from_raw(a.as_ptr()), Box::from_raw(b.as_ptr()))) };
    /// ```
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            next: self.head,
            len: self.len,
            queue: core::marker::PhantomData,
        }
    }
}

/// The walk [`Queue::iter`] returns: each queued node's pointer, oldest first.
///
/// It knows how many nodes it has left, from the queue's count, so it is an
/// [`ExactSizeIterator`]. A clone walks the rest of the queue again from where this walk stands,
/// and printing one shows that place and the count, reading no node.
pub struct Iter<'a, T> {
    next: Option<NonNull<T>>,
    /// How many nodes the walk has yet to yield.
    len: usize,
    queue: core::marker::PhantomData<&'a Queue<T>>,
}

impl<T: Node> Iterator for Iter<'_, T> {
    type Item = NonNull<T>;

    fn next(&mut self) -> Option<NonNull<T>> {
        let node = self.next?;
        // SAFETY: every node between head and tail was pushed with a token and is still valid
        // (rule 2), and the borrow of the queue this walk holds means no push or pop links or
        // unlinks one while it runs; the caller writes no link outside the queue (rule 3).
        self.next = unsafe { read_link(node) };
        // Wrapping: `next` was `Some`, so at least one node was left and `len >= 1`, as in
        // `pop_front`.
        self.len = self.len.wrapping_sub(1);
        Some(node)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

impl<T: Node> ExactSizeIterator for Iter<'_, T> {}

impl<T: Node> core::iter::FusedIterator for Iter<'_, T> {}

impl<T> Clone for Iter<'_, T> {
    fn clone(&self) -> Self {
        Self {
            next: self.next,
            len: self.len,
            queue: core::marker::PhantomData,
        }
    }
}

impl<T> core::fmt::Debug for Iter<'_, T> {
    /// Prints the next node's pointer and how many are left, and walks nothing, as `Queue`'s
    /// `Debug` does.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Iter")
            .field("next", &self.next)
            .field("len", &self.len)
            .finish()
    }
}

impl<T> core::fmt::Debug for Queue<T> {
    /// Prints the two end pointers and the count, and walks nothing: a walk would read nodes,
    /// which only the caller may do (rule 3).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Queue")
            .field("head", &self.head)
            .field("tail", &self.tail)
            .field("len", &self.len)
            .finish()
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Node> Extend<Unqueued<T>> for Queue<T> {
    /// Push each token in turn, as [`push_back`](Queue::push_back) does.
    fn extend<I: IntoIterator<Item = Unqueued<T>>>(&mut self, tokens: I) {
        for token in tokens {
            self.push_back(token);
        }
    }
}

impl<T: Node> FromIterator<Unqueued<T>> for Queue<T> {
    /// A queue of the tokens' nodes, in the order the iterator yields them.
    fn from_iter<I: IntoIterator<Item = Unqueued<T>>>(tokens: I) -> Self {
        let mut queue = Self::new();
        queue.extend(tokens);
        queue
    }
}

impl<'a, T: Node> IntoIterator for &'a Queue<T> {
    type Item = NonNull<T>;
    type IntoIter = Iter<'a, T>;

    /// The same walk as [`Queue::iter`].
    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

/// The one test node the host tests and the proof harnesses share.
#[cfg(any(test, kani))]
mod test_support {
    use super::{Link, Node, NonNull};

    /// A node whose `tag` says which one it is.
    pub(crate) struct N {
        pub(crate) link: Link<N>,
        pub(crate) tag: usize,
    }

    impl N {
        /// A node with `tag`, on no queue.
        pub(crate) fn new(tag: usize) -> Self {
            Self {
                link: Link::new(),
                tag,
            }
        }
    }

    // SAFETY: `link` projects to the `link` field by `&raw mut`, forms no reference and does
    // nothing else, which is the whole of the `Node` contract.
    unsafe impl Node for N {
        unsafe fn link(this: NonNull<Self>) -> NonNull<Link<Self>> {
            // SAFETY: `this` is a live node (the caller's promise), and a field of a non-null
            // pointer to a live value is non-null.
            unsafe { NonNull::new_unchecked(&raw mut (*this.as_ptr()).link) }
        }
    }
}

/// Machine-checked proofs. The harness drives the real `Queue` with a symbolic operation sequence
/// against a trivially-correct model (an array and two counters), covering every push/pop pattern
/// up to the bound, the regrowth-from-empty transitions included.
#[cfg(kani)]
mod verification {
    use super::test_support::N;
    use super::*;

    /// Falsification: replayable `falsifications/verification.any_push_pop_interleaving_is_first_in_first_out_and_lossless.patch`
    /// First in, first out order, no loss, no invention, across every operation sequence. Six symbolic
    /// steps over three nodes: each step pushes some not-currently-queued node or pops. The
    /// model records the order tags went in; the queue must hand them back in exactly that
    /// order, agree about emptiness and length at every step, and never dereference a stale
    /// link (which Kani would report as an invalid pointer access).
    #[kani::proof]
    fn any_push_pop_interleaving_is_first_in_first_out_and_lossless() {
        let mut nodes = [N::new(0), N::new(1), N::new(2)];
        // One token per node, minted from one pointer to the array, the way a kernel mints
        // from the pointer its table stores. `nodes` is not named again, so every later access
        // goes through a token. A token is in `held` while its node is off the queue.
        let base = (&raw mut nodes).cast::<N>();
        // SAFETY: three distinct live elements of a local declared before `q`, each on no queue,
        // each minted once from the array's own pointer, and the harness is the only accessor.
        let mut held: [Option<Unqueued<N>>; 3] = unsafe {
            [
                Some(Unqueued::new(NonNull::new_unchecked(base))),
                Some(Unqueued::new(NonNull::new_unchecked(base.add(1)))),
                Some(Unqueued::new(NonNull::new_unchecked(base.add(2)))),
            ]
        };

        let mut q: Queue<N> = Queue::new();

        // The model: a ring of tags in push order, and which nodes are currently queued.
        let mut model = [usize::MAX; 6];
        let (mut m_head, mut m_tail) = (0usize, 0usize);
        let mut queued = [false; 3];

        for _ in 0..6 {
            let choice: usize = kani::any();
            kani::assume(choice <= 3);
            if choice < 3 {
                // The token is the check now: a node is pushed exactly when its token is held.
                if let Some(token) = held[choice].take() {
                    assert!(!queued[choice], "a token was held for a queued node");
                    queued[choice] = true;
                    model[m_tail] = choice;
                    m_tail += 1;
                    q.push_back(token);
                }
            } else {
                let popped = q.pop_front();
                if m_head == m_tail {
                    assert!(popped.is_none(), "popped from an empty queue");
                } else {
                    let expect = model[m_head];
                    m_head += 1;
                    queued[expect] = false;
                    let token = popped.expect("lost a node");
                    // SAFETY: pop returns only nodes we pushed, all still valid.
                    let got = unsafe { (*token.as_ptr()).tag };
                    assert_eq!(got, expect, "not FIFO");
                    held[expect] = Some(token);
                }
            }
            assert_eq!(q.len(), m_tail - m_head);
            assert_eq!(q.is_empty(), m_head == m_tail);
        }
    }

    /// Falsification: replayable `falsifications/verification.a_popped_node_leaves_with_a_clear_link.patch`
    /// A popped node leaves with a clear link, whichever node it was and however many were
    /// queued behind it: the queue's order stays right without that write, so the first in, first out
    /// harness cannot see it go.
    #[kani::proof]
    #[kani::unwind(4)]
    fn a_popped_node_leaves_with_a_clear_link() {
        let mut nodes = [N::new(0), N::new(1), N::new(2)];
        let base = (&raw mut nodes).cast::<N>();
        let mut q: Queue<N> = Queue::new();
        let count: usize = kani::any();
        kani::assume((1..=3).contains(&count));
        for i in 0..count {
            // SAFETY: a distinct live element of a local declared before `q`, on no queue, minted
            // once from the array's own pointer, and the harness is the only accessor.
            q.push_back(unsafe { Unqueued::new(NonNull::new_unchecked(base.add(i))) });
        }
        let token = q.pop_front().expect("lost a node");
        // SAFETY: pop returns only nodes we pushed, all still valid.
        assert!(unsafe { read_link(token.as_non_null()) }.is_none());
    }

    /// Four nodes for the removal harnesses, the last never queued before the call under proof.
    fn four_nodes() -> [N; 4] {
        [0, 1, 2, 3].map(N::new)
    }

    /// Falsification: replayable `falsifications/verification.removing_any_node_keeps_the_rest_in_order.patch`
    /// Removing any node, or one the queue does not hold, from a queue of up to three takes out
    /// exactly that node with a clear link, keeps the rest in order, and leaves the tail right: a
    /// node pushed after the removal comes out last.
    #[kani::proof]
    #[kani::unwind(6)]
    fn removing_any_node_keeps_the_rest_in_order() {
        let mut nodes = four_nodes();
        let base = (&raw mut nodes).cast::<N>();
        let mut q: Queue<N> = Queue::new();
        let count: usize = kani::any();
        kani::assume(count <= 3);
        for i in 0..count {
            // SAFETY: a distinct live element of a local declared before `q`, on no queue, minted
            // once from the array's own pointer, and the harness is the only accessor.
            q.push_back(unsafe { Unqueued::new(NonNull::new_unchecked(base.add(i))) });
        }
        let victim: usize = kani::any();
        kani::assume(victim <= 3);
        // SAFETY: an element of the array, which outlives the call.
        let removed = q.remove(unsafe { NonNull::new_unchecked(base.add(victim)) });
        let was_queued = victim < count;
        assert_eq!(removed.is_some(), was_queued);
        if let Some(token) = &removed {
            // SAFETY: a node of the array, just taken off the queue.
            assert!(token.as_ptr() == unsafe { base.add(victim) });
            // SAFETY: as above.
            assert!(unsafe { read_link(token.as_non_null()) }.is_none());
        }
        assert_eq!(q.len(), count - usize::from(was_queued));
        // SAFETY: node 3 was never pushed and has no token yet.
        q.push_back(unsafe { Unqueued::new(NonNull::new_unchecked(base.add(3))) });
        for expect in (0..count).filter(|&i| i != victim).chain([3]) {
            let token = q.pop_front().expect("lost a node");
            // SAFETY: pop returns only nodes we pushed, all still valid.
            assert_eq!(unsafe { (*token.as_ptr()).tag }, expect);
        }
        assert!(q.is_empty());
    }

    /// Falsification: replayable `falsifications/verification.appending_joins_two_queues_in_order.patch`
    /// Appending a queue of up to two nodes to another of up to two leaves every node on the first,
    /// its own first and the appended ones after, in order, and the second empty; a node pushed
    /// onto either afterwards comes out of that one alone.
    #[kani::proof]
    #[kani::unwind(6)]
    fn appending_joins_two_queues_in_order() {
        let mut nodes = [0, 1, 2, 3, 4].map(N::new);
        let base = (&raw mut nodes).cast::<N>();
        let (mut q, mut r): (Queue<N>, Queue<N>) = (Queue::new(), Queue::new());
        let (in_q, in_r): (usize, usize) = (kani::any(), kani::any());
        kani::assume(in_q <= 2 && in_r <= 2);
        for i in 0..in_q {
            // SAFETY: a distinct live element of a local declared before the queues, on no
            // queue, minted once from the array's own pointer, and the harness is the only
            // accessor.
            q.push_back(unsafe { Unqueued::new(NonNull::new_unchecked(base.add(i))) });
        }
        for i in 2..2 + in_r {
            // SAFETY: as above.
            r.push_back(unsafe { Unqueued::new(NonNull::new_unchecked(base.add(i))) });
        }
        q.append(&mut r);
        assert_eq!((q.len(), r.len()), (in_q + in_r, 0));
        assert!(r.is_empty() && r.front().is_none());
        let onto_r: bool = kani::any();
        let target = if onto_r { &mut r } else { &mut q };
        // SAFETY: node 4 was never pushed and has no token yet.
        target.push_back(unsafe { Unqueued::new(NonNull::new_unchecked(base.add(4))) });
        let mut expect = (0..in_q)
            .chain(2..2 + in_r)
            .chain(if onto_r { None } else { Some(4) });
        while let Some(token) = q.pop_front() {
            // SAFETY: pop returns only nodes we pushed, all still valid.
            assert_eq!(Some(unsafe { (*token.as_ptr()).tag }), expect.next());
        }
        assert!(expect.next().is_none(), "lost a node");
        let tail_of_r = r.pop_front();
        assert_eq!(tail_of_r.is_some(), onto_r);
        assert!(r.is_empty());
    }

    /// Falsification: replayable `falsifications/verification.extract_if_splits_the_queue_in_order.patch`
    /// `extract_if` with a filter that picks any subset of up to three nodes shows the filter each
    /// node once in queue order, hands the picked ones back in that order, keeps the rest in
    /// theirs, and leaves both tails right: a node pushed onto either queue afterwards comes out
    /// last.
    #[kani::proof]
    #[kani::unwind(6)]
    fn extract_if_splits_the_queue_in_order() {
        let mut nodes = four_nodes();
        let base = (&raw mut nodes).cast::<N>();
        let mut q: Queue<N> = Queue::new();
        let count: usize = kani::any();
        kani::assume(count <= 3);
        for i in 0..count {
            // SAFETY: as in the harness above.
            q.push_back(unsafe { Unqueued::new(NonNull::new_unchecked(base.add(i))) });
        }
        let mut picked = [false; 3];
        let mut seen = 0;
        let mut extracted = q.extract_if(|node| {
            // SAFETY: the queue hands the filter only nodes of the array.
            let tag = unsafe { (*node.as_ptr()).tag };
            assert_eq!(tag, seen, "the filter saw a node out of order");
            seen += 1;
            picked[tag] = kani::any();
            picked[tag]
        });
        assert_eq!(seen, count, "the filter missed a node");
        let taken = picked.iter().filter(|&&p| p).count();
        assert_eq!(extracted.len(), taken);
        assert_eq!(q.len(), count - taken);
        let target = if kani::any() { &mut q } else { &mut extracted };
        // SAFETY: node 3 was never pushed and has no token yet.
        target.push_back(unsafe { Unqueued::new(NonNull::new_unchecked(base.add(3))) });
        let mut three_seen = 0;
        for (queue, keep) in [(&mut extracted, true), (&mut q, false)] {
            let mut expect = (0..count).filter(|&i| picked[i] == keep);
            while let Some(token) = queue.pop_front() {
                // SAFETY: pop returns only nodes we pushed, all still valid.
                let (tag, next) =
                    unsafe { ((*token.as_ptr()).tag, read_link(token.as_non_null())) };
                assert!(next.is_none());
                if tag != 3 {
                    assert_eq!(Some(tag), expect.next(), "a node out of order or invented");
                } else {
                    three_seen += 1;
                    assert!(queue.is_empty(), "node 3 was not last");
                }
            }
            assert!(expect.next().is_none(), "lost a node");
        }
        assert_eq!(three_seen, 1, "lost the node pushed afterwards");
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::N;
    use super::*;

    /// A test's node, owned through the pointer `Box::into_raw` returned, as a kernel owns a
    /// thread through the pointer its table stores. Every access, the token's and the test's,
    /// derives from that one pointer, and dropping it frees the node.
    struct Owned(NonNull<N>);

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the pointer came from `Box::into_raw` in `node`, and is freed only here.
            drop(unsafe { Box::from_raw(self.0.as_ptr()) });
        }
    }

    impl core::ops::Deref for Owned {
        type Target = N;
        fn deref(&self) -> &N {
            // SAFETY: live until `drop`; the reference is the test's, taken between queue
            // operations (rule 3).
            unsafe { self.0.as_ref() }
        }
    }

    impl core::ops::DerefMut for Owned {
        fn deref_mut(&mut self) -> &mut N {
            // SAFETY: as `deref`.
            unsafe { self.0.as_mut() }
        }
    }

    fn node(tag: usize) -> Owned {
        Owned(NonNull::new(Box::into_raw(Box::new(N::new(tag)))).unwrap())
    }

    /// Mint `n`'s token from its owning pointer. Every test calls this once per node, declared
    /// before its queue, so the node outlives the queue (locals drop in reverse) and is on no
    /// queue yet.
    fn token(n: &mut Owned) -> Unqueued<N> {
        // SAFETY: a live boxed node on no queue, minted once per test (see above), from the
        // pointer every other access derives from.
        unsafe { Unqueued::new(n.0) }
    }

    fn tag(t: &Unqueued<N>) -> usize {
        // SAFETY: every token in these tests names one of the test's live boxed nodes.
        unsafe { (*t.as_ptr()).tag }
    }

    #[test]
    fn queue_order() {
        let (mut a, mut b, mut c) = (node(1), node(2), node(3));
        let mut q: Queue<N> = Queue::new();
        assert!(q.is_empty());

        q.push_back(token(&mut a));
        q.push_back(token(&mut b));
        q.push_back(token(&mut c));
        assert_eq!(q.len(), 3);

        // A non-empty queue says so; a body stuck at `true` passes every empty-state check.
        assert!(!q.is_empty());

        // len falls by exactly one per pop; the scheduler's run-queue accounting trusts it.
        let first = q.pop_front().map(|t| tag(&t));
        assert_eq!(first, Some(1));
        assert_eq!(q.len(), 2);

        let tags: Vec<usize> = core::iter::from_fn(|| q.pop_front())
            .map(|t| tag(&t))
            .collect();
        assert_eq!(tags, vec![2, 3]);
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    /// The empty-again transitions: a queue drained to empty accepts new nodes correctly (the
    /// classic tail-pointer bug), and a popped node can be pushed again with the token its pop
    /// handed back.
    #[test]
    fn drain_then_reuse() {
        let (mut a, mut b) = (node(1), node(2));
        let mut q: Queue<N> = Queue::new();

        q.push_back(token(&mut a));
        let ta = q.pop_front().unwrap();
        assert_eq!(tag(&ta), 1);
        assert!(q.pop_front().is_none());

        q.push_back(token(&mut b));
        q.push_back(ta); // the token the pop handed back is what makes queuing `a` again legal
        assert_eq!(q.pop_front().map(|t| tag(&t)), Some(2));
        assert_eq!(q.pop_front().map(|t| tag(&t)), Some(1));
        assert!(q.is_empty());
    }

    /// The token is the pointer and nothing more: `#[repr(transparent)]` is the zero-cost claim a
    /// caller's footprint rests on, and `Option` of one is still one word.
    #[test]
    fn a_token_is_one_pointer_wide() {
        use core::mem::size_of;
        assert_eq!(size_of::<Unqueued<N>>(), size_of::<NonNull<N>>());
        assert_eq!(size_of::<Option<Unqueued<N>>>(), size_of::<NonNull<N>>());
    }

    /// A token compares equal to its node's pointer and to no other, and prints as that pointer,
    /// which is what lets a caller assert which node a pop handed back.
    #[test]
    fn a_token_is_its_nodes_pointer() {
        let (mut a, mut b) = (node(1), node(2));
        let pb = NonNull::from(&mut *b);
        let ta = token(&mut a);
        assert!(ta == NonNull::from(&mut *a));
        assert!(!(ta == pb));
        assert_eq!(
            format!("{ta:?}"),
            format!("Unqueued({:?})", ta.as_non_null())
        );
    }

    /// Two tokens for one node compare equal and for two nodes unequal, a node's pointer compares
    /// with a token from either side, and a queue prints its ends and count without walking.
    #[test]
    fn tokens_compare_each_way_and_a_queue_prints_its_ends() {
        let (mut a, mut b) = (node(1), node(2));
        let (pointer_a, pointer_b) = (a.0, b.0);
        let (first, second) = (token(&mut a), token(&mut b));
        assert!(first == first && first != second);
        assert!(pointer_a == first && !(pointer_a == second) && pointer_b == second);
        let mut q: Queue<N> = Queue::new();
        assert_eq!(format!("{q:?}"), "Queue { head: None, tail: None, len: 0 }");
        q.push_back(first);
        assert_eq!(
            format!("{q:?}"),
            format!("Queue {{ head: Some({pointer_a:?}), tail: Some({pointer_a:?}), len: 1 }}")
        );
        q.push_back(second);
    }

    /// A walk yields the queued nodes oldest first and changes nothing: an empty queue yields
    /// none, a walk after a pop starts at the new head, and the queue pops in the same order after.
    #[test]
    fn a_walk_yields_the_queue_in_order_and_leaves_it() {
        let (mut a, mut b, mut c) = (node(1), node(2), node(3));
        let mut q: Queue<N> = Queue::new();
        assert_eq!(q.iter().next(), None);
        q.push_back(token(&mut a));
        q.push_back(token(&mut b));
        q.push_back(token(&mut c));
        let walk = |q: &Queue<N>| -> Vec<usize> {
            q.iter()
                // SAFETY: every pointer the walk yields names one of the test's live boxed nodes.
                .map(|p| unsafe { p.as_ref() }.tag)
                .collect()
        };
        assert_eq!(walk(&q), vec![1, 2, 3]);
        assert_eq!(q.iter().next(), Some(a.0));
        let first = q.pop_front().unwrap();
        assert_eq!(walk(&q), vec![2, 3]);
        q.push_back(first);
        assert_eq!(walk(&q), vec![2, 3, 1]);
        let mut iter = q.iter();
        assert_eq!(iter.by_ref().count(), 3);
        assert_eq!(iter.next(), None);
        let tags: Vec<usize> = core::iter::from_fn(|| q.pop_front())
            .map(|t| tag(&t))
            .collect();
        assert_eq!(tags, vec![2, 3, 1]);
    }

    /// A walk knows how many nodes it has left at every step, a clone walks the rest again from
    /// the same place, and a walk prints its place and count.
    #[test]
    fn a_walk_counts_down_clones_and_prints() {
        let (mut a, mut b, mut c) = (node(1), node(2), node(3));
        let mut q: Queue<N> = Queue::new();
        assert_eq!(q.iter().size_hint(), (0, Some(0)));
        q.push_back(token(&mut a));
        q.push_back(token(&mut b));
        q.push_back(token(&mut c));
        let mut walk = q.iter();
        for left in (0..=3).rev() {
            assert_eq!((walk.len(), walk.size_hint()), (left, (left, Some(left))));
            walk.next();
        }
        let mut walk = q.iter();
        assert_eq!(walk.next(), Some(a.0));
        let again = walk.clone();
        assert_eq!(walk.collect::<Vec<_>>(), vec![b.0, c.0]);
        assert_eq!(again.len(), 2);
        assert_eq!(again.collect::<Vec<_>>(), vec![b.0, c.0]);
        let walk = q.iter();
        assert_eq!(
            format!("{walk:?}"),
            format!("Iter {{ next: Some({:?}), len: 3 }}", a.0)
        );
        assert_eq!(drain(&mut q), vec![1, 2, 3]);
    }

    /// A queue collects from tokens in order, extends with more behind them, and a `for` loop
    /// over a borrowed queue walks it as `iter` does.
    #[test]
    fn a_queue_collects_extends_and_loops() {
        let mut nodes: Vec<Owned> = (0..4).map(node).collect();
        let mut tokens: Vec<Unqueued<N>> = nodes.iter_mut().map(token).collect();
        let rest = tokens.split_off(2);
        let mut q: Queue<N> = tokens.into_iter().collect();
        assert_eq!(q.len(), 2);
        q.extend(rest);
        assert_eq!(q.len(), 4);
        let mut walked = Vec::new();
        for pointer in &q {
            walked.push(pointer);
        }
        assert_eq!(walked, nodes.iter().map(|n| n.0).collect::<Vec<_>>());
        assert_eq!(drain(&mut q), vec![0, 1, 2, 3]);
    }

    /// `front` names the next node a pop takes; `append` joins two queues in order, empty or not,
    /// leaves the second empty, and both tails take a later push.
    #[test]
    fn front_and_append() {
        for (in_q, in_r) in [(0, 0), (0, 2), (2, 0), (2, 2)] {
            let mut nodes: Vec<Owned> = (0..6).map(node).collect();
            let mut tokens: Vec<Unqueued<N>> = nodes.iter_mut().map(token).collect();
            let (extra_r, extra_q) = (tokens.pop().unwrap(), tokens.pop().unwrap());
            let mut r: Queue<N> = tokens.drain(2..2 + in_r).collect();
            let mut q: Queue<N> = tokens.drain(..in_q).collect();
            assert_eq!(q.front(), (in_q > 0).then(|| nodes[0].0));
            q.append(&mut r);
            assert_eq!((q.len(), r.len(), r.front()), (in_q + in_r, 0, None));
            let first = if in_q > 0 { 0 } else { 2 };
            assert_eq!(q.front(), (in_q + in_r > 0).then(|| nodes[first].0));
            q.push_back(extra_q);
            r.push_back(extra_r);
            let expect: Vec<usize> = (0..in_q).chain(2..2 + in_r).chain([4]).collect();
            assert_eq!(drain(&mut q), expect);
            assert_eq!(drain(&mut r), vec![5]);
        }
    }

    /// A queue of `Sync` nodes is `Sync`: two threads may walk one shared queue at once.
    #[test]
    fn a_shared_queue_walks_from_two_threads() {
        fn assert_sync<S: Sync>(_: &S) {}
        let (mut a, mut b) = (node(1), node(2));
        let mut q: Queue<N> = Queue::new();
        q.push_back(token(&mut a));
        q.push_back(token(&mut b));
        assert_sync(&q);
        std::thread::scope(|s| {
            let walks = [s.spawn(|| q.iter().count()), s.spawn(|| q.iter().count())];
            for walk in walks {
                assert_eq!(walk.join().ok(), Some(2));
            }
        });
        assert_eq!(drain(&mut q), vec![1, 2]);
    }

    /// A popped node leaves with a clean link: it does not secretly point back into the queue.
    #[test]
    fn a_popped_node_carries_no_link() {
        let (mut a, mut b) = (node(1), node(2));
        let mut q: Queue<N> = Queue::new();
        q.push_back(token(&mut a));
        q.push_back(token(&mut b));
        let p = q.pop_front().unwrap();
        // SAFETY: `p` was just popped, so it is live and unlinked; that it is unlinked is what this test asserts.
        assert!(unsafe { read_link(p.as_non_null()) }.is_none());
    }

    /// Drain `q` into its nodes' tags, oldest first.
    fn drain(q: &mut Queue<N>) -> Vec<usize> {
        core::iter::from_fn(|| q.pop_front())
            .map(|t| tag(&t))
            .collect()
    }

    /// `remove` takes the head, a middle node and the tail, refuses a node the queue does not
    /// hold, and leaves the tail right after each, so a later push comes out last.
    #[test]
    fn remove_takes_any_node_and_keeps_the_tail() {
        for victim in 0..4 {
            let mut nodes: Vec<Owned> = (0..5).map(node).collect();
            let pointers: Vec<NonNull<N>> = nodes.iter().map(|n| n.0).collect();
            let mut q: Queue<N> = Queue::new();
            for n in &mut nodes[..3] {
                q.push_back(token(n));
            }
            let removed = q.remove(pointers[victim]);
            assert_eq!(removed.is_some(), victim < 3);
            if let Some(t) = &removed {
                assert!(*t == pointers[victim]);
                // SAFETY: just taken off the queue, and its node is live.
                assert!(unsafe { read_link(t.as_non_null()) }.is_none());
            }
            assert_eq!(q.len(), if victim < 3 { 2 } else { 3 });
            q.push_back(token(&mut nodes[4]));
            let expect: Vec<usize> = (0..3).filter(|&i| i != victim).chain([4]).collect();
            assert_eq!(drain(&mut q), expect);
        }
    }

    /// `remove` on an empty queue finds nothing.
    #[test]
    fn remove_from_an_empty_queue_finds_nothing() {
        let a = node(0);
        let mut q: Queue<N> = Queue::new();
        assert!(q.remove(a.0).is_none());
        assert!(q.is_empty());
    }

    /// `extract_if` with every subset of three nodes: the filter sees each once in order, the
    /// picked nodes come back in order with clear links, the rest stay in order, and both tails
    /// take a later push.
    #[test]
    fn extract_if_splits_by_every_subset() {
        for mask in 0..8usize {
            let mut nodes: Vec<Owned> = (0..5).map(node).collect();
            let mut q: Queue<N> = Queue::new();
            for n in &mut nodes[..3] {
                q.push_back(token(n));
            }
            let mut seen = Vec::new();
            let mut taken = q.extract_if(|n| {
                // SAFETY: the filter is handed only the test's live nodes, between operations.
                let t = unsafe { (*n.as_ptr()).tag };
                seen.push(t);
                mask & (1 << t) != 0
            });
            assert_eq!(seen, vec![0, 1, 2]);
            let picked: Vec<usize> = (0..3).filter(|i| mask & (1 << i) != 0).collect();
            let kept: Vec<usize> = (0..3).filter(|i| mask & (1 << i) == 0).collect();
            assert_eq!((taken.len(), q.len()), (picked.len(), kept.len()));
            taken.push_back(token(&mut nodes[3]));
            q.push_back(token(&mut nodes[4]));
            let mut popped = Vec::new();
            while let Some(t) = taken.pop_front() {
                // SAFETY: popped from the test's own queue, so live.
                assert!(unsafe { read_link(t.as_non_null()) }.is_none());
                popped.push(tag(&t));
            }
            assert_eq!(popped, picked.into_iter().chain([3]).collect::<Vec<_>>());
            assert_eq!(
                drain(&mut q),
                kept.into_iter().chain([4]).collect::<Vec<_>>()
            );
        }
    }

    /// **The queue retags no node.** It writes a link through the node's projection, into the
    /// link's `UnsafeCell`, so a shared reference to the whole node, or to a field of it, stays
    /// valid across a push that writes that node's link. Rule 3 still forbids holding one for
    /// now; this test is what keeps a later release free to narrow rule 3 to the link alone. Run
    /// it under Miri with either borrow model: both pass it. Under a trait whose method took the
    /// node by `&mut`, Stacked Borrows reports the held reference as invalidated by that call.
    #[test]
    fn a_shared_reference_to_a_queued_node_survives_a_push() {
        let (mut a, mut b) = (node(1), node(2));
        let mut q: Queue<N> = Queue::new();
        q.push_back(token(&mut a));
        // SAFETY: `a` is live until its `Owned` drops at the end of the test.
        let whole: &N = unsafe { a.0.as_ref() };
        let tag_of_a: &usize = &whole.tag;
        // This push writes `a`'s link, since `a` is the tail.
        q.push_back(token(&mut b));
        assert_eq!((whole.tag, *tag_of_a), (1, 1));
        assert_eq!(drain(&mut q), vec![1, 2]);
    }

    /// **A token minted from the owning pointer survives the owner's later reborrows.** The
    /// sequence is a kernel thread table's over a thread's life: the table stores the pointer
    /// `Box::into_raw` returned and mints the token from it, then hands out `&mut` reborrows of
    /// the node (`Table::get_mut` below) while the token moves on and off a queue, until the node is
    /// retired. Run it under Miri with either borrow model (`cargo miri test -p intrusive_queue`,
    /// and again with `MIRIFLAGS=-Zmiri-tree-borrows`): both pass it. A token minted instead from
    /// one of those reborrows, `NonNull::from(table.get_mut(0))`, is invalidated by the next
    /// `get_mut`, and both models report the queue's next use of it as undefined behavior.
    #[test]
    fn a_token_from_the_owning_pointer_survives_later_borrows() {
        /// The thread table's shape: it owns each node through the pointer it stores.
        struct Table(Vec<NonNull<N>>);
        impl Table {
            fn get_mut(&mut self, i: usize) -> &mut N {
                // SAFETY: a stored pointer from `Box::into_raw`, live until the retire below;
                // `&mut self` makes this the only reference.
                unsafe { self.0[i].as_mut() }
            }
        }

        let mut table = Table(
            (1..=2)
                .map(|tag| NonNull::new(Box::into_raw(Box::new(N::new(tag)))).unwrap())
                .collect(),
        );
        // SAFETY: two live nodes on no queue, each minted once from the pointer the table
        // stores, and freed only after their tokens are gone.
        let (token_a, token_b) = unsafe { (Unqueued::new(table.0[0]), Unqueued::new(table.0[1])) };
        let mut q: Queue<N> = Queue::new();

        q.push_back(token_a);
        // A reborrow while the node is queued, as a mailbox write or the revocation sweep makes.
        table.get_mut(0).tag = 10;
        // This push writes the queued node's link through its token.
        q.push_back(token_b);
        table.get_mut(1).tag = 20;
        let token_a = q.pop_front().unwrap();
        assert_eq!(tag(&token_a), 10);
        table.get_mut(0).tag = 11;
        q.push_back(token_a);
        let token_b = q.pop_front().unwrap();
        let token_a = q.pop_front().unwrap();
        assert_eq!((tag(&token_b), tag(&token_a)), (20, 11));
        assert!(q.is_empty());

        // Retire: both tokens are off every queue, and the nodes go back through the stored
        // pointers.
        drop((token_a, token_b));
        for p in table.0 {
            // SAFETY: each came from `Box::into_raw` above, and no queue or token reaches it now.
            drop(unsafe { Box::from_raw(p.as_ptr()) });
        }
    }
}
