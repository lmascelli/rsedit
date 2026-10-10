# Performance checks

The editor suite is excluded from ordinary `cargo test` runs. Run it alone so
unit tests do not compete for CPU time:

```sh
PERF_MACHINE=my-workstation cargo test --manifest-path core/Cargo.toml tests::perf::editor_performance_suite -- --ignored --exact --test-threads=1
```

Each elapsed-time row reports the median and min–max of nine measured samples.
Stateful workloads start from a fresh fixture for every sample. Timing ratios
and targets are advisory; only behavioral checks and exact fuel-accounting
checks fail the command. Fuel in this report covers interpreter charges along
the editor command path, not the Rust cost of buffer operations or rendering.

Reports (`performance-cost.txt` and `performance-timing.txt`) are generated and
ignored by Git. Baselines are separate versioned files. After reviewing a run
on the intended reference machine, save its baseline explicitly:

```sh
PERF_MACHINE=my-workstation PERF_BLESS=1 cargo test --manifest-path core/Cargo.toml tests::perf::editor_performance_suite -- --ignored --exact --test-threads=1
```

Review and commit `performance-cost.baseline` and
`performance-timing.baseline`. Future runs with the same `PERF_MACHINE` label,
build profile, target and CPU count show deltas against those files. Baselines
are left untouched unless `PERF_BLESS=1` is set.

When applying both repository patches, merge the `risp` patch first. Then run
`cargo update -p risp` from the rsedit root before building; the current lockfile
names the previous `main` commit.
