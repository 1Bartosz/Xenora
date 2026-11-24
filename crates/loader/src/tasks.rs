// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded manual work and completion ports for synthetic runtime services.
//! The caller supplies monotonic time, allowing deterministic dispatch in tests.

use std::collections::VecDeque;
use std::time::Duration;

/// Identifier issued by a task queue and never reused within that queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskId(u64);

/// Why the queue could not accept a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    Full,
    IdExhausted,
}

/// Result delivered once through the completion port.
#[derive(Debug, PartialEq, Eq)]
pub enum TaskResult<T, E> {
    Finished(Result<T, E>),
    Cancelled,
    TimedOut,
}

type Work<T, E> = Box<dyn FnOnce() -> Result<T, E> + Send>;
type Callback<T, E> = Box<dyn FnOnce(TaskId, TaskResult<T, E>) + Send>;

struct Pending<T, E> {
    id: TaskId,
    deadline: Option<Duration>,
    work: Work<T, E>,
    callback: Callback<T, E>,
}

struct Completion<T, E> {
    id: TaskId,
    result: TaskResult<T, E>,
    callback: Callback<T, E>,
}

/// A manual task queue whose capacity covers pending work and undelivered completions.
pub struct TaskQueue<T, E> {
    capacity: usize,
    last_id: u64,
    pending: VecDeque<Pending<T, E>>,
    completions: VecDeque<Completion<T, E>>,
}

impl<T, E> TaskQueue<T, E> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            last_id: 0,
            pending: VecDeque::new(),
            completions: VecDeque::new(),
        }
    }

    /// Enqueue work and a completion callback. The deadline uses the same clock as dispatch.
    pub fn submit(
        &mut self,
        deadline: Option<Duration>,
        work: impl FnOnce() -> Result<T, E> + Send + 'static,
        callback: impl FnOnce(TaskId, TaskResult<T, E>) + Send + 'static,
    ) -> Result<TaskId, SubmitError> {
        if self.pending.len() + self.completions.len() >= self.capacity {
            return Err(SubmitError::Full);
        }
        self.last_id = self
            .last_id
            .checked_add(1)
            .ok_or(SubmitError::IdExhausted)?;
        let id = TaskId(self.last_id);
        self.pending.push_back(Pending {
            id,
            deadline,
            work: Box::new(work),
            callback: Box::new(callback),
        });
        Ok(id)
    }

    /// Cancel work that is still queued. The completion port still receives its callback.
    pub fn cancel(&mut self, id: TaskId) -> bool {
        let Some(index) = self.pending.iter().position(|task| task.id == id) else {
            return false;
        };
        let task = self
            .pending
            .remove(index)
            .expect("position in pending queue");
        self.completions.push_back(Completion {
            id,
            result: TaskResult::Cancelled,
            callback: task.callback,
        });
        true
    }

    /// Run one queued operation. An expired operation completes without running.
    pub fn dispatch_work(&mut self, now: Duration) -> bool {
        let Some(task) = self.pending.pop_front() else {
            return false;
        };
        let result = if task.deadline.is_some_and(|deadline| now >= deadline) {
            TaskResult::TimedOut
        } else {
            TaskResult::Finished((task.work)())
        };
        self.completions.push_back(Completion {
            id: task.id,
            result,
            callback: task.callback,
        });
        true
    }

    /// Invoke one callback on the caller's thread.
    pub fn dispatch_completion(&mut self) -> bool {
        let Some(completion) = self.completions.pop_front() else {
            return false;
        };
        (completion.callback)(completion.id, completion.result);
        true
    }
}

#[cfg(test)]
#[test]
fn manual_ports_deliver_errors_cancellation_and_deadlines_once() {
    use std::sync::mpsc::channel;

    let mut queue = TaskQueue::<u32, u32>::new(3);
    let (sender, receiver) = channel();
    let callback = || {
        let sender = sender.clone();
        move |id, result| sender.send((id, result)).unwrap()
    };
    let first = queue.submit(None, || Err(7), callback()).unwrap();
    let cancelled = queue
        .submit(None, || panic!("cancelled"), callback())
        .unwrap();
    let deadline = Duration::from_millis(10);
    let expired = queue
        .submit(Some(deadline), || panic!("expired"), callback())
        .unwrap();
    assert_eq!(
        queue.submit(None, || Ok(1), callback()),
        Err(SubmitError::Full)
    );
    assert!(queue.cancel(cancelled));
    assert!(!queue.cancel(cancelled));
    assert!(queue.dispatch_work(Duration::ZERO));
    assert!(queue.dispatch_work(deadline));
    assert!(!queue.dispatch_work(deadline));
    assert_eq!(
        queue.submit(None, || Ok(1), callback()),
        Err(SubmitError::Full)
    );
    assert!(receiver.try_recv().is_err());
    for expected in [
        (cancelled, TaskResult::Cancelled),
        (first, TaskResult::Finished(Err(7))),
        (expired, TaskResult::TimedOut),
    ] {
        assert!(queue.dispatch_completion());
        assert_eq!(receiver.try_recv().unwrap(), expected);
    }
    assert!(!queue.dispatch_completion());
    let next = queue.submit(None, || Ok(1), callback()).unwrap();
    assert_ne!(next, first);
    assert!(queue.dispatch_work(deadline));
    assert!(queue.dispatch_completion());
    assert_eq!(
        receiver.try_recv().unwrap(),
        (next, TaskResult::Finished(Ok(1)))
    );
}
