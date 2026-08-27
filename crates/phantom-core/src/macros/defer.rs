//! Running code on the way out of a scope, however it is left.

/// Runs `$body` when the enclosing scope ends, however it ends.
///
/// The closure is `FnOnce`, so the body may consume what it captures — which
/// is what lets [`scope_restore!`] move a non-`Copy` value back into place.
#[macro_export]
macro_rules! defer {
    ($body:block) => {
        struct _Defer_<F: FnOnce()> {
            closure: Option<F>,
        }

        impl<F: FnOnce()> Drop for _Defer_<F> {
            fn drop(&mut self) {
                if let Some(closure) = self.closure.take() {
                    closure();
                }
            }
        }

        let _defer_ = _Defer_ {
            closure: Some(|| $body),
        };
    };

    ($body:expr_2021) => {
        $crate::defer! {{ $body }}
    };
}

/// Sets `*$val` to `$ours` for the rest of the scope, restoring the previous
/// value on the way out.
///
/// `$val` must be a `&mut T` binding, and it stays uniquely borrowed by the
/// deferred restore until the scope ends.
#[macro_export]
macro_rules! scope_restore {
    ($val:ident, $ours:expr_2021) => {
        let theirs = $crate::exchange($val, $ours);
        $crate::defer! {{ *$val = theirs; }};
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn defer_runs_at_scope_end() {
        let mut ran = false;
        {
            defer! {{ ran = true; }};
        }
        assert!(ran);
    }

    #[test]
    fn scope_restore_restores() {
        let mut val = String::from("theirs");
        {
            let val = &mut val;
            scope_restore!(val, String::from("ours"));
        }
        assert_eq!(val, "theirs");
    }
}
