# After step 33: the size fix, an early part of the performance pass

The size fix changes how the shipped binary is built, not the tests: `release` at `opt-level = "s"` touches no test job, which builds in the `ci` profile. What it adds to a pull request's run is the `pr-size` job ([testing-and-ci.md](../../testing-and-ci.md), "Each pull request's binary size"), which starts after the four required jobs and blocks nothing; the tables below give its time beside theirs. Its sizes and speed figures are in [measurements/binary-size.md](../../measurements/binary-size.md).

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
<!-- end of what cargo xtask ci-times wrote -->
