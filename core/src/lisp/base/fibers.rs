//! Work that can be put down and picked up.
//!
//! A fiber is a suspended evaluation: `resume` gives it a turn and
//! `fiber-done-p` asks whether it has finished. What a *turn* costs, and
//! which thread it runs on, is the host's business -- see
//! `crate::background`.
use super::*;

const RESUME_DOC: &str = "(resume FIBER): Carry FIBER on from wherever it stopped, and return \
                 the value it stopped with. Returns nil once FIBER has nothing left to run.\n\n\
                 A fiber runs until it reaches a `yield\' or until its body is finished, \
                 whichever comes first. A `yield\' stops it *inside* a form, and the next \
                 `resume\' picks up at that exact spot with its variables as it left them.\n\n\
                 The body is a `progn\': `(fiber A B C)\' runs all three in one `resume\' unless \
                 one of them yields. Put a `yield\' between them to stop after each.\n\n\
                 An error inside a fiber ends it: the position it stopped at is gone, so there is \
                 nothing left to come back to, and `resume\' would otherwise silently restart it \
                 somewhere it had already been.\n\n\
                 Example:\n\
                 (setq f (fiber (log \"a\") (yield 1) (log \"b\") 2))\n\
                 (resume f) ; logs \"a\", => 1\n\
                 (resume f) ; logs \"b\", => 2\n\
                 (resume f) => nil ; fiber is done";

fn primitive_resume<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    let Some(LispExp::Fiber(shared_fiber)) = args.first() else {
        return match args.first() {
            None => Err(EvalError::WrongNumberOfArguments {
                expected: 1,
                got: 0,
            }),
            Some(other) => Err(EvalError::WrongArgumentType {
                expected: "Fiber".into(),
                got: other.clone(),
            }),
        };
    };

    // # Why the stack is taken out rather than borrowed
    //
    // Running it means running arbitrary Lisp, and this used to do that with
    // the fiber's write lock still held. That was survivable while a fiber
    // could only run one whole form to completion; it is not now. The first
    // thing that Lisp does that reaches its own fiber -- `(resume f)' on
    // itself, or asking whether it has finished -- deadlocks on a lock nobody
    // can see. So the lock answers one question, is dropped, and is taken
    // again to record the answer.
    //
    // Taking it also leaves the fiber holding *nothing* while it runs, which
    // is the honest state: a second `resume' arriving from another thread
    // finds an empty stack and does nothing, rather than starting the same
    // program a second time alongside the first.
    let frames = {
        let mut fiber = shared_fiber
            .0
            .write()
            .map_err(|_| EvalError::UncorrectFunctionDefinition)?;
        if fiber.is_done {
            return Ok(LispExp::nil());
        }
        std::mem::take(&mut fiber.pending)
    };
    if frames.is_empty() {
        let mut fiber = shared_fiber
            .0
            .write()
            .map_err(|_| EvalError::UncorrectFunctionDefinition)?;
        fiber.is_done = true;
        return Ok(LispExp::nil());
    }

    let outcome = resume_frames(frames, ctx);

    let mut fiber = shared_fiber
        .0
        .write()
        .map_err(|_| EvalError::UncorrectFunctionDefinition)?;
    match outcome {
        // Suspended again: the new stack is where it stopped.
        Err(EvalError::Yielded { value, frames }) => {
            fiber.pending = frames;
            Ok(value)
        }
        // Nothing left on the stack, so nothing left to come back to.
        Ok(value) => {
            fiber.is_done = true;
            Ok(value)
        }
        Err(other) => {
            // See the docstring: unwinding destroyed the position, so there is
            // no coming back. Resuming again would restart the program
            // somewhere it had already been.
            fiber.is_done = true;
            fiber.pending.clear();
            Err(other)
        }
    }
}

const FIBER_DONE_P_DOC: &str = "(fiber-done-p FIBER): t when FIBER has nothing left to run, nil \
                 while it still has. Signals if FIBER is not a fiber.\n\n\
                 What a scheduler asks so it can stop resuming one. A fiber that is merely \
                 *suspended* is not done -- it is in the middle of a form, waiting to be resumed \
                 -- so this is the only honest way to tell a finished task from a parked one; \
                 `resume\' answering nil cannot, since a fiber may perfectly well yield nil.\n\n\
                 Example:\n\
                 (while (not (fiber-done-p task)) (resume task))";

fn primitive_fiber_done_p<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    let Some(exp) = args.first() else {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    };
    match exp.fiber_is_done() {
        Some(done) => Ok(LispExp::boolean(done)),
        None => Err(EvalError::WrongArgumentType {
            expected: "Fiber".into(),
            got: exp.clone(),
        }),
    }
}

/// Register this module's primitives: work that can be put down and picked up.
///
/// Called by [`super::setup_base_env`]. Here rather than there because a
/// primitive's name, its implementation and its docstring are one fact in three
/// pieces, and they were a thousand lines apart.
pub(super) fn install<T: LispContext>(into: &Registry<T>) {
    into.function("fiber-done-p", primitive_fiber_done_p, FIBER_DONE_P_DOC);
    into.function("resume", primitive_resume, RESUME_DOC);
}
