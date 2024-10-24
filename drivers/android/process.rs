// SPDX-License-Identifier: GPL-2.0

use core::{convert::TryFrom, mem::take, ops::Range};
use kernel::{
    bindings,
    cred::Credential,
    file::{self, File, IoctlCommand, IoctlHandler, PollTable},
    io_buffer::{IoBufferReader, IoBufferWriter},
    linked_list::{List, GetLinks, Links},
    mm,
    pages::Pages,
    prelude::*,
    rbtree::RBTree,
    sync::{Guard, Mutex, SpinLock, Ref, RefBorrow, UniqueRef},
    task::Task,
    user_ptr::{UserSlicePtr, UserSlicePtrReader},
    workqueue::{self, Work},
    Either,
};

use crate::{
    allocation::Allocation,
    context::Context,
    defs::*,
    node::{DeliveredNodeDeath, Node, NodeDeath, NodeRef},
    range_alloc::RangeAllocator,
    transaction::Transaction,
    thread::{BinderError, BinderResult, Thread},
    DeliverToRead, DeliverToReadListAdapter,
};

// TODO: Review this:
// Lock order: Process::node_refs -> Process::inner -> Thread::inner

#[derive(Default)]
pub(crate) struct AllocationInfo {
    /// Range within the allocation where we can find the offsets to the object descriptors.
    pub(crate) offsets: Option<Range<usize>>,
    /// The target node of the transaction this allocation is associated to.
    /// Not set for replies.
    pub(crate) target_node: Option<NodeRef>,
    /// When this allocation is dropped, call `pending_oneway_finished` on the node.
    ///
    /// This is used to serialize oneway transaction on the same node. Binder guarantees that
    /// oneway transactions to the same node are delivered sequentially in the order they are sent.
    pub(crate) oneway_node: Option<Ref<Node>>,
    /// Zero the data in the buffer on free.
    pub(crate) clear_on_free: bool,
}

struct Mapping {
    address: usize,
    alloc: RangeAllocator<AllocationInfo>,
    pages: Ref<[Pages<0>]>,
}

impl Mapping {
    fn new(address: usize, size: usize, pages: Ref<[Pages<0>]>) -> Result<Self> {
        let alloc = RangeAllocator::new(size)?;
        Ok(Self {
            address,
            alloc,
            pages,
        })
    }
}

const PROC_DEFER_FLUSH: u8 = 1;
const PROC_DEFER_RELEASE: u8 = 2;

// TODO: Make this private.
pub(crate) struct ProcessInner {
    is_manager: bool,
    is_dead: bool,
    threads: RBTree<i32, Ref<Thread>>,
    ready_threads: List<Ref<Thread>>,
    work: List<DeliverToReadListAdapter>,
    mapping: Option<Mapping>,
    nodes: RBTree<usize, Ref<Node>>,

    delivered_deaths: List<DeliveredNodeDeath>,

    /// The number of requested threads that haven't registered yet.
    requested_thread_count: u32,

    /// The maximum number of threads used by the process thread pool.
    max_threads: u32,

    /// The number of threads the started and registered with the thread pool.
    started_thread_count: u32,

    /// Bitmap of deferred work to do.
    defer_work: u8,
}

impl ProcessInner {
    fn new() -> Self {
        Self {
            is_manager: false,
            is_dead: false,
            threads: RBTree::new(),
            ready_threads: List::new(),
            work: List::new(),
            mapping: None,
            nodes: RBTree::new(),
            requested_thread_count: 0,
            max_threads: 0,
            started_thread_count: 0,
            delivered_deaths: List::new(),
            defer_work: 0,
        }
    }

    /// Called when pushing a transaction to this process.
    ///
    /// The transaction should be "new" in the sense that this process is not already part of the
    /// transaction stack. For that case, `push_work` should be used on the appropriate thread
    /// instead.
    pub(crate) fn push_new_transaction(&mut self, work: Ref<Transaction>) -> BinderResult {
        // Try to find a ready thread to which to push the work.
        if let Some(thread) = self.ready_threads.pop_front() {
            // Push to thread while holding state lock. This prevents the thread from giving up
            // (for example, because of a signal) when we're about to deliver work.
            match thread.push_new_transaction(work) {
                Ok(success) => {
                    if !success {
                        self.ready_threads.push_back(thread);
                    }
                    Ok(())
                },
                Err(err) => Err(err),
            }
        } else if self.is_dead {
            Err(BinderError::new_dead())
        } else {
            let sync = work.should_sync_wakeup();

            // There are no ready threads. Push work to process queue.
            self.work.push_back(work);

            // Wake up polling threads, if any.
            for thread in self.threads.values() {
                thread.notify_if_poll_ready(sync);
            }
            Ok(())
        }
    }

    pub(crate) fn push_work(&mut self, work: Ref<dyn DeliverToRead>) -> BinderResult {
        // Try to find a ready thread to which to push the work.
        if let Some(thread) = self.ready_threads.pop_front() {
            // Push to thread while holding state lock. This prevents the thread from giving up
            // (for example, because of a signal) when we're about to deliver work.
            match thread.push_work(work) {
                Ok(success) => {
                    if !success {
                        self.ready_threads.push_back(thread);
                    }
                    Ok(())
                },
                Err(err) => Err(err),
            }
        } else if self.is_dead {
            Err(BinderError::new_dead())
        } else {
            let sync = work.should_sync_wakeup();

            // There are no ready threads. Push work to process queue.
            self.work.push_back(work);

            // Wake up polling threads, if any.
            for thread in self.threads.values() {
                thread.notify_if_poll_ready(sync);
            }
            Ok(())
        }
    }

    pub(crate) fn is_dead(&self) -> bool {
        self.is_dead
    }

    // TODO: Should this be private?
    pub(crate) fn remove_node(&mut self, ptr: usize) {
        self.nodes.remove(&ptr);
    }

    /// Updates the reference count on the given node.
    // TODO: Decide if this should be private.
    pub(crate) fn update_node_refcount(
        &mut self,
        node: &Ref<Node>,
        inc: bool,
        strong: bool,
        count: usize,
        othread: Option<&Thread>,
    ) {
        let push = node.update_refcount_locked(inc, strong, count, self);

        // If we decided that we need to push work, push either to the process or to a thread if
        // one is specified.
        if push {
            if let Some(thread) = othread {
                thread.push_work_deferred(node.clone());
            } else {
                let _ = self.push_work(node.clone());
                // Nothing to do: `push_work` may fail if the process is dead, but that's ok as in
                // that case, it doesn't care about the notification.
            }
        }
    }

    // TODO: Make this private.
    pub(crate) fn new_node_ref(
        &mut self,
        node: Ref<Node>,
        strong: bool,
        thread: Option<&Thread>,
    ) -> NodeRef {
        self.update_node_refcount(&node, true, strong, 1, thread);
        let strong_count = if strong { 1 } else { 0 };
        NodeRef::new(node, strong_count, 1 - strong_count)
    }

    /// Returns an existing node with the given pointer and cookie, if one exists.
    ///
    /// Returns an error if a node with the given pointer but a different cookie exists.
    fn get_existing_node(&self, ptr: usize, cookie: usize) -> Result<Option<Ref<Node>>> {
        match self.nodes.get(&ptr) {
            None => Ok(None),
            Some(node) => {
                let (_, node_cookie) = node.get_id();
                if node_cookie == cookie {
                    Ok(Some(node.clone()))
                } else {
                    Err(EINVAL)
                }
            }
        }
    }

    /// Returns a reference to an existing node with the given pointer and cookie. It requires a
    /// mutable reference because it needs to increment the ref count on the node, which may
    /// require pushing work to the work queue (to notify userspace of 0 to 1 transitions).
    fn get_existing_node_ref(
        &mut self,
        ptr: usize,
        cookie: usize,
        strong: bool,
        thread: Option<&Thread>,
    ) -> Result<Option<NodeRef>> {
        Ok(self
            .get_existing_node(ptr, cookie)?
            .map(|node| self.new_node_ref(node, strong, thread)))
    }

    fn register_thread(&mut self) -> bool {
        if self.requested_thread_count == 0 {
            return false;
        }

        self.requested_thread_count -= 1;
        self.started_thread_count += 1;
        true
    }

    /// Finds a delivered death notification with the given cookie, removes it from the thread's
    /// delivered list, and returns it.
    fn pull_delivered_death(&mut self, cookie: usize) -> Option<Ref<NodeDeath>> {
        let mut cursor = self.delivered_deaths.cursor_front_mut();
        while let Some(death) = cursor.current() {
            if death.cookie == cookie {
                return cursor.remove_current();
            }
            cursor.move_next();
        }
        None
    }

    pub(crate) fn death_delivered(&mut self, death: Ref<NodeDeath>) {
        self.delivered_deaths.push_back(death);
    }
}

struct NodeRefInfo {
    node_ref: NodeRef,
    death: Option<Ref<NodeDeath>>,
}

impl NodeRefInfo {
    fn new(node_ref: NodeRef) -> Self {
        Self {
            node_ref,
            death: None,
        }
    }
}

struct ProcessNodeRefs {
    by_handle: RBTree<u32, NodeRefInfo>,
    by_global_id: RBTree<u64, u32>,
}

impl ProcessNodeRefs {
    fn new() -> Self {
        Self {
            by_handle: RBTree::new(),
            by_global_id: RBTree::new(),
        }
    }
}

pub(crate) struct Process {
    pub(crate) ctx: Ref<Context>,

    // The task leader (process).
    pub(crate) task: ARef<Task>,

    // Credential associated with file when `Process` is created.
    pub(crate) cred: ARef<Credential>,

    // TODO: Make this private again.
    pub(crate) inner: SpinLock<ProcessInner>,

    // References are in a different mutex to avoid recursive acquisition when
    // incrementing/decrementing a node in another process.
    node_refs: Mutex<ProcessNodeRefs>,

    // Work node for deferred work item.
    defer_work: Work,

    // Links for process list in Context.
    links: Links<Process>,
}

impl GetLinks for Process {
    type EntryType = Process;
    fn get_links(data: &Process) -> &Links<Process> {
        &data.links
    }
}

kernel::impl_self_work_adapter!(Process, defer_work, Process::run_deferred);

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl Process {
    fn new(ctx: Ref<Context>, cred: ARef<Credential>) -> Result<Ref<Self>> {
        let process = UniqueRef::try_new(Self {
            ctx,
            cred,
            task: Task::current().group_leader().into(),
            // SAFETY: `inner` is initialised in the call to `mutex_init` below.
            inner: unsafe { SpinLock::new(ProcessInner::new()) },
            // SAFETY: `node_refs` is initialised in the call to `mutex_init` below.
            node_refs: unsafe { Mutex::new(ProcessNodeRefs::new()) },
            // SAFETY: `node_refs` is initialised in the call to `init_work_item` below.
            defer_work: unsafe { Work::new() },
            links: Links::new(),
        })?;
        kernel::init_work_item!(&process);

        let mut process = Pin::from(process);

        // SAFETY: `inner` is pinned when `Process` is.
        let pinned = unsafe { process.as_mut().map_unchecked_mut(|p| &mut p.inner) };
        kernel::spinlock_init!(pinned, "Process::inner");

        // SAFETY: `node_refs` is pinned when `Process` is.
        let pinned = unsafe { process.as_mut().map_unchecked_mut(|p| &mut p.node_refs) };
        kernel::mutex_init!(pinned, "Process::node_refs");

        let process: Ref<Self> = process.into();
        process.ctx.register_process(process.clone());

        Ok(process)
    }

    #[inline(never)]
    pub(crate) fn debug_print(&self, m: &mut crate::debug::SeqFile) -> Result<()> {
        seq_print!(m, "pid: {}\n", self.task.pid_in_current_ns());

        let is_manager;
        let started_threads;
        let has_proc_work;
        let mut ready_threads = Vec::new();
        let mut all_threads = Vec::new();
        let mut all_nodes = Vec::new();
        loop {
            let inner = self.inner.lock();
            let ready_threads_len = {
                let mut ready_threads_len = 0;
                let mut cursor = inner.ready_threads.cursor_front();
                while cursor.current().is_some() {
                    ready_threads_len += 1;
                    cursor.move_next();
                }
                ready_threads_len
            };
            let all_threads_len = inner.threads.values().count();
            let all_nodes_len = inner.nodes.values().count();
            
            let resize_ready_threads = ready_threads_len > ready_threads.capacity();
            let resize_all_threads = all_threads_len > all_threads.capacity();
            let resize_all_nodes = all_nodes_len > all_nodes.capacity();
            if resize_ready_threads || resize_all_threads || resize_all_nodes {
                drop(inner);
                ready_threads.try_reserve(ready_threads_len)?;
                all_threads.try_reserve(all_threads_len)?;
                all_nodes.try_reserve(all_nodes_len)?;
                continue;
            }

            is_manager = inner.is_manager;
            started_threads = inner.started_thread_count;
            has_proc_work = !inner.work.is_empty();

            {
                let mut cursor = inner.ready_threads.cursor_front();
                while let Some(thread) = cursor.current() {
                    assert!(ready_threads.len() < ready_threads.capacity());
                    ready_threads.try_push(thread.id)?;
                    cursor.move_next();
                }
            }

            for thread in inner.threads.values() {
                assert!(all_threads.len() < all_threads.capacity());
                all_threads.try_push(thread.clone())?;
            }

            for node in inner.nodes.values() {
                assert!(all_nodes.len() < all_nodes.capacity());
                all_nodes.try_push(node.clone())?;
            }

            break;
        }

        seq_print!(m, "is_manager: {}\n", is_manager);
        seq_print!(m, "started_threads: {}\n", started_threads);
        seq_print!(m, "has_proc_work: {}\n", has_proc_work);
        if ready_threads.is_empty() {
            seq_print!(m, "ready_thread_ids: none\n");
        } else {
            seq_print!(m, "ready_thread_ids:");
            for thread_id in ready_threads {
                seq_print!(m, " {}", thread_id);
            }
            seq_print!(m, "\n");
        }
        for node in all_nodes {
            node.debug_print(m)?;
        }

        seq_print!(m, "all threads:\n");
        for thread in all_threads {
            thread.debug_print(m);
        }

        Ok(())
    }

    pub(crate) fn is_dead(&self) -> bool {
        self.inner.lock().is_dead
    }

    /// Attempts to fetch a work item from the process queue.
    pub(crate) fn get_work(&self) -> Option<Ref<dyn DeliverToRead>> {
        self.inner.lock().work.pop_front()
    }

    /// Attempts to fetch a work item from the process queue. If none is available, it registers the
    /// given thread as ready to receive work directly.
    ///
    /// This must only be called when the thread is not participating in a transaction chain; when
    /// it is, work will always be delivered directly to the thread (and not through the process
    /// queue).
    pub(crate) fn get_work_or_register<'a>(
        &'a self,
        thread: &'a Ref<Thread>,
    ) -> Either<Ref<dyn DeliverToRead>, Registration<'a>> {
        let mut inner = self.inner.lock();

        // Try to get work from the process queue.
        if let Some(work) = inner.work.pop_front() {
            return Either::Left(work);
        }

        // Register the thread as ready.
        Either::Right(Registration::new(self, thread, &mut inner))
    }

    fn get_current_thread(self: RefBorrow<'_, Self>) -> Result<Ref<Thread>> {
        let task = Task::current();

        // TODO: Consider using read/write locks here instead.
        {
            let inner = self.inner.lock();
            if let Some(thread) = inner.threads.get(&task.pid()) {
                return Ok(thread.clone());
            }
        }

        // Allocate a new `Thread` without holding any locks.
        let ta = Thread::new(task.into(), self.into())?;
        let node = RBTree::try_allocate_node(ta.id, ta.clone())?;

        let mut inner = self.inner.lock();

        // Recheck. It's possible the thread was create while we were not holding the lock.
        if let Some(thread) = inner.threads.get(&ta.id) {
            return Ok(thread.clone());
        }

        inner.threads.insert(node);
        Ok(ta)
    }

    pub(crate) fn push_new_transaction(&self, work: Ref<Transaction>) -> BinderResult {
        self.inner.lock().push_new_transaction(work)
    }

    pub(crate) fn push_work(&self, work: Ref<dyn DeliverToRead>) -> BinderResult {
        self.inner.lock().push_work(work)
    }

    fn set_as_manager(
        self: RefBorrow<'_, Self>,
        info: Option<FlatBinderObject>,
        thread: &Thread,
    ) -> Result {
        let (ptr, cookie, flags) = if let Some(obj) = info {
            (
                // SAFETY: The object type for this ioctl is implicitly `BINDER_TYPE_BINDER`, so it
                // is safe to access the `binder` field.
                unsafe { obj.__bindgen_anon_1.binder },
                obj.cookie,
                obj.flags,
            )
        } else {
            (0, 0, 0)
        };
        let node_ref = self.get_node(ptr as _, cookie as _, flags as _, true, Some(thread))?;
        let node = node_ref.node.clone();
        self.ctx.set_manager_node(node_ref)?;
        self.inner.lock().is_manager = true;

        // Force the state of the node to prevent the delivery of acquire/increfs.
        let mut owner_inner = node.owner.inner.lock();
        node.force_has_count(&mut owner_inner);
        Ok(())
    }

    pub(crate) fn get_node(
        self: RefBorrow<'_, Self>,
        ptr: usize,
        cookie: usize,
        flags: u32,
        strong: bool,
        thread: Option<&Thread>,
    ) -> Result<NodeRef> {
        // Try to find an existing node.
        {
            let mut inner = self.inner.lock();
            if let Some(node) = inner.get_existing_node_ref(ptr, cookie, strong, thread)? {
                return Ok(node);
            }
        }

        // Allocate the node before reacquiring the lock.
        let node = Ref::try_new(Node::new(ptr, cookie, flags, self.into()))?;
        let rbnode = RBTree::try_allocate_node(ptr, node.clone())?;

        let mut inner = self.inner.lock();
        if let Some(node) = inner.get_existing_node_ref(ptr, cookie, strong, thread)? {
            return Ok(node);
        }

        inner.nodes.insert(rbnode);
        Ok(inner.new_node_ref(node, strong, thread))
    }

    pub(crate) fn insert_or_update_handle(
        &self,
        node_ref: NodeRef,
        is_mananger: bool,
    ) -> Result<u32> {
        {
            let mut refs = self.node_refs.lock();

            // Do a lookup before inserting.
            if let Some(handle_ref) = refs.by_global_id.get(&node_ref.node.global_id) {
                let handle = *handle_ref;
                let info = refs.by_handle.get_mut(&handle).unwrap();
                info.node_ref.absorb(node_ref);
                return Ok(handle);
            }
        }

        // Reserve memory for tree nodes.
        let reserve1 = RBTree::try_reserve_node()?;
        let reserve2 = RBTree::try_reserve_node()?;

        let mut refs = self.node_refs.lock();

        // Do a lookup again as node may have been inserted before the lock was reacquired.
        if let Some(handle_ref) = refs.by_global_id.get(&node_ref.node.global_id) {
            let handle = *handle_ref;
            let info = refs.by_handle.get_mut(&handle).unwrap();
            info.node_ref.absorb(node_ref);
            return Ok(handle);
        }

        // Find id.
        let mut target = if is_mananger { 0 } else { 1 };
        for handle in refs.by_handle.keys() {
            if *handle > target {
                break;
            }
            if *handle == target {
                target = target.checked_add(1).ok_or(ENOMEM)?;
            }
        }

        // Ensure the process is still alive while we insert a new reference.
        let inner = self.inner.lock();
        if inner.is_dead {
            return Err(ESRCH);
        }
        refs.by_global_id
            .insert(reserve1.into_node(node_ref.node.global_id, target));
        refs.by_handle
            .insert(reserve2.into_node(target, NodeRefInfo::new(node_ref)));
        Ok(target)
    }

    pub(crate) fn get_transaction_node(&self, handle: u32) -> BinderResult<NodeRef> {
        // When handle is zero, try to get the context manager.
        let node = if handle == 0 {
            self.ctx.get_manager_node(true)
        } else {
            self.get_node_from_handle(handle, true)
        };

        if let Ok(node_ref) = &node {
            node_ref.node.set_used_for_transaction();
        }

        node
    }

    pub(crate) fn get_node_from_handle(&self, handle: u32, strong: bool) -> BinderResult<NodeRef> {
        self.node_refs
            .lock()
            .by_handle
            .get(&handle)
            .ok_or(ENOENT)?
            .node_ref
            .clone(strong)
    }

    pub(crate) fn remove_from_delivered_deaths(&self, death: &Ref<NodeDeath>) {
        let mut inner = self.inner.lock();
        let removed = unsafe { inner.delivered_deaths.remove(death) };
        drop(inner);
        drop(removed);
    }

    pub(crate) fn update_ref(&self, handle: u32, inc: bool, strong: bool) -> Result {
        if inc && handle == 0 {
            if let Ok(node_ref) = self.ctx.get_manager_node(strong) {
                if core::ptr::eq(self, &*node_ref.node.owner) {
                    return Err(EINVAL);
                }
                let _ = self.insert_or_update_handle(node_ref, true);
                return Ok(());
            }
        }

        // To preserve original binder behaviour, we only fail requests where the manager tries to
        // increment references on itself.
        let mut refs = self.node_refs.lock();
        if let Some(info) = refs.by_handle.get_mut(&handle) {
            if info.node_ref.update(inc, strong) {
                // Clean up death if there is one attached to this node reference.
                if let Some(death) = info.death.take() {
                    death.set_cleared(true);
                    self.remove_from_delivered_deaths(&death);
                }

                // Remove reference from process tables.
                let id = info.node_ref.node.global_id;
                refs.by_handle.remove(&handle);
                refs.by_global_id.remove(&id);
            }
        }
        Ok(())
    }

    /// Decrements the refcount of the given node, if one exists.
    pub(crate) fn update_node(&self, ptr: usize, cookie: usize, strong: bool) {
        let mut inner = self.inner.lock();
        if let Ok(Some(node)) = inner.get_existing_node(ptr, cookie) {
            inner.update_node_refcount(&node, false, strong, 1, None);
        }
    }

    pub(crate) fn inc_ref_done(&self, reader: &mut UserSlicePtrReader, strong: bool) -> Result {
        let ptr = reader.read::<usize>()?;
        let cookie = reader.read::<usize>()?;
        let mut inner = self.inner.lock();
        if let Ok(Some(node)) = inner.get_existing_node(ptr, cookie) {
            if node.inc_ref_done_locked(strong, &mut inner) {
                let _ = inner.push_work(node);
            }
        }
        Ok(())
    }

    pub(crate) fn buffer_alloc(&self, size: usize) -> BinderResult<Allocation<'_>> {
        let mut inner = self.inner.lock();
        let mut mapping = inner.mapping.as_mut().ok_or_else(BinderError::new_dead)?;
        let offset = match mapping.alloc.reserve_new_noalloc(size)? {
            Some(offset) => offset,
            None => {
                drop(mapping);
                drop(inner);
                let alloc = crate::range_alloc::ReserveNewBox::try_new()?;
                inner = self.inner.lock();
                mapping = inner.mapping.as_mut().ok_or_else(BinderError::new_dead)?;
                mapping.alloc.reserve_new(size, alloc)?
            },
        };
        Ok(Allocation::new(
            self,
            offset,
            size,
            mapping.address + offset,
            mapping.pages.clone(),
        ))
    }

    // TODO: Review if we want an Option or a Result.
    pub(crate) fn buffer_get(&self, ptr: usize) -> Option<Allocation<'_>> {
        let mut inner = self.inner.lock();
        let mapping = inner.mapping.as_mut()?;
        let offset = ptr.checked_sub(mapping.address)?;
        let (size, odata) = mapping.alloc.reserve_existing(offset).ok()?;
        let mut alloc = Allocation::new(self, offset, size, ptr, mapping.pages.clone());
        if let Some(data) = odata {
            alloc.set_info(data);
        }
        Some(alloc)
    }

    pub(crate) fn buffer_raw_free(&self, ptr: usize) {
        let mut inner = self.inner.lock();
        if let Some(ref mut mapping) = &mut inner.mapping {
            if ptr < mapping.address
                || mapping
                    .alloc
                    .reservation_abort(ptr - mapping.address)
                    .is_err()
            {
                pr_warn!(
                    "Pointer {:x} failed to free, base = {:x}\n",
                    ptr,
                    mapping.address
                );
            }
        }
    }

    pub(crate) fn buffer_make_freeable(&self, offset: usize, data: Option<AllocationInfo>) {
        let mut inner = self.inner.lock();
        if let Some(ref mut mapping) = &mut inner.mapping {
            if mapping.alloc.reservation_commit(offset, data).is_err() {
                pr_warn!("Offset {} failed to be marked freeable\n", offset);
            }
        }
    }

    fn create_mapping(&self, vma: &mut mm::virt::Area) -> Result {
        let size = core::cmp::min(vma.end() - vma.start(), bindings::SZ_4M as usize);
        let page_count = size / kernel::PAGE_SIZE;

        // Allocate and map all pages.
        //
        // N.B. If we fail halfway through mapping these pages, the kernel will unmap them.
        let mut pages = Vec::new();
        pages.try_reserve_exact(page_count)?;
        let mut address = vma.start();
        for _ in 0..page_count {
            let page = Pages::<0>::new()?;
            vma.insert_page(address, &page)?;
            pages.try_push(page)?;
            address += kernel::PAGE_SIZE;
        }

        let ref_pages = Ref::try_from(pages)?;
        let mapping = Mapping::new(vma.start(), size, ref_pages)?;

        // Save pages for later.
        let mut inner = self.inner.lock();
        match &inner.mapping {
            None => inner.mapping = Some(mapping),
            Some(_) => {
                drop(inner);
                drop(mapping);
                return Err(EBUSY)
            },
        }
        Ok(())
    }

    fn version(&self, data: UserSlicePtr) -> Result {
        data.writer().write(&BinderVersion::current())
    }

    pub(crate) fn register_thread(&self) -> bool {
        self.inner.lock().register_thread()
    }

    fn remove_thread(&self, thread: Ref<Thread>) {
        self.inner.lock().threads.remove(&thread.id);
        thread.release();
    }

    fn set_max_threads(&self, max: u32) {
        self.inner.lock().max_threads = max;
    }

    fn get_node_debug_info(&self, data: UserSlicePtr) -> Result {
        let (mut reader, mut writer) = data.reader_writer();

        // Read the starting point.
        let ptr = reader.read::<BinderNodeDebugInfo>()?.ptr as usize;
        let mut out = BinderNodeDebugInfo::default();

        {
            let inner = self.inner.lock();
            for (node_ptr, node) in &inner.nodes {
                if *node_ptr > ptr {
                    node.populate_debug_info(&mut out, &inner);
                    break;
                }
            }
        }

        writer.write(&out)
    }

    fn get_node_info_from_ref(&self, data: UserSlicePtr) -> Result {
        let (mut reader, mut writer) = data.reader_writer();
        let mut out = reader.read::<BinderNodeInfoForRef>()?;

        if out.strong_count != 0
            || out.weak_count != 0
            || out.reserved1 != 0
            || out.reserved2 != 0
            || out.reserved3 != 0
        {
            return Err(EINVAL);
        }

        // Only the context manager is allowed to use this ioctl.
        if !self.inner.lock().is_manager {
            return Err(EPERM);
        }

        let node_ref = self
            .get_node_from_handle(out.handle, true)
            .or(Err(EINVAL))?;

        // Get the counts from the node.
        {
            let owner_inner = node_ref.node.owner.inner.lock();
            node_ref.node.populate_counts(&mut out, &owner_inner);
        }

        // Write the result back.
        writer.write(&out)
    }

    pub(crate) fn needs_thread(&self) -> bool {
        let mut inner = self.inner.lock();
        let ret = inner.requested_thread_count == 0
            && inner.ready_threads.is_empty()
            && inner.started_thread_count < inner.max_threads;
        if ret {
            inner.requested_thread_count += 1
        };
        ret
    }

    pub(crate) fn request_death(
        self: &Ref<Self>,
        reader: &mut UserSlicePtrReader,
        thread: &Thread,
    ) -> Result {
        let handle: u32 = reader.read()?;
        let cookie: usize = reader.read()?;

        // TODO: First two should result in error, but not the others.

        // TODO: Do we care about the context manager dying?

        // Queue BR_ERROR if we can't allocate memory for the death notification.
        let death = UniqueRef::try_new_uninit().map_err(|err| {
            thread.push_return_work(BR_ERROR);
            err
        })?;

        let mut refs = self.node_refs.lock();
        let info = refs.by_handle.get_mut(&handle).ok_or(EINVAL)?;

        // Nothing to do if there is already a death notification request for this handle.
        if info.death.is_some() {
            return Ok(());
        }

        let death = {
            let mut pinned = Pin::from(death.write(
                // SAFETY: `init` is called below.
                unsafe { NodeDeath::new(info.node_ref.node.clone(), self.clone(), cookie) },
            ));
            pinned.as_mut().init();
            Ref::<NodeDeath>::from(pinned)
        };

        info.death = Some(death.clone());

        // Register the death notification.
        {
            let mut owner_inner = info.node_ref.node.owner.inner.lock();
            if owner_inner.is_dead {
                drop(owner_inner);
                let _ = self.push_work(death);
            } else {
                info.node_ref.node.add_death(death, &mut owner_inner);
            }
        }
        Ok(())
    }

    pub(crate) fn clear_death(&self, reader: &mut UserSlicePtrReader, thread: &Thread) -> Result {
        let handle: u32 = reader.read()?;
        let cookie: usize = reader.read()?;

        let mut refs = self.node_refs.lock();
        let info = refs.by_handle.get_mut(&handle).ok_or(EINVAL)?;

        let death = info.death.take().ok_or(EINVAL)?;
        if death.cookie != cookie {
            info.death = Some(death);
            return Err(EINVAL);
        }

        // Update state and determine if we need to queue a work item. We only need to do it when
        // the node is not dead or if the user already completed the death notification.
        if death.set_cleared(false) {
            let _ = thread.push_work_if_looper(death);
        }

        Ok(())
    }

    pub(crate) fn dead_binder_done(&self, cookie: usize, thread: &Thread) {
        if let Some(death) = self.inner.lock().pull_delivered_death(cookie) {
            death.set_notification_done(thread);
        }
    }

    pub(crate) fn flush(this: RefBorrow<'_, Process>) -> Result {
        let should_schedule;
        {
            let mut inner = this.inner.lock();
            should_schedule = inner.defer_work == 0;
            inner.defer_work |= PROC_DEFER_FLUSH;
        }

        if should_schedule {
            workqueue::system().enqueue(Ref::from(this));
        }

        Ok(())
    }

    fn deferred_flush(&self) {
        let inner = self.inner.lock();
        for thread in inner.threads.values() {
            thread.notify_flush();
        }
    }

    fn deferred_release(self: Ref<Self>) {
        // Mark this process as dead. We'll do the same for the threads later.
        let is_manager = {
            let mut inner = self.inner.lock();
            inner.is_dead = true;
            inner.is_manager
        };

        // If this process is the manager, unset it.
        if is_manager {
            self.ctx.unset_manager_node();
        }

        self.ctx.deregister_process(&self);

        // Cancel all pending work items.
        while let Some(work) = self.get_work() {
            work.cancel();
        }

        // Free any resources kept alive by allocated buffers.
        let omapping = self.inner.lock().mapping.take();
        if let Some(mut mapping) = omapping {
            let address = mapping.address;
            let pages = mapping.pages.clone();
            mapping.alloc.for_each(|offset, size, odata| {
                let ptr = offset + address;
                let mut alloc = Allocation::new(&self, offset, size, ptr, pages.clone());
                if let Some(data) = odata {
                    alloc.set_info(data);
                }
                drop(alloc)
            });
        }

        // Drop all references. We do this dance with `swap` to avoid destroying the references
        // while holding the lock.
        let mut refs = self.node_refs.lock();
        let mut node_refs = take(&mut refs.by_handle);
        drop(refs);

        // Remove all death notifications from the nodes (that belong to a different process).
        for info in node_refs.values_mut() {
            let death = if let Some(existing) = info.death.take() {
                existing
            } else {
                continue;
            };

            death.set_cleared(false);
        }

        // Do similar dance for the state lock.
        let mut inner = self.inner.lock();
        let threads = take(&mut inner.threads);
        let nodes = take(&mut inner.nodes);
        drop(inner);

        // Cleanup queued oneway transactions.
        for node in nodes.values() {
            node.cleanup_oneway();
        }

        // Release all threads.
        for thread in threads.values() {
            thread.release();
        }

        // Deliver death notifications.
        for node in nodes.values() {
            loop {
                let death = {
                    let mut inner = self.inner.lock();
                    if let Some(death) = node.next_death(&mut inner) {
                        death
                    } else {
                        break;
                    }
                };

                death.set_dead();
            }
        }
    }

    pub(crate) fn run_deferred(self: Ref<Self>) {
        let defer;
        {
            let mut inner = self.inner.lock();
            defer = inner.defer_work;
            inner.defer_work = 0;
        }

        if defer & PROC_DEFER_FLUSH != 0 {
            self.deferred_flush();
        }
        if defer & PROC_DEFER_RELEASE != 0 {
            self.deferred_release();
        }
    }
}

impl IoctlHandler for Process {
    type Target<'a> = RefBorrow<'a, Process>;

    fn write(
        this: RefBorrow<'_, Process>,
        _file: &File,
        cmd: u32,
        reader: &mut UserSlicePtrReader,
    ) -> Result<i32> {
        let thread = this.get_current_thread()?;
        match cmd {
            bindings::BINDER_SET_MAX_THREADS => this.set_max_threads(reader.read()?),
            bindings::BINDER_SET_CONTEXT_MGR => this.set_as_manager(None, &thread)?,
            bindings::BINDER_THREAD_EXIT => this.remove_thread(thread),
            bindings::BINDER_SET_CONTEXT_MGR_EXT => {
                this.set_as_manager(Some(reader.read()?), &thread)?
            }
            bindings::BINDER_ENABLE_ONEWAY_SPAM_DETECTION => { /* do nothing */ },
            _ => return Err(EINVAL),
        }
        Ok(0)
    }

    fn read_write(
        this: RefBorrow<'_, Process>,
        file: &File,
        cmd: u32,
        data: UserSlicePtr,
    ) -> Result<i32> {
        let thread = this.get_current_thread()?;
        let blocking = (file.flags() & file::flags::O_NONBLOCK) == 0;
        match cmd {
            bindings::BINDER_WRITE_READ => thread.write_read(data, blocking)?,
            bindings::BINDER_GET_NODE_DEBUG_INFO => this.get_node_debug_info(data)?,
            bindings::BINDER_GET_NODE_INFO_FOR_REF => this.get_node_info_from_ref(data)?,
            bindings::BINDER_VERSION => this.version(data)?,
            _ => return Err(EINVAL),
        }
        Ok(0)
    }
}

#[vtable]
impl file::Operations for Process {
    type Data = Ref<Self>;
    type OpenData = Ref<Context>;

    fn open(ctx: &Ref<Context>, file: &File) -> Result<Self::Data> {
        Self::new(ctx.clone(), file.cred().into())
    }

    fn release(this: Self::Data, _file: &File) {
        let should_schedule;
        {
            let mut inner = this.inner.lock();
            should_schedule = inner.defer_work == 0;
            inner.defer_work |= PROC_DEFER_RELEASE;
        }

        if should_schedule {
            workqueue::system().enqueue(this.clone());
        }
    }

    fn ioctl(this: RefBorrow<'_, Process>, file: &File, cmd: &mut IoctlCommand) -> Result<i32> {
        cmd.dispatch::<Self>(this, file)
    }

    fn compat_ioctl(
        this: RefBorrow<'_, Process>,
        file: &File,
        cmd: &mut IoctlCommand,
    ) -> Result<i32> {
        cmd.dispatch::<Self>(this, file)
    }

    fn mmap(this: RefBorrow<'_, Process>, _file: &File, vma: &mut mm::virt::Area) -> Result {
        // We don't allow mmap to be used in a different process.
        if !core::ptr::eq(Task::current().group_leader(), &*this.task) {
            return Err(EINVAL);
        }

        if vma.start() == 0 {
            return Err(EINVAL);
        }

        let mut flags = vma.flags();
        use mm::virt::flags::*;
        if flags & WRITE != 0 {
            return Err(EPERM);
        }

        flags |= DONTCOPY | MIXEDMAP;
        flags &= !MAYWRITE;
        vma.set_flags(flags);

        // TODO: Set ops. We need to learn when the user unmaps so that we can stop using it.
        this.create_mapping(vma)
    }

    fn poll(this: RefBorrow<'_, Process>, file: &File, table: &PollTable) -> Result<u32> {
        let thread = this.get_current_thread()?;
        let (from_proc, mut mask) = thread.poll(file, table);
        if mask == 0 && from_proc && !this.inner.lock().work.is_empty() {
            mask |= bindings::POLLIN;
        }
        Ok(mask)
    }
}

pub(crate) struct Registration<'a> {
    process: &'a Process,
    thread: &'a Ref<Thread>,
}

impl<'a> Registration<'a> {
    fn new(
        process: &'a Process,
        thread: &'a Ref<Thread>,
        guard: &mut Guard<'_, SpinLock<ProcessInner>>,
    ) -> Self {
        guard.ready_threads.push_back(thread.clone());
        Self { process, thread }
    }
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        let mut inner = self.process.inner.lock();
        unsafe { inner.ready_threads.remove(self.thread) };
    }
}
