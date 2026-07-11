/// Application events flowing on the bus.
///
/// Each variant represents something that happened in the system.
/// Modules publish events; other modules react to them via
/// registered handlers — without ever knowing each other directly.
///
/// This enum will be extended phase by phase (moves, analysis, UI, …).
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Request for a clean shutdown of the application.
    Shutdown,
}

// ---------------------------------------------------------------------------
// EventBus
// ---------------------------------------------------------------------------

use std::sync::{Arc, Mutex, PoisonError};

/// Type of a handler: function called on every published event.
///
/// `Arc` rather than `Box` (code audit 04/07/2026, point 3): lets
/// [`EventBus::publish`] clone the handler list (cheap clone,
/// just reference counters) and iterate over it **outside** the lock —
/// see the `publish` doc for the two problems this solves.
type Handler = Arc<dyn Fn(&AppEvent) + Send + Sync + 'static>;

/// Central event bus (pub/sub pattern).
///
/// - **Publishing**: `publish(&event)` notifies all subscribers.
/// - **Subscribing**: `subscribe(fn)` registers a handler.
/// - **Thread-safe**: shareable via `Arc<EventBus>`.
///
/// # Example
///
/// ```rust
/// use core::events::{AppEvent, EventBus};
/// use std::sync::{Arc, Mutex};
///
/// let bus = Arc::new(EventBus::new());
/// let received = Arc::new(Mutex::new(false));
///
/// let received_clone = Arc::clone(&received);
/// bus.subscribe(move |_event| {
///     *received_clone.lock().unwrap() = true;
/// });
///
/// bus.publish(&AppEvent::Shutdown);
/// assert!(*received.lock().unwrap());
/// ```
#[derive(Default)]
pub struct EventBus {
    handlers: Mutex<Vec<Handler>>,
}

impl EventBus {
    /// Creates a new empty bus.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: Mutex::new(Vec::new()),
        }
    }

    /// Registers a handler called on every published event.
    ///
    /// Recovers from a poisoned lock (code audit 04/07/2026, point 3)
    /// rather than panicking — consistent with the rest of the code (`prefs.rs`,
    /// `debug_log.rs`, `i18n`…): a panic in a handler must never
    /// *permanently* block any subsequent subscription/publication.
    pub fn subscribe<F>(&self, handler: F)
    where
        F: Fn(&AppEvent) + Send + Sync + 'static,
    {
        self.handlers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(handler));
    }

    /// Publishes an event: all registered handlers are called
    /// in registration order, synchronously.
    ///
    /// The lock is released **before** calling the handlers (code audit
    /// 04/07/2026, point 3): the list is cloned (cheap `Arc`
    /// clone) then iterated over outside the lock. Fixes two problems from
    /// the previous implementation, which held the lock during the call to
    /// each handler:
    /// - a handler that republished an event (or resubscribed) from
    ///   itself caused a deadlock (std `Mutex` is not reentrant);
    /// - a panic in a single handler poisoned the mutex, and with
    ///   the old non-recoverable `.expect()`, **all** subsequent calls to
    ///   `subscribe`/`publish` would in turn panic for the rest of the
    ///   process.
    pub fn publish(&self, event: &AppEvent) {
        let handlers: Vec<Handler> = self
            .handlers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();

        for handler in &handlers {
            handler(event);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_subscribe_and_publish() {
        let bus = EventBus::new();
        let received = Arc::new(Mutex::new(false));

        let received_clone = Arc::clone(&received);
        bus.subscribe(move |_event| {
            *received_clone.lock().unwrap() = true;
        });

        bus.publish(&AppEvent::Shutdown);

        assert!(
            *received.lock().unwrap(),
            "Le handler aurait dû recevoir AppEvent::Shutdown"
        );
    }

    #[test]
    fn test_multiple_subscribers() {
        let bus = EventBus::new();
        let counter = Arc::new(Mutex::new(0u32));

        for _ in 0..3 {
            let counter_clone = Arc::clone(&counter);
            bus.subscribe(move |_event| {
                *counter_clone.lock().unwrap() += 1;
            });
        }

        bus.publish(&AppEvent::Shutdown);

        assert_eq!(
            *counter.lock().unwrap(),
            3,
            "Les 3 handlers auraient dû être appelés"
        );
    }

    #[test]
    fn test_no_subscribers_no_panic() {
        let bus = EventBus::new();
        // Publishing with no subscribers must not panic
        bus.publish(&AppEvent::Shutdown);
    }

    #[test]
    fn test_shared_bus_via_arc() {
        let bus = Arc::new(EventBus::new());
        let received = Arc::new(Mutex::new(false));

        let received_clone = Arc::clone(&received);
        bus.subscribe(move |_| {
            *received_clone.lock().unwrap() = true;
        });

        // Simulates another thread publishing on the shared bus
        let bus_clone = Arc::clone(&bus);
        std::thread::spawn(move || {
            bus_clone.publish(&AppEvent::Shutdown);
        })
        .join()
        .unwrap();

        assert!(*received.lock().unwrap());
    }

    // ── Code audit 04/07/2026, point 3: poisoning/reentrancy non-regression ──

    #[test]
    fn test_lock_not_poisoned_after_handler_panics_during_publish() {
        let bus = EventBus::new();
        bus.subscribe(|_event| panic!("handler volontairement en échec pour le test"));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bus.publish(&AppEvent::Shutdown);
        }));
        assert!(result.is_err(), "le handler devait paniquer");

        // With the old implementation (lock held during handler calls
        // + non-recoverable `.expect()`), this panic would have *permanently*
        // poisoned the mutex: the call below would in turn have
        // panicked. Now the lock is never held during the execution
        // of the handlers: it can no longer be poisoned by a handler
        // panic.
        bus.subscribe(|_event| {});
    }

    #[test]
    fn test_reentrant_subscribe_from_handler_does_not_deadlock() {
        let bus = Arc::new(EventBus::new());
        let bus_clone = Arc::clone(&bus);
        let reentered = Arc::new(Mutex::new(false));
        let reentered_clone = Arc::clone(&reentered);

        // A handler that subscribes again from within itself: with the old
        // implementation (lock held throughout `publish`), this would have
        // caused a deadlock (std `Mutex` is not reentrant).
        bus.subscribe(move |_event| {
            let reentered_inner = Arc::clone(&reentered_clone);
            bus_clone.subscribe(move |_| {
                *reentered_inner.lock().unwrap() = true;
            });
        });

        bus.publish(&AppEvent::Shutdown); // must not block
        // The new subscriber (added during the first publish) must be
        // present for the next one.
        bus.publish(&AppEvent::Shutdown);

        assert!(*reentered.lock().unwrap());
    }
}
