<!-- Derived from the Firsthand master build spec. Internal working document. -->

# Track Q

**Track Q: Cold-start QA** (branch `track/q-coldstart`, ~1.5 hours, fresh context): clones the repo fresh into a temp directory, follows `README.md` and `docs/quickstart.md` literally on a scratch repo with `fh-fakeagent`, times it, records every friction point in `docs/internal/coldstart-report.md`, and files fixes. Time to first verified handoff must be under 10 minutes including install from the release artifact or `cargo install --git`.
