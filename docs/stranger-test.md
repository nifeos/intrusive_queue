# The stranger cadence

The release's whole claim is that a stranger, given this repository and nothing else, can build
the crate and run its proofs from the README alone. A real agent runs that check on a
cadence, and the weekly `stranger cadence` job fails when the
last run is past 28 days.

What a run is:

1. Clone the repository fresh.
2. Follow the README and nothing else: build, test, and run the proofs.
   The README must name the build and test commands (with any feature a test target needs), the
   bare-metal build with `rustup target add`, the Kani install before the first `cargo kani`, how
   long the proofs take and how to run one harness, what a passing run prints that looks alarming,
   how to run and revert one falsification patch, and how to check the minimum Rust version.
   Each one it lacks is a finding.
3. Note every place the README lied, omitted, or assumed. Those are findings against the README,
   not against the stranger.
4. Record the date on the line below.

Last run: 2026-11-22

## What the last run did

A fresh clone and the README, on a machine that already had Rust, the 1.85 toolchain and Kani
0.68.0. It followed "Building and testing", "Running the proofs", the falsification commands and
"Minimum Rust version" command by command.

- `cargo test` passes: 10 unit tests, 5 doctests and 2 compile-fail doctests. The three bare-metal
  builds finish.
- `cargo kani` ends with `Complete - 4 successfully verified harnesses, 0 failures, 4 total.`
  after under three minutes, with the warnings and `aborting path on assume(false)` lines the
  README says to expect. The one-harness command verifies one harness.
- The README's three falsification commands do what it says: the patched run names the failed
  assertion and ends with `0 successfully verified harnesses, 1 failures, 1 total.`, and the
  reverse patch leaves the tree clean.
- At Rust 1.85, the stated minimum, `cargo test` and the three bare-metal builds pass.

The run found one gap: the README did not say what a passing `cargo test` prints, three result
lines a stranger could not tell apart. The README now names each line.

Result: pass.
