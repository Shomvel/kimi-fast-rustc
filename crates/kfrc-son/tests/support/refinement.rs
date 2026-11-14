//! Explicit S→release-L relation. Do not use for strict R→S substitution or
//! passes that promise diagnostic preservation. A test supplies its value/ABI
//! relation and must still compare external effects on defined executions.
use kfrc_son::InterpretError;
use std::fmt::Debug;

fn language_ub(error: &InterpretError) -> bool {
    matches!(
        error,
        InterpretError::PoisonObserved(_) | InterpretError::UndefinedBehaviour(_)
    )
}

pub fn assert_release_refines<S: Debug, T: Debug>(
    source: &Result<S, InterpretError>,
    target: &Result<T, InterpretError>,
    defined_equivalent: impl FnOnce(&S, &T) -> bool,
) {
    if let Err(error) = source {
        assert!(
            language_ub(error),
            "source is not a language UB result: {error:?}"
        );
        if let Err(error) = target {
            assert!(
                language_ub(error),
                "target compiler/diagnostic failure: {error:?}"
            );
        }
        return;
    }
    let source = source.as_ref().unwrap();
    let target = target
        .as_ref()
        .unwrap_or_else(|error| panic!("defined source {source:?} became target error {error:?}"));
    assert!(
        defined_equivalent(source, target),
        "defined execution differs: source={source:?}, target={target:?}"
    );
}
