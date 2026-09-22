# ghma — git hide my ass

Reshape the history of the **current branch**: spread commits over a time range on chosen weekdays and working hours (with a
believable distribution), rewrite author/committer identities, and clean or rewrite messages — without ever losing a commit
or touching its content. Every rewritten commit keeps its tree; every run is verified before any ref moves and leaves a backup.

```
ghma init                       # write a starter .git-hide-my-ass.yml
ghma plan  [--from <rev|root>]  # dry run; add --out plan.json to save, --check to exit 6 if anything is nonconforming
ghma apply [--from <rev|root>]  # verify, back up, rewrite
ghma restore [<id>] [--force|--prune]
ghma export / import            # compact JSONL for LLM-written messages (see below)
ghma hook install|uninstall     # pre-push hook
```

By default the range is `upstream..HEAD` (unpushed commits). Without an upstream pass `--from <rev>` (exclusive) or
`--from root`. Commits already on the upstream need `--rewrite-pushed`. Commits that already conform are left alone, so
running `apply` twice is a no-op and the hook only touches new commits.

## Config (`.git-hide-my-ass.yml`)

```yaml
version: 1
from: 2026-04-01          # required with `schedule`; the schedule's lower bound
to: now                   # `now` or a date (end of that day)
timezone: Europe/Berlin   # default UTC
schedule:                 # optional; without it original times are kept
  days: [mon, tue, wed, thu, fri]
  hours: "09:30-18:00"
  distribution: bursty    # uniform | weekday-weighted | bursty
  seed: 42
identity:                 # first match wins; `set` needs name and email
  - match: { email: me@home.org }
    set:   { name: Jane Doe, email: jane@work.com }
messages:
  strip_trailers: [Signed-off-by, Co-Authored-By]   # dropped from the trailing trailer block
  add_trailers:                                      # appended unless that exact trailer is present
    - "Assisted-By: Claude <noreply@anthropic.com>"
signing: strip            # strip | resign (re-sign through your git signing config)
hook: { mode: verify }    # verify | rewrite
```

## Safety model

- New commits are built from the old tree with remapped parents; before any ref moves, `apply` checks tree equality, parent
  order, commit count, the whole-tree diff, and that all unchanged commits are still reachable.
- One atomic ref transaction creates `refs/ghma/backup/<branch>/<id>/{old,new}` and moves the branch with compare-and-swap.
  Backups are never pruned automatically. `ghma restore <id>` goes back; it refuses if the branch moved on (unless `--force`).
- Refused (exit 3): shallow clones, replace refs/grafts, detached HEAD, staged changes, rebase/merge/cherry-pick in progress.
- Exit codes: 0 ok, 1 internal, 2 usage/config, 3 refused, 4 branch moved, 5 pushed commits, 6 nonconforming, 7 bad LLM reply.

## LLM workflow

```
ghma plan --from root --all --out plan.json      # --all makes every commit editable
ghma export --plan plan.json [--batch 100 --offset 0] > rows.jsonl   # prompt template goes to stderr
# give rows.jsonl to an LLM; it answers with JSONL: {"i": 3, "t": "Title", "b": "Body"}
ghma import --plan plan.json reply.jsonl         # validated all-or-nothing; prints rows to retry on error
ghma apply  --plan plan.json
```

## Backends and performance

Commits are read and written in-process through gitoxide (`gix`) by default: 5,000 commits apply in ~0.3 s.
The `git` backend spawns the `git` binary for every written commit (~2 ms each; 5,000 commits: ~11 s) and produces
byte-identical commits. Opt out of `gix` with any of:

```sh
ghma --backend git apply        # one run
GHMA_BACKEND=git ghma apply     # environment
# .git-hide-my-ass.yml
backend: git
```

Precedence: `--backend`, `GHMA_BACKEND`, config `backend:`, then the build default. A binary built with
`--no-default-features` has no gix and defaults to `git`; asking it for `gix` is an error. Only batch object reads and
commit writes move between backends; refs, signing and hooks always use git. Measure on your machine with
`cargo test --release --test perf -- --ignored --nocapture` (`GHMA_PERF_COMMITS=50000` for more).

## Hook

`ghma hook install` adds a `pre-push` hook. In `verify` mode it blocks pushes of nonconforming commits ("run `ghma apply`").
In `rewrite` mode it rewrites them and aborts the push so you push again. Deletes and pushes of other branches are ignored; `git push origin HEAD` counts as the checked-out branch.

## Architecture

Domain-driven design with a hexagonal layout (`src/`):

```
domain/        pure model and rules (no I/O): error, settings, scheduling, text, history
application/   use cases and the ports they need: planning, rewrite (apply/restore), llm, push_guard, ports
adapters/      git_cli, gix_store, repository (composition), config_file, plan_file, hook_installer, cli
```

Dependencies point inwards only: `domain` knows nothing of ours, `application` only the domain, the leaf adapters
(`git_cli`, `gix_store`, files) the application and domain, `repository` composes the leaf adapters, `cli` may use all.
Inside the domain and application there are sub-layers too (e.g. `scheduling` builds on `settings`; `rewrite` and
`push_guard` build on `planning`). `tests/architecture.rs` enforces this with
[archunit](https://crates.io/crates/archunit), together with: no module cycles, no I/O or CLI crates in the domain,
`git_cli` and `gix_store` independent of each other, and a size cap per file.

## Development

```sh
cargo test                       # unit, integration, architecture tests (default features, gix)
cargo test --no-default-features # git-CLI-only build
cargo clippy --all-targets
```

GitHub Actions (`.github/workflows`): `ci.yml` runs fmt, clippy and the tests (default and git-only features, Linux and
macOS) on every push and pull request, then builds a release binary. `release.yml` is optional: start it from the
Actions tab ("Run workflow", enter the version from `Cargo.toml`) or push a `vX.Y.Z` tag. It re-runs CI, builds binaries
for Linux and macOS (x86_64 and arm64), and creates a GitHub release with archives and checksums.
