// SPDX-License-Identifier: GPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};
use kernel::{
    io_buffer::IoBufferWriter,
    linked_list::{GetLinks, GetLinksWrapped, Links, List},
    prelude::*,
    sync::{Guard, LockedBy, Ref, SpinLock},
    user_ptr::UserSlicePtrWriter,
};

use crate::{
    defs::*,
    process::{Process, ProcessInner},
    thread::{BinderError, BinderResult, Thread},
    transaction::Transaction,
    DeliverToRead, DeliverToReadListAdapter
};

struct CountState {
    count: usize,
    has_count: bool,
}

impl CountState {
    fn new() -> Self {
        Self {
            count: 0,
            has_count: false,
        }
    }
}

struct NodeInner {
    strong: CountState,
    weak: CountState,
    death_list: List<Ref<NodeDeath>>,
    oneway_todo: List<DeliverToReadListAdapter>,
    has_pending_oneway_todo: bool,
    /// The number of active BR_INCREFS or BR_ACQUIRE acquire operations. (should be maximum two)
    ///
    /// We can never submit a BR_RELEASE or BR_DECREFS while this is non-zero.
    active_inc_refs: u8,
}

pub(crate) struct Node {
    pub(crate) global_id: u64,
    ptr: usize,
    cookie: usize,
    pub(crate) flags: u32,
    pub(crate) owner: Ref<Process>,
    inner: LockedBy<NodeInner, SpinLock<ProcessInner>>,
    links: Links<dyn DeliverToRead>,
}

impl Node {
    pub(crate) fn new(ptr: usize, cookie: usize, flags: u32, owner: Ref<Process>) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let inner = LockedBy::new(
            &owner.inner,
            NodeInner {
                strong: CountState::new(),
                weak: CountState::new(),
                death_list: List::new(),
                oneway_todo: List::new(),
                has_pending_oneway_todo: false,
                active_inc_refs: 0,
            },
        );
        Self {
            global_id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            ptr,
            cookie,
            flags,
            owner,
            inner,
            links: Links::new(),
        }
    }

    pub(crate) fn set_used_for_transaction(&self) {
        let mut guard = self.owner.inner.lock();
        let inner = self.inner.access_mut(&mut guard);
        let has_strong = inner.strong.has_count;
        drop(inner);
        if !has_strong {
            pr_err!("Failure: Sending transaction to {} but strong.has_count is false", self.global_id);
        }
    }

    #[inline(never)]
    pub(crate) fn debug_print(&self, m: &mut crate::debug::SeqFile) -> Result<()> {
        let weak;
        let strong;
        let has_weak;
        let has_strong;
        let active_inc_refs;
        {
            let mut guard = self.owner.inner.lock();
            let inner = self.inner.access_mut(&mut guard);
            weak = inner.weak.count;
            has_weak = inner.weak.has_count;
            strong = inner.strong.count;
            has_strong = inner.strong.has_count;
            active_inc_refs = inner.active_inc_refs;
        }

        let has_weak = if has_weak { "Y" } else { "N" };
        let has_strong = if has_strong { "Y" } else { "N" };

        seq_print!(m, "node {},{:#x},{}: strong{}{} weak{}{} active{}\n", self.global_id, self.ptr, self.cookie, strong, has_strong, weak, has_weak, active_inc_refs);
        Ok(())
    }

    pub(crate) fn get_id(&self) -> (usize, usize) {
        (self.ptr, self.cookie)
    }

    pub(crate) fn next_death(
        &self,
        guard: &mut Guard<'_, SpinLock<ProcessInner>>,
    ) -> Option<Ref<NodeDeath>> {
        self.inner.access_mut(guard).death_list.pop_front()
    }

    pub(crate) fn add_death(
        &self,
        death: Ref<NodeDeath>,
        guard: &mut Guard<'_, SpinLock<ProcessInner>>,
    ) {
        self.inner.access_mut(guard).death_list.push_back(death);
    }

    pub(crate) fn inc_ref_done_locked(
        &self,
        _strong: bool,
        owner_inner: &mut ProcessInner,
    ) -> bool {
        let inner = self.inner.access_from_mut(owner_inner);
        if inner.active_inc_refs == 0 {
            pr_err!("inc_ref_done called when no active inc_refs");
            return false;
        }

        inner.active_inc_refs -= 1;
        if inner.active_inc_refs == 0 {
            // Having active inc_refs can inhibit dropping of ref-counts. Calculate whether we
            // would send a refcount decrement, and if so, tell the caller to schedule us.
            let strong = inner.strong.count > 0;
            let has_strong = inner.strong.has_count;
            let weak = strong || inner.weak.count > 0;
            let has_weak = inner.weak.has_count;

            let should_drop_weak = !weak && has_weak;
            let should_drop_strong = !strong && has_strong;

            // If we want to drop the ref-count again, tell the caller to schedule a work node for
            // that.
            should_drop_weak || should_drop_strong
        } else {
            false
        }
    }

    pub(crate) fn update_refcount_locked(
        &self,
        inc: bool,
        strong: bool,
        count: usize,
        owner_inner: &mut ProcessInner,
    ) -> bool {
        let inner = self.inner.access_from_mut(owner_inner);

        // Get a reference to the state we'll update.
        let state = if strong {
            &mut inner.strong
        } else {
            &mut inner.weak
        };

        // Update the count and determine whether we need to push work.
        // TODO: Here we may want to check the weak count being zero but the strong count being 1,
        // because in such cases, we won't deliver anything to userspace, so we shouldn't queue
        // either.
        if inc {
            state.count += count;
            !state.has_count
        } else {
            if state.count < count {
                pr_err!("Failure: refcount underflow!");
                return false;
            }
            state.count -= count;
            state.count == 0 && state.has_count
        }
    }

    pub(crate) fn update_refcount(self: &Ref<Self>, inc: bool, count: usize, strong: bool) {
        self.owner
            .inner
            .lock()
            .update_node_refcount(self, inc, strong, count, None);
    }

    pub(crate) fn populate_counts(
        &self,
        out: &mut BinderNodeInfoForRef,
        guard: &Guard<'_, SpinLock<ProcessInner>>,
    ) {
        let inner = self.inner.access(guard);
        out.strong_count = inner.strong.count as _;
        out.weak_count = inner.weak.count as _;
    }

    pub(crate) fn populate_debug_info(
        &self,
        out: &mut BinderNodeDebugInfo,
        guard: &Guard<'_, SpinLock<ProcessInner>>,
    ) {
        out.ptr = self.ptr as _;
        out.cookie = self.cookie as _;
        let inner = self.inner.access(guard);
        if inner.strong.has_count {
            out.has_strong_ref = 1;
        }
        if inner.weak.has_count {
            out.has_weak_ref = 1;
        }
    }

    pub(crate) fn force_has_count(&self, guard: &mut Guard<'_, SpinLock<ProcessInner>>) {
        let inner = self.inner.access_mut(guard);
        inner.strong.has_count = true;
        inner.weak.has_count = true;
    }

    fn write(&self, writer: &mut UserSlicePtrWriter, code: u32) -> Result {
        writer.write(&code)?;
        writer.write(&self.ptr)?;
        writer.write(&self.cookie)?;
        Ok(())
    }

    pub(crate) fn submit_oneway(&self, transaction: Ref<Transaction>) -> BinderResult {
        let mut guard = self.owner.inner.lock();
        let inner = self.inner.access_mut(&mut guard);
        if inner.has_pending_oneway_todo {
            inner.oneway_todo.push_back(transaction);
        } else {
            inner.has_pending_oneway_todo = true;
            drop(inner);
            guard.push_work(transaction)?;
        }
        Ok(())
    }

    pub(crate) fn pending_oneway_finished(&self) {
        let mut guard = self.owner.inner.lock();
        if !guard.is_dead() {
            let transaction = {
                let inner = self.inner.access_mut(&mut guard);

                match inner.oneway_todo.pop_front() {
                    Some(transaction) => transaction,
                    None => {
                        inner.has_pending_oneway_todo = false;
                        return;
                    }
                }
            };

            let push_res = guard.push_work(transaction);
            let inner = self.inner.access_mut(&mut guard);
            match push_res {
                Ok(()) => {
                    inner.has_pending_oneway_todo = true;
                    return;
                },
                Err(_err) => {
                    // This only fails if the process is dead.
                    // We fall through to cleanup below.
                },
            }
        }

        // Process is dead. Just clean up everything.
        loop {
            let inner = self.inner.access_mut(&mut guard);
            let mut oneway_todo = core::mem::take(&mut inner.oneway_todo);
            inner.has_pending_oneway_todo = false;
            drop(guard);

            if oneway_todo.is_empty() {
                break;
            }

            while let Some(work) = oneway_todo.pop_front() {
                work.cancel();
            }

            guard = self.owner.inner.lock();
        }
    }

    pub(crate) fn cleanup_oneway(&self) {
        let mut guard = self.owner.inner.lock();
        loop {
            let inner = self.inner.access_mut(&mut guard);
            let mut oneway_todo = core::mem::take(&mut inner.oneway_todo);
            inner.has_pending_oneway_todo = false;
            drop(guard);

            if oneway_todo.is_empty() {
                break;
            }

            while let Some(work) = oneway_todo.pop_front() {
                work.cancel();
            }

            guard = self.owner.inner.lock();
        }
    }
}

impl DeliverToRead for Node {
    fn do_work(self: Ref<Self>, _thread: &Thread, writer: &mut UserSlicePtrWriter) -> Result<bool> {
        let mut owner_inner = self.owner.inner.lock();
        let mut inner = self.inner.access_mut(&mut owner_inner);
        let strong = inner.strong.count > 0;
        let has_strong = inner.strong.has_count;
        let weak = strong || inner.weak.count > 0;
        let has_weak = inner.weak.has_count;

        if weak && !has_weak {
            inner.weak.has_count = true;
            inner.active_inc_refs += 1;
        }

        if strong && !has_strong {
            inner.strong.has_count = true;
            inner.active_inc_refs += 1;
        }

        let no_active_inc_refs = inner.active_inc_refs == 0;
        let should_drop_weak = no_active_inc_refs && (!weak && has_weak);
        let should_drop_strong = no_active_inc_refs && (!strong && has_strong);
        if should_drop_weak {
            inner.weak.has_count = false;
        }
        if should_drop_strong {
            inner.strong.has_count = false;
        }

        if no_active_inc_refs && !weak {
            // Remove the node if there are no references to it.
            owner_inner.remove_node(self.ptr);
        }

        drop(owner_inner);

        if weak && !has_weak {
            self.write(writer, BR_INCREFS)?;
        }

        if strong && !has_strong {
            self.write(writer, BR_ACQUIRE)?;
        }

        if should_drop_strong {
            self.write(writer, BR_RELEASE)?;
        }

        if should_drop_weak {
            self.write(writer, BR_DECREFS)?;
        }

        Ok(true)
    }

    fn get_links(&self) -> &Links<dyn DeliverToRead> {
        &self.links
    }

    fn should_sync_wakeup(&self) -> bool {
        false
    }
}

pub(crate) struct NodeRef {
    pub(crate) node: Ref<Node>,
    /// How many times does this NodeRef hold a refcount on the Node?
    strong_node_count: usize,
    weak_node_count: usize,
    /// How many times does userspace hold a refcount on this NodeRef?
    strong_count: usize,
    weak_count: usize,
}

impl NodeRef {
    pub(crate) fn new(node: Ref<Node>, strong_count: usize, weak_count: usize) -> Self {
        Self {
            node,
            strong_node_count: strong_count,
            weak_node_count: weak_count,
            strong_count,
            weak_count,
        }
    }

    pub(crate) fn absorb(&mut self, mut other: Self) {
        assert!(Ref::ptr_eq(&self.node, &other.node), "absorb called with differing nodes");

        self.strong_node_count += other.strong_node_count;
        self.weak_node_count += other.weak_node_count;
        self.strong_count += other.strong_count;
        self.weak_count += other.weak_count;
        other.strong_count = 0;
        other.weak_count = 0;
        other.strong_node_count = 0;
        other.weak_node_count = 0;
    }

    pub(crate) fn clone(&self, strong: bool) -> BinderResult<NodeRef> {
        if strong && self.strong_count == 0 {
            return Err(BinderError::new_failed());
        }

        Ok(self
            .node
            .owner
            .inner
            .lock()
            .new_node_ref(self.node.clone(), strong, None))
    }

    /// Updates (increments or decrements) the number of references held against the node. If the
    /// count being updated transitions from 0 to 1 or from 1 to 0, the node is notified by having
    /// its `update_refcount` function called.
    ///
    /// Returns whether `self` should be removed (when both counts are zero).
    pub(crate) fn update(&mut self, inc: bool, strong: bool) -> bool {
        if strong && self.strong_count == 0 {
            return false;
        }

        let (count, node_count, other_count) = if strong {
            (&mut self.strong_count, &mut self.strong_node_count, self.weak_count)
        } else {
            (&mut self.weak_count, &mut self.weak_node_count, self.strong_count)
        };

        if inc {
            if *count == 0 {
                *node_count = 1;
                self.node.update_refcount(true, 1, strong);
            }
            *count += 1;
        } else {
            *count -= 1;
            if *count == 0 {
                self.node.update_refcount(false, *node_count, strong);
                *node_count = 0;
                return other_count == 0;
            }
        }

        false
    }
}

impl Drop for NodeRef {
    fn drop(&mut self) {
        if self.strong_node_count > 0 {
            self.node.update_refcount(false, self.strong_node_count, true);
        }

        if self.weak_node_count > 0 {
            self.node.update_refcount(false, self.weak_node_count, false);
        }
    }
}

struct NodeDeathInner {
    dead: bool,
    cleared: bool,
    notification_done: bool,

    /// Indicates whether the normal flow was interrupted by removing the handle. In this case, we
    /// need behave as if the death notification didn't exist (i.e., we don't deliver anything to
    /// the user.
    aborted: bool,
}

pub(crate) struct NodeDeath {
    node: Ref<Node>,
    process: Ref<Process>,
    // TODO: Make this private.
    pub(crate) cookie: usize,
    work_links: Links<dyn DeliverToRead>,
    death_links: Links<NodeDeath>,
    delivered_links: Links<NodeDeath>,
    inner: SpinLock<NodeDeathInner>,
}

pub(crate) struct DeliveredNodeDeath;

impl NodeDeath {
    /// Constructs a new node death notification object.
    ///
    /// # Safety
    ///
    /// The caller must call `NodeDeath::init` before using the notification object.
    pub(crate) unsafe fn new(node: Ref<Node>, process: Ref<Process>, cookie: usize) -> Self {
        Self {
            node,
            process,
            cookie,
            work_links: Links::new(),
            death_links: Links::new(),
            delivered_links: Links::new(),
            inner: unsafe {
                SpinLock::new(NodeDeathInner {
                    dead: false,
                    cleared: false,
                    notification_done: false,
                    aborted: false,
                })
            },
        }
    }

    pub(crate) fn init(self: Pin<&mut Self>) {
        // SAFETY: `inner` is pinned when `self` is.
        let inner = unsafe { self.map_unchecked_mut(|n| &mut n.inner) };
        kernel::spinlock_init!(inner, "NodeDeath::inner");
    }

    /// Sets the cleared flag to `true`.
    ///
    /// It removes `self` from the node's death notification list if needed. It must only be called
    /// once.
    ///
    /// Returns whether it needs to be queued.
    pub(crate) fn set_cleared(self: &Ref<Self>, abort: bool) -> bool {
        let (needs_removal, needs_queueing) = {
            // Update state and determine if we need to queue a work item. We only need to do it
            // when the node is not dead or if the user already completed the death notification.
            let mut inner = self.inner.lock();
            inner.cleared = true;
            if abort {
                inner.aborted = true;
            }
            (!inner.dead, !inner.dead || inner.notification_done)
        };

        // Remove death notification from node.
        if needs_removal {
            let mut owner_inner = self.node.owner.inner.lock();
            let node_inner = self.node.inner.access_mut(&mut owner_inner);
            unsafe { node_inner.death_list.remove(self) };
        }

        needs_queueing
    }

    /// Sets the 'notification done' flag to `true`.
    ///
    /// Returns whether it needs to be queued.
    pub(crate) fn set_notification_done(self: Ref<Self>, thread: &Thread) {
        let needs_queueing = {
            let mut inner = self.inner.lock();
            inner.notification_done = true;
            inner.cleared
        };

        if needs_queueing {
            let _ = thread.push_work_if_looper(self);
        }
    }

    /// Sets the 'dead' flag to `true` and queues work item if needed.
    pub(crate) fn set_dead(self: Ref<Self>) {
        let needs_queueing = {
            let mut inner = self.inner.lock();
            if inner.cleared {
                false
            } else {
                inner.dead = true;
                true
            }
        };

        if needs_queueing {
            // Push the death notification to the target process. There is nothing else to do if
            // it's already dead.
            let process = self.process.clone();
            let _ = process.push_work(self);
        }
    }
}

impl GetLinks for NodeDeath {
    type EntryType = NodeDeath;
    fn get_links(data: &NodeDeath) -> &Links<NodeDeath> {
        &data.death_links
    }
}

impl GetLinks for DeliveredNodeDeath {
    type EntryType = NodeDeath;
    fn get_links(data: &NodeDeath) -> &Links<NodeDeath> {
        &data.delivered_links
    }
}

impl GetLinksWrapped for DeliveredNodeDeath {
    type Wrapped = Ref<NodeDeath>;
}

impl DeliverToRead for NodeDeath {
    fn do_work(self: Ref<Self>, _thread: &Thread, writer: &mut UserSlicePtrWriter) -> Result<bool> {
        let done = {
            let inner = self.inner.lock();
            if inner.aborted {
                return Ok(true);
            }
            inner.cleared && (!inner.dead || inner.notification_done)
        };

        let cookie = self.cookie;
        let cmd = if done {
            BR_CLEAR_DEATH_NOTIFICATION_DONE
        } else {
            let process = self.process.clone();
            let mut process_inner = process.inner.lock();
            let inner = self.inner.lock();
            if inner.aborted {
                return Ok(true);
            }
            // We're still holding the inner lock, so it cannot be aborted while we insert it into
            // the delivered list.
            process_inner.death_delivered(self.clone());
            BR_DEAD_BINDER
        };

        writer.write(&cmd)?;
        writer.write(&cookie)?;

        // Mimic the original code: we stop processing work items when we get to a death
        // notification.
        Ok(cmd != BR_DEAD_BINDER)
    }

    fn get_links(&self) -> &Links<dyn DeliverToRead> {
        &self.work_links
    }

    fn should_sync_wakeup(&self) -> bool {
        false
    }
}
