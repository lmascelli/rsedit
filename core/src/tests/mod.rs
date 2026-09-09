mod bug_fix_tests;
mod command_test;
mod deadlock_tests;
mod debug_lisp_tests;
mod deletion_tests;
mod echo_tests;
mod frame_snapshot_tests;
mod fuel_tests;
mod minibuffer_lisp_tests;
mod movement_tests;
/// Performance characterisation of the whole editor, interpreter included.
/// One submodule, one runner, two reports -- see `perf::mod` for the rationale.
mod perf;
mod undo_tests;
