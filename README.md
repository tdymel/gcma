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
  strip_trailers: [Signed-off-by]
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

By default every git operation spawns the `git` binary, which costs about 2 ms per written commit (5,000 commits: ~11 s).
Build with `--features gix` to write commits in-process through gitoxide instead (5,000 commits: ~0.3 s, identical commits):

```sh
cargo build --release --features gix
ghma --backend gix apply            # or GHMA_BACKEND=gix, or `backend: gix` in .git-hide-my-ass.yml
```

Precedence: `--backend`, `GHMA_BACKEND`, config `backend:`, then `git`. Only batch object reads and commit writes move;
refs, signing and hooks always use git. Measure on your machine with
`cargo test --release --features gix --test perf -- --ignored --nocapture` (`GHMA_PERF_COMMITS=50000` for more).

## Hook

`ghma hook install` adds a `pre-push` hook. In `verify` mode it blocks pushes of nonconforming commits ("run `ghma apply`").
In `rewrite` mode it rewrites them and aborts the push so you push again. Deletes and pushes of other branches are ignored; `git push origin HEAD` counts as the checked-out branch.

See `DESIGN.md` for the full design and the implementation notes.
