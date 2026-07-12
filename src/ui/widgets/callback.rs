use std::cell::OnceCell;
use std::rc::Rc;

/// A set-once UI callback slot: the one idiom behind every `connect_*`
/// method on the app's components. `emit` before `set` is a silent no-op —
/// widgets may fire during construction, before wiring completes — and a
/// second `set` is ignored, matching `OnceCell` semantics.
pub struct Callback<T>(OnceCell<Rc<dyn Fn(T)>>);

impl<T> Callback<T> {
    pub fn new() -> Self {
        Self(OnceCell::new())
    }

    /// Registers the callback. Ignored if one is already registered.
    pub fn set(&self, f: impl Fn(T) + 'static) {
        let _ = self.0.set(Rc::new(f));
    }

    /// Invokes the registered callback with `value`, if one is registered.
    pub fn emit(&self, value: T) {
        if let Some(f) = self.0.get() {
            f(value);
        }
    }

    /// The registered handler, for callers that need to move it into a
    /// closure of their own (e.g. a context-menu action built lazily).
    pub fn handler(&self) -> Option<Rc<dyn Fn(T)>> {
        self.0.get().cloned()
    }
}

impl<T> Default for Callback<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn emit_before_set_is_a_no_op() {
        let callback: Callback<u32> = Callback::new();

        // Nothing to observe — the assertion is that this doesn't panic.
        callback.emit(7);

        assert!(callback.handler().is_none());
    }

    #[test]
    fn emit_after_set_invokes_the_callback() {
        let callback: Callback<u32> = Callback::new();
        let received = Rc::new(Cell::new(0u32));

        let received_in_callback = Rc::clone(&received);
        callback.set(move |track_number| received_in_callback.set(track_number));
        callback.emit(7);

        assert_eq!(received.get(), 7);
    }

    #[test]
    fn second_set_is_ignored() {
        let callback: Callback<u32> = Callback::new();
        let received = Rc::new(Cell::new(0u32));

        let first = Rc::clone(&received);
        callback.set(move |value| first.set(value));
        callback.set(|_| panic!("second registration must not win"));
        callback.emit(1998);

        assert_eq!(received.get(), 1998);
    }

    #[test]
    fn handler_returns_none_when_unset() {
        let callback: Callback<String> = Callback::new();

        assert!(callback.handler().is_none());
    }

    #[test]
    fn handler_returns_the_registered_callback() {
        let callback: Callback<String> = Callback::new();
        let received = Rc::new(Cell::new(false));

        let received_in_callback = Rc::clone(&received);
        callback.set(move |_| received_in_callback.set(true));
        let handler = callback.handler().expect("callback was just set");
        handler("Boards of Canada".to_string());

        assert!(received.get());
    }
}
