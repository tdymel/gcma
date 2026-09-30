# gcma — git cover my ass

Reshape the history of the **current branch**: spread commits over a time range on chosen weekdays and working hours (with a
believable distribution), rewrite author/committer identities, and clean or rewrite messages — without ever losing a commit
or touching its content. Every rewritten commit keeps its tree; every run is verified before any ref moves and leaves a backup.

## Install

Needs a Unix system (Linux, macOS) and `git` on the `PATH` (refs, signing and hooks always go through it; 2.30 or newer
recommended). Rust 1.90 or newer to build:

```sh
cargo install --path .           # or: cargo build --release  ->  target/release/gcma
```

## Usage

```
gcma init                       # write a starter gcma.yml (inert until you enable something)
gcma plan  [--from <rev|root>]  # dry run; add --out plan.json to save, --check to exit 6 if anything is nonconforming
gcma apply [--from <rev|root>]  # verify, back up, rewrite; --retag also moves tags and copies notes
gcma restore [<id>] [--force|--prune]   # list, undo, or forget a backup
-C <dir>, --config <file>, --backend git|gix    # global options
gcma export / import            # compact JSONL for LLM-written messages (see below)
gcma hook install [--force] [--post-commit] | uninstall   # pre-push hook, optionally post-commit too
```

By default the range is `upstream..HEAD` (unpushed commits). Without an upstream pass `--from <rev>` (exclusive) or
`--from root`. Commits already on the upstream need `--rewrite-pushed`. Commits that already conform are left alone, so
running `apply` twice is a no-op and the hook only touches new commits. Every other commit of the range is rewritten,
and with a `schedule` it gets a fresh time even if its old one was already inside the hours (a commit that only needs a new
message or identity is rescheduled too), together with every descendant of a nonconforming commit.

## Config (`gcma.yml`)

```yaml
version: 1
from: 2026-04-01          # required with `schedule`; the schedule's lower bound
to: now                   # `now` or a date (end of that day)
timezone: Europe/Berlin   # default UTC
schedule:                 # optional; without it original times are kept
  days: [mon, tue, wed, thu, fri]   # any of mon tue wed thu fri sat sun (full names work too, case-insensitive)
  hours: "09:30-18:00"    # one range, or a list: ["18:00-24:00", "06:00-07:00"]
                          # a range ending before it starts runs past midnight: "18:00-06:00"
  distribution: bursty    # uniform | weekday-weighted | bursty
  seed: 42
identity:                 # first match wins; `set` needs name and email
  - match: { email: me@home.org }
    set:   { name: Jane Doe, email: jane@work.com }
messages:
  strip_trailers: [Signed-off-by, Co-Authored-By]   # dropped from the trailing trailer block
  add_trailers:                                      # appended unless that exact trailer is present
    - "Assisted-By: Claude <noreply@anthropic.com>"
  rewrite_trailers:                                  # sed-like, on the lines of the trailer block, run first
    - match: '^Co-Authored-By: Claude (Opus|Sonnet)\b.*<noreply@anthropic\.com>$'
      replace: 'Assisted-By: Claude $1 <noreply@anthropic.com>'
  title_only: false                                  # true drops the body; subject and trailer block stay
paths:                                               # remove paths from history (gitignore syntax)
  exclude: ["secrets/", "*.env"]
  gitignore: true                                    # default: add the patterns to .gitignore
  only_excluded_commits: drop                        # drop | keep (keep leaves an empty commit)
signing: strip            # strip | resign (re-sign through your git signing config)
hook: { mode: verify }    # verify | rewrite
```

Working hours may be several ranges, and a range may run past midnight. An overnight range belongs to the weekday it
starts on: with `days: [fri]` and `hours: "18:00-06:00"` commits land on Friday evening and early Saturday, never on
Saturday evening. A commit that conforms in every respect (time inside the window with the configured timezone's offset,
no signature to strip, identity and message already as the rules want them) is left alone; every other commit is moved
to a scheduled time inside it, so a commit made at noon with evening-only hours
ends up on an evening, never earlier than its parents.

Message rules run in this order: `rewrite_trailers`, `strip_trailers` (both repeated until nothing changes), `title_only`,
`add_trailers`. A rewrite replaces every match of its regex (the Rust `regex` syntax, `$1` for groups) in each line of the
final trailer block; a line that becomes empty is dropped and lines that become identical collapse. Patterns are Unicode
patterns: a trailer line with bytes that are not UTF-8 only matches a `(?-u)` pattern. Rules must be idempotent (a rewrite
must not produce something another rule or itself rewrites again), otherwise `plan --check` never settles. The subject is
the first paragraph; `title_only` keeps it and the final trailer block, so a body paragraph that looks like `Key: value`
lines at the very end counts as that block.

## Tags and notes

A rewrite leaves tags and notes on the old commits, and `plan`/`apply` warn about them. `gcma apply --retag` (also with
`--plan`) makes them follow: a lightweight tag on a rewritten commit moves to its new commit, and an unsigned annotated tag
is recreated with the same name, tagger, date and message, pointing at the new commit. The tag moves are part of the
transaction that moves the branch (each compare-and-swap, so everything moves or nothing does) and are checked before it.
Tags that stay, with a warning: signed annotated tags (a copy would lose the signature), tags of tags (a tag object that
points at another tag object; the inner tag moves, the outer one keeps pointing at its old object), tags whose name is not valid UTF-8, and tags on
commits that `paths.exclude` drops. Tags on commits that keep their id are not touched. `gcma plan --retag` lists what would move.
Notes (`refs/notes/*`) on rewritten commits are copied to the new commits after the branch moved, best effort: a failure
is a warning and the old notes stay. `gcma restore <id>` puts the tags back, and refuses (exit 4) if one of them changed
since; with `--force` such a tag is left as it is.

## Removing paths from history

`paths.exclude` takes gitignore patterns. Every rewritten commit loses those paths; a commit that touched nothing else is
dropped (or kept empty with `only_excluded_commits: keep`) and its children are re-parented. The files are **not** removed
from your project: the patterns are appended to the root `.gitignore` starting with the first kept commit that had such
paths, and in everything built on top of it, so the files stay in the working copy but out of git. If the tip itself only
touched excluded paths and no earlier kept commit carries the patterns, the tip stays as a commit that only adds them.
After the branch moves, the index is reset to the new tip, and the working copy's `.gitignore` is updated unless you have
local edits to it. `plan` lists dropped commits. Only unpushed commits are rewritten (see `--rewrite-pushed`). Apply
re-derives every tree from the rules and re-checks that nothing but `.gitignore` differs from the old tip minus the excluded
paths, so a hand-edited plan cannot change content. This is the one feature that changes trees; the backup keeps the originals.

Dropping a commit on a side branch can leave a merge whose two parents are now the same commit, or one parent an ancestor of
the other. That is valid in git and stable under re-running `apply`; gcma does not prune such parents because that would change
which commits the merge is "of".

### Purging removed paths for real

Backups are the safety net, so the original commits (and anything in the excluded paths) stay reachable from
`refs/gcma/backup/…`, the reflog, tags, other branches and any remote that already has them; `git push --mirror` or a
`refs/*` refspec would upload the backup refs. If excluded files held secrets, rotate them. To purge the old history
locally: `gcma restore <id> --prune`, then `git reflog expire --expire=now --all && git gc --prune=now`.

## Safety model

- New commits are built from the old tree (minus excluded paths, if configured) with remapped parents; before any ref moves,
  `apply` checks the trees, parent order, commit count (old minus dropped), the tip-tree diff, and that all unchanged commits
  are still reachable.
- One atomic ref transaction creates `refs/gcma/backup/<branch>/<id>/{old,new}` and moves the branch with compare-and-swap.
  Backups are never pruned automatically. `gcma restore <id>` goes back; it refuses if the branch moved on (unless `--force`).
  With `--retag` the moved tags are recorded in a blob `…/<id>/tags` (plus `…/<id>/tag-<n>`, which keeps the old annotated
  tag objects reachable); no tag name ever becomes part of a backup ref name.
- `apply` only moves the checked-out branch named in the plan, a plan from a file must carry the same path rules as the
  config, and every object id and the branch ref in it are validated; `.gitignore` is never written through a symlink.
  The config file (`gcma.yml`) is trusted like a script: with the hook in `rewrite` mode, a config pulled from
  an untrusted branch can rewrite your unpushed history on push (backups exist). `gcma init` keeps it out of commits via
  `.git/info/exclude`.
- `restore` also puts the index (and gcma's `.gitignore` change) back when path rules had changed the content.
- Names, emails and messages need not be UTF-8: they are carried byte for byte (identity rules match them as text with invalid bytes
  replaced, so an email rule still applies; a rule that matches sets both fields, otherwise the bytes are left as they are). One limitation: git recodes a message that is not valid UTF-8 when it signs, so with `signing: resign`
  `plan` warns and `apply` refuses (exit 3) to rewrite such a commit unless an imported reply gives it a new message;
  use `signing: strip` otherwise.
- Refused (exit 3): shallow clones, replace refs/grafts, detached HEAD, staged changes, rebase/merge/cherry-pick in progress.
- Exit codes: 0 ok, 1 internal, 2 usage/config, 3 refused, 4 branch moved, 5 pushed commits (also for a dry run), 6 nonconforming,
  7 bad LLM reply. `gcma restore --force` parks what it discards at `refs/gcma/discarded/…`.

## LLM workflow

```
gcma plan --from root --all --out plan.json      # --all makes every commit editable
gcma export --plan plan.json [--batch 100 --offset 0] > rows.jsonl   # prompt template goes to stderr
# give rows.jsonl to an LLM; it answers with JSONL: {"i": 3, "t": "Title", "b": "Body"}
gcma import --plan plan.json reply.jsonl         # validated all-or-nothing; prints rows to retry on error
gcma apply  --plan plan.json
```

## Backends and performance

Commits are read and written in-process through gitoxide (`gix`) by default: 5,000 commits apply in ~0.3 s.
The `git` backend spawns the `git` binary for every written commit (~2 ms each; 5,000 commits: ~11 s) and produces
byte-identical commits. Opt out of `gix` with any of:

```sh
gcma --backend git apply        # one run
GCMA_BACKEND=git gcma apply     # environment
# gcma.yml
backend: git
```

Precedence: `--backend`, `GCMA_BACKEND`, config `backend:`, then the build default (`restore`, `hook install`/`uninstall`
and `import` never read the config, so they skip the config step). A binary built with
`--no-default-features` has no gix and defaults to `git`; asking it for `gix` is an error. Only batch object reads and
commit writes move between backends; refs, signing and hooks always use git. Measure on your machine with
`cargo test --release --test perf -- --ignored --nocapture` (`GCMA_PERF_COMMITS=50000` for more).

## Hook

`gcma hook install` adds a `pre-push` hook. In `verify` mode it blocks pushes of nonconforming commits ("run `gcma apply`").
In `rewrite` mode it rewrites them and aborts the push so you push again. Deletes and pushes of other branches are ignored; `git push origin HEAD` and a revision of the branch
(`HEAD~1:main`) count as the checked-out branch. Skip it once with `git push --no-verify`. Remember that config errors block pushes too.

### Post-commit hook (opt-in)

`gcma hook install --post-commit` installs the `pre-push` hook and a `post-commit` one (`hook uninstall` removes both; a
hook gcma did not write needs `--force`). After every commit it respreads the times of **all unpushed commits** over the
schedule, so a day's work always looks spread over the hours instead of landing in one burst; the pushed history never
changes, so no force push is needed. It is deterministic per seed and base commit, keeps the trees, and leaves the working copy and
index alone (path rules aside, as with `apply`). It is the `apply --all` of the unpushed range, with one backup per commit:
clean them up with `gcma restore <id> --prune`.

- It acts only with `hook: { mode: rewrite }` and a `schedule`; in `verify` mode it does nothing (a commit cannot be
  blocked), and `pre-push` stays the safety net. The unpushed range is `upstream..HEAD`; without an upstream it is every
  commit on no remote-tracking ref, and with neither upstream nor remote it does nothing (use `gcma apply --from root`).
- It never fails the commit and is silent unless something is wrong (`gcma: post-commit skipped: <reason>` on stderr). It
  skips quietly on a detached HEAD, during a rebase, merge, cherry-pick or revert, with staged changes, and when the window has no
  time left.
- Tags on unpushed commits keep pointing at the old commits, as with `apply`.

## Architecture

Domain-driven design with a hexagonal layout (`src/`):

```
domain/        pure model and rules (no I/O): error, settings, scheduling, text, paths, history
application/   use cases and the ports they need: planning, rewrite (apply/restore), llm, push_guard, commit_hook,
               preconditions, pathrules, ports
adapters/      git_cli, gix_store, repository (composition), config_file, plan_file, hook_installer,
               llm_jsonl, fsutil, convert, cli (the commands) and cli_support (grammar, session, output)
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

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project, as defined
in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
