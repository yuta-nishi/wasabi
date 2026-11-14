//! A cooperative executor for `async` tasks.
//!
//! A task is a future boxed and pinned on the heap, plus the flag that says it
//! wants to be polled again. Waking works through [`alloc::task::Wake`], so the
//! `Waker` a future receives can be cloned and kept: waking simply sets the
//! flag, and the executor polls the task on its next round.
//!
//! Nothing here is preemptive. A future must return `Pending` for other tasks to
//! get a turn, which is what [`yield_now`] is for.

use crate::x86::hlt;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::cell::UnsafeCell;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;
use core::task::Context;
use core::task::Poll;
use core::task::Waker;

/// One future the executor can poll.
pub struct Task {
    future: UnsafeCell<Pin<Box<dyn Future<Output = ()>>>>,
    woken: AtomicBool,
}

impl Task {
    fn new(future: impl Future<Output = ()> + 'static) -> Self {
        Self {
            future: UnsafeCell::new(Box::pin(future)),
            woken: AtomicBool::new(true),
        }
    }

    fn poll(self: &Arc<Self>) {
        let waker = Waker::from(self.clone());
        let mut context = Context::from_waker(&waker);
        // SAFETY: the executor polls a task from one place at a time, and the
        // future is pinned, so handing out `&mut` here cannot alias a poll.
        let future = unsafe { &mut *self.future.get() };
        let _ = future.as_mut().poll(&mut context);
    }
}

// SAFETY: `future` is only touched by `poll`, which the executor calls from one
// thread at a time; everything else is atomic.
unsafe impl Send for Task {}
// SAFETY: as above; there is no OS thread that could race with the executor.
unsafe impl Sync for Task {}

impl alloc::task::Wake for Task {
    fn wake(self: Arc<Self>) {
        self.woken.store(true, Ordering::Release);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::Release);
    }
}

/// Runs `async` tasks one at a time, in spawn order, whenever they are woken.
pub struct Executor {
    tasks: RefCell<Vec<Arc<Task>>>,
}

// SAFETY: the OS is single-threaded and nothing here is touched from an
// interrupt yet; the `RefCell` only guards the task list against re-entrant
// borrows.
unsafe impl Sync for Executor {}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor {
    pub const fn new() -> Self {
        Self {
            tasks: RefCell::new(Vec::new()),
        }
    }

    /// Adds a future and returns the task that owns it.
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) -> Arc<Task> {
        let task = Arc::new(Task::new(future));
        self.tasks.borrow_mut().push(task.clone());
        task
    }

    /// Polls every task that has been woken and reports how many ran.
    pub fn run_ready(&self) -> usize {
        let mut ran = 0;
        let count = self.tasks.borrow().len();
        for index in 0..count {
            let task = self.tasks.borrow()[index].clone();
            if task.woken.swap(false, Ordering::AcqRel) {
                task.poll();
                ran += 1;
            }
        }
        ran
    }

    /// Runs tasks forever, idling the CPU while none of them is ready.
    pub fn run(&self) -> ! {
        loop {
            if self.run_ready() == 0 {
                hlt();
            }
        }
    }
}

/// A future that returns `Pending` once, waking itself, so that the executor
/// gets to poll the other tasks first.
pub struct YieldNow {
    yielded: bool,
}

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Gives the other tasks a chance to run.
pub fn yield_now() -> YieldNow {
    YieldNow { yielded: false }
}
