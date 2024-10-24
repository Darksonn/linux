// SPDX-License-Identifier: GPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};
use kernel::{
    bindings,
    file::{File, FileDescriptorReservation},
    io_buffer::IoBufferWriter,
    linked_list::List,
    linked_list::{GetLinks, Links},
    prelude::*,
    sync::{Ref, SpinLock, UniqueRef},
    task::{Kuid, Task},
    user_ptr::UserSlicePtrWriter,
    Either, ScopeGuard,
};

use crate::{
    defs::*,
    node::{Node, NodeRef},
    process::Process,
    ptr_align,
    thread::{BinderResult, BinderError, Thread},
    DeliverToRead,
};

struct TransactionInner {
    file_list: List<Box<FileInfo>>,
}

pub(crate) struct Transaction {
    inner: SpinLock<TransactionInner>,
    target_node: Option<Ref<Node>>,
    stack_next: Option<Ref<Transaction>>,
    pub(crate) from: Ref<Thread>,
    to: Ref<Process>,
    pub(crate) pi_node: crate::pi::PINode,
    free_allocation: AtomicBool,
    code: u32,
    pub(crate) flags: u32,
    data_size: usize,
    offsets_size: usize,
    data_address: usize,
    links: Links<dyn DeliverToRead>,
    sender_euid: Kuid,
    txn_security_ctx_off: Option<usize>,
}

impl Transaction {
    pub(crate) fn new(
        node_ref: NodeRef,
        stack_next: Option<Ref<Transaction>>,
        from: &Ref<Thread>,
        tr: &BinderTransactionDataSg,
    ) -> BinderResult<Ref<Self>> {
        let trd = &tr.transaction_data;
        let allow_fds = node_ref.node.flags & FLAT_BINDER_FLAG_ACCEPTS_FDS != 0;
        let txn_security_ctx = node_ref.node.flags & FLAT_BINDER_FLAG_TXN_SECURITY_CTX != 0;
        let mut txn_security_ctx_off = if txn_security_ctx { Some(0) } else { None };
        let to = node_ref.node.owner.clone();
        let mut alloc = match from.copy_transaction_data(&to, tr, allow_fds, txn_security_ctx_off.as_mut()) {
            Ok(alloc) => alloc,
            Err(err) => {
                pr_warn!("Failure in copy_transaction_data: {:?}", err);
                return Err(err);
            },
        };
        if trd.flags & TF_ONE_WAY != 0 {
            if stack_next.is_some() {
                pr_warn!("Oneway transaction should not be in a transaction stack.");
                return Err(BinderError::new_failed());
            }
            alloc.set_info_oneway_node(node_ref.node.clone());
        }
        if trd.flags & TF_CLEAR_BUF != 0 {
            alloc.set_info_clear_on_drop();
        }
        let target_node = node_ref.node.clone();
        alloc.set_info_target_node(node_ref);
        let data_address = alloc.ptr;
        let file_list = alloc.take_file_list();
        alloc.keep_alive();
        let mut tr = Pin::from(UniqueRef::try_new(Self {
            // SAFETY: `spinlock_init` is called below.
            inner: unsafe { SpinLock::new(TransactionInner { file_list }) },
            // SAFETY: `PINode::init` is called below.
            pi_node: unsafe { crate::pi::PINode::new(&from.task) },
            target_node: Some(target_node),
            stack_next,
            from: from.clone(),
            to,
            code: trd.code,
            flags: trd.flags,
            data_size: trd.data_size as _,
            data_address,
            offsets_size: trd.offsets_size as _,
            links: Links::new(),
            free_allocation: AtomicBool::new(true),
            sender_euid: from.process.task.euid(),
            txn_security_ctx_off,
        })?);

        // SAFETY: `inner` is pinned when `tr` is.
        let inner = unsafe { tr.as_mut().map_unchecked_mut(|t| &mut t.inner) };
        kernel::spinlock_init!(inner, "Transaction::inner");

        // SAFETY: `pi_node` is pinned when `tr` is.
        let pi_node = unsafe { tr.as_mut().map_unchecked_mut(|t| &mut t.pi_node) };
        pi_node.init();

        Ok(tr.into())
    }

    pub(crate) fn new_reply(
        from: &Ref<Thread>,
        to: Ref<Process>,
        tr: &BinderTransactionDataSg,
        allow_fds: bool,
    ) -> BinderResult<Ref<Self>> {
        let trd = &tr.transaction_data;
        let mut alloc = match from.copy_transaction_data(&to, tr, allow_fds, None) {
            Ok(alloc) => alloc,
            Err(err) => {
                pr_warn!("Failure in copy_transaction_data: {:?}", err);
                return Err(err);
            },
        };
        if trd.flags & TF_CLEAR_BUF != 0 {
            alloc.set_info_clear_on_drop();
        }
        let data_address = alloc.ptr;
        let file_list = alloc.take_file_list();
        alloc.keep_alive();
        let mut tr = Pin::from(UniqueRef::try_new(Self {
            // SAFETY: `spinlock_init` is called below.
            inner: unsafe { SpinLock::new(TransactionInner { file_list }) },
            // SAFETY: `PINode::init` is called below.
            pi_node: unsafe { crate::pi::PINode::new(&from.task) },
            target_node: None,
            stack_next: None,
            from: from.clone(),
            to,
            code: trd.code,
            flags: trd.flags,
            data_size: trd.data_size as _,
            data_address,
            offsets_size: trd.offsets_size as _,
            links: Links::new(),
            free_allocation: AtomicBool::new(true),
            sender_euid: from.process.task.euid(),
            txn_security_ctx_off: None,
        })?);

        // SAFETY: `inner` is pinned when `tr` is.
        let pinned = unsafe { tr.as_mut().map_unchecked_mut(|t| &mut t.inner) };
        kernel::spinlock_init!(pinned, "Transaction::inner");

        // SAFETY: `pi_node` is pinned when `tr` is.
        let pi_node = unsafe { tr.as_mut().map_unchecked_mut(|t| &mut t.pi_node) };
        pi_node.init();

        Ok(tr.into())
    }

    /// Determines if the transaction is stacked on top of the given transaction.
    pub(crate) fn is_stacked_on(&self, onext: &Option<Ref<Self>>) -> bool {
        match (&self.stack_next, onext) {
            (None, None) => true,
            (Some(stack_next), Some(next)) => Ref::ptr_eq(stack_next, next),
            _ => false,
        }
    }

    /// Returns a pointer to the next transaction on the transaction stack, if there is one.
    pub(crate) fn clone_next(&self) -> Option<Ref<Self>> {
        let next = self.stack_next.as_ref()?;
        Some(next.clone())
    }

    /// Searches in the transaction stack for a thread that belongs to the target process. This is
    /// useful when finding a target for a new transaction: if the node belongs to a process that
    /// is already part of the transaction stack, we reuse the thread.
    fn find_target_thread(&self) -> Option<Ref<Thread>> {
        let mut it = &self.stack_next;
        while let Some(transaction) = it {
            if Ref::ptr_eq(&transaction.from.process, &self.to) {
                return Some(transaction.from.clone());
            }
            it = &transaction.stack_next;
        }
        None
    }

    /// Searches in the transaction stack for a transaction originating at the given thread.
    pub(crate) fn find_from(&self, thread: &Thread) -> Option<Ref<Transaction>> {
        let mut it = &self.stack_next;
        while let Some(transaction) = it {
            if core::ptr::eq(thread, transaction.from.as_ref()) {
                return Some(transaction.clone());
            }

            it = &transaction.stack_next;
        }
        None
    }

    /// Submits the transaction to a work queue. Use a thread if there is one in the transaction
    /// stack, otherwise use the destination process.
    ///
    /// Not used for replies.
    pub(crate) fn submit(self: Ref<Self>) -> BinderResult {
        if self.flags & TF_ONE_WAY != 0 {
            if let Some(target_node) = self.target_node.clone() {
                target_node.submit_oneway(self)?;
                return Ok(());
            } else {
                pr_err!("Failed to submit oneway transaction to node.");
            }
        }

        if let Some(thread) = self.find_target_thread() {
            // We don't call `set_owner` here because this condition only triggers when we are
            // sending the transaction to a thread already part of this transaction stack, and the
            // target thread is probably waiting for *us* on an rtmutex earlier in the transaction
            // stack. This means that setting the owner now would look like a deadlock to the
            // rtmutex.
            //
            // Instead, we rely on `set_owner` being called by the target thread itself once it has
            // been woken up and stopped waiting for us.
            thread.push_work(self)?;

            Ok(())
        } else {
            let process = self.to.clone();
            process.push_new_transaction(self)
        }
    }

    /// Prepares the file list for delivery to the caller.
    fn prepare_file_list(&self) -> Result<List<Box<FileInfo>>> {
        // Get list of files that are being transferred as part of the transaction.
        let mut file_list = core::mem::replace(&mut self.inner.lock().file_list, List::new());

        // If the list is non-empty, prepare the buffer.
        if !file_list.is_empty() {
            let alloc = self.to.buffer_get(self.data_address).ok_or(ESRCH)?;
            let cleanup = ScopeGuard::new(|| {
                self.free_allocation.store(false, Ordering::Relaxed);
            });

            let mut it = file_list.cursor_front_mut();
            while let Some(file_info) = it.current() {
                let reservation = FileDescriptorReservation::new(bindings::O_CLOEXEC)?;
                alloc.write(file_info.buffer_offset, &reservation.reserved_fd())?;
                file_info.reservation = Some(reservation);
                it.move_next();
            }

            alloc.keep_alive();
            cleanup.dismiss();
        }

        Ok(file_list)
    }

    /// Called when assigning the transaction to a thread from the sender.
    ///
    /// If the target has no available threads, then this is done inside `do_work` when a thread
    /// picks it up instead.
    pub(crate) fn set_pi_owner(&self, owner: &Task) {
        if self.flags & TF_ONE_WAY == 0 {
            self.pi_node.set_owner(owner);
        }
    }

    /// Called on transactions when a reply has been delivered.
    ///
    /// Should be called from the thread that sent the reply, after waking up the sleeping thread.
    pub(crate) fn set_reply_delivered(&self) {
        self.pi_node.owner_is_done();
    }
}

impl DeliverToRead for Transaction {
    fn do_work(self: Ref<Self>, thread: &Thread, writer: &mut UserSlicePtrWriter) -> Result<bool> {
        let send_failed_reply = ScopeGuard::new(|| {
            if self.target_node.is_some() && self.flags & TF_ONE_WAY == 0 {
                let reply = Either::Right(BR_FAILED_REPLY);
                self.from.deliver_reply(reply, &self);
            }
        });

        if self.target_node.is_some() && self.flags & TF_ONE_WAY == 0 {
            // Not a reply and not one-way.
            self.pi_node.set_owner(&Task::current());
        }

        let mut file_list = if let Ok(list) = self.prepare_file_list() {
            list
        } else {
            // On failure to process the list, we send a reply back to the sender and ignore the
            // transaction on the recipient.
            return Ok(true);
        };

        let mut tr_sec = BinderTransactionDataSecctx::default();
        let tr = tr_sec.tr_data();

        if let Some(target_node) = &self.target_node {
            let (ptr, cookie) = target_node.get_id();
            tr.target.ptr = ptr as _;
            tr.cookie = cookie as _;
        };

        tr.code = self.code;
        tr.flags = self.flags;
        tr.data_size = self.data_size as _;
        tr.data.ptr.buffer = self.data_address as _;
        tr.offsets_size = self.offsets_size as _;
        if tr.offsets_size > 0 {
            tr.data.ptr.offsets = (self.data_address + ptr_align(self.data_size)) as _;
        }

        tr.sender_euid = self.sender_euid.into_uid_in_current_ns();

        tr.sender_pid = 0;
        if self.target_node.is_some() && self.flags & TF_ONE_WAY == 0 {
            // Not a reply and not one-way.
            let from_proc = &*self.from.process;
            if !from_proc.is_dead() {
                let pid = from_proc.task.pid_in_current_ns();
                tr.sender_pid = pid;
            }
        }

        let code = if self.target_node.is_none() {
            BR_REPLY
        } else {
            if self.txn_security_ctx_off.is_some() {
                BR_TRANSACTION_SEC_CTX
            } else {
                BR_TRANSACTION
            }
        };

        // Write the transaction code and data to the user buffer.
        writer.write(&code)?;
        if let Some(off) = self.txn_security_ctx_off {
            tr_sec.secctx = (self.data_address + off) as u64;
            writer.write(&tr_sec)?;
        } else {
            writer.write(&*tr)?;
        }

        // Dismiss the completion of transaction with a failure. No failure paths are allowed from
        // here on out.
        send_failed_reply.dismiss();

        // Commit all files.
        {
            let mut it = file_list.cursor_front_mut();
            while let Some(file_info) = it.current() {
                if let Some(reservation) = file_info.reservation.take() {
                    if let Some(file) = file_info.file.take() {
                        reservation.commit(file);
                    }
                }

                it.move_next();
            }
        }

        // When `drop` is called, we don't want the allocation to be freed because it is now the
        // user's reponsibility to free it.
        //
        // `drop` is guaranteed to see this relaxed store because `Ref` guarantess that everything
        // that happens when an object is referenced happens-before the eventual `drop`.
        self.free_allocation.store(false, Ordering::Relaxed);

        // When this is not a reply and not an async transaction, update `current_transaction`. If
        // it's a reply, `current_transaction` has already been updated appropriately.
        if self.target_node.is_some() && tr_sec.transaction_data.flags & TF_ONE_WAY == 0 {
            thread.set_current_transaction(self);
        }

        Ok(false)
    }

    fn cancel(self: Ref<Self>) {
        // If this is not a reply or oneway transaction, then send a dead reply.
        if self.target_node.is_some() && self.flags & TF_ONE_WAY == 0 {
            let reply = Either::Right(BR_DEAD_REPLY);
            self.from.deliver_reply(reply, &self);
        }
    }

    fn get_links(&self) -> &Links<dyn DeliverToRead> {
        &self.links
    }

    fn should_sync_wakeup(&self) -> bool {
        self.flags & TF_ONE_WAY == 0
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if self.free_allocation.load(Ordering::Relaxed) {
            self.to.buffer_get(self.data_address);
        }
    }
}

pub(crate) struct FileInfo {
    links: Links<FileInfo>,

    /// The file for which a descriptor will be created in the recipient process.
    file: Option<ARef<File>>,

    /// The file descriptor reservation on the recipient process.
    reservation: Option<FileDescriptorReservation>,

    /// The offset in the buffer where the file descriptor is stored.
    buffer_offset: usize,
}

impl FileInfo {
    pub(crate) fn new(file: ARef<File>, buffer_offset: usize) -> Self {
        Self {
            file: Some(file),
            reservation: None,
            buffer_offset,
            links: Links::new(),
        }
    }
}

impl GetLinks for FileInfo {
    type EntryType = Self;

    fn get_links(data: &Self::EntryType) -> &Links<Self::EntryType> {
        &data.links
    }
}
