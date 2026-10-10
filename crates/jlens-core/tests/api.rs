//! Properties of the public API that the compiler checks: which types can cross threads. Losing
//! one of them is a breaking change; `cargo-semver-checks` in CI catches it too.

use jlens_core::audit::{AuditOptions, AuditTree, Explanation};
use jlens_core::{
    Annotation, Ast, Bindings, CancelToken, Diagnostic, Document, Error, Expression, FunctionInfo,
    HostFunction, Options,
};

fn send_sync<T: Send + Sync + 'static>() {}

#[test]
fn shared_types_are_send_and_sync() {
    // Compile once, evaluate on many threads.
    send_sync::<Expression>();
    send_sync::<Ast>();
    send_sync::<Bindings>();
    send_sync::<HostFunction>();
    send_sync::<Options>();
    send_sync::<CancelToken>();
    // Results and errors can be sent back to another thread, and boxed as `dyn Error + Send +
    // Sync` (as `anyhow` does).
    send_sync::<Error>();
    send_sync::<Diagnostic>();
    send_sync::<Annotation>();
    send_sync::<FunctionInfo>();
    send_sync::<AuditOptions>();
    send_sync::<AuditTree>();
    send_sync::<Explanation>();
    let _: Box<dyn std::error::Error + Send + Sync> =
        Box::new(Expression::compile("(").expect_err("an unterminated block is a syntax error"));
}

/// `Document` holds the evaluator's internal values, which use `Rc` so that evaluation needs no
/// atomic reference counting. It is neither `Send` nor `Sync`, as its documentation says: each
/// thread converts its own. If it ever becomes `Send`, update that documentation and this test.
#[test]
fn document_stays_on_its_thread() {
    trait AmbiguousIfSend<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    #[allow(dead_code)]
    struct IsSend;
    impl<T: ?Sized + Send> AmbiguousIfSend<IsSend> for T {}

    trait AmbiguousIfSync<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSync<()> for T {}
    #[allow(dead_code)]
    struct IsSync;
    impl<T: ?Sized + Sync> AmbiguousIfSync<IsSync> for T {}

    // Each call resolves only while exactly one impl applies, that is, while `Document` is
    // neither `Send` nor `Sync`.
    <Document as AmbiguousIfSend<_>>::check();
    <Document as AmbiguousIfSync<_>>::check();
}
