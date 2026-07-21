# Archive.org Redump Link Updater

A small desktop app that rewrites **`redump.org` → `redump.info`** in the
descriptions of your own archive.org uploads.

Sign in, scan your items, review what will change, and apply. That is the whole
app — the replacement is baked in, so there is nothing to configure and no way
to accidentally run a different find-and-replace.

## What it changes

The rewrite applies everywhere the domain appears, including inside links:

| Before | After |
| --- | --- |
| `Redump.org` | `Redump.info` |
| `www.redump.org/disc/192839128392` | `www.redump.info/disc/192839128392` |
| `<a href="http://redump.org/disc/123">` | `<a href="http://redump.info/disc/123">` |

Casing is preserved (`REDUMP.ORG` → `REDUMP.INFO`), and lookalikes are left
alone — `notredump.org` and `redump.organization` are not touched.

Only the **description** field is modified. Titles, files, and every other
field are left exactly as they are.

## Running it

Install [Rust](https://rustup.rs), then:

```sh
cargo run --release
```

The built binary lands at `target/release/archive-org-redump-link-updater` and is self-contained —
copy it to another machine of the same OS and it just runs. The same source
builds on macOS, Windows, and Linux.

On Linux you may need the usual GUI development packages first:

```sh
sudo apt install libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev \
                 libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

## Signing in

Two options. Nothing is persisted between runs; you sign in each time.

**Email + password** — your normal archive.org login. Simplest if you know it.

**S3 keys (Advanced Only)** — a pair of API credentials from
[archive.org/account/s3.php](https://archive.org/account/s3.php). The name is
historical: archive.org's API mimics Amazon's S3 interface, so the credentials
inherited the term. They have nothing to do with Amazon.

Either way the app ends up using the S3 keys, because a password cannot write
metadata on its own — signing in with a password simply exchanges it for these
same keys behind the scenes. Pasting the keys directly just skips that step.

Reasons to prefer the keys:

- You never type your account password into someone else's program.
- A leaked key can be regenerated on that page and instantly stops working,
  without touching your password or your other sessions.
- Accounts that sign in through Google may have no archive.org password at all;
  the keys are the only way in for those.

## How it works

1. **Scan** lists every item you uploaded and flags the descriptions that
   mention `redump.org`. Nothing is written during a scan.
2. **Review** each flagged item, with a before/after preview. Uncheck anything
   you want to leave alone.
3. **Apply** writes the changes, one item at a time with a short pause between
   them so archive.org is not hammered.

To keep the list focused on what is left to do, either tick **Clear when fixed**
so items drop off as they finish, or press **Clear completed** to sweep the
finished ones away whenever you like. Both leave failures on the list so you can
retry them, and both record every cleared item in the Log panel, so nothing
disappears without a trace.

Before each write the app re-reads that item's current description from
archive.org and re-applies the fix to the fresh copy, so a stale scan can never
overwrite an edit made in the meantime. Items already using `redump.info` are
skipped rather than rewritten.

Failures are listed in the Log panel at the bottom and never stop the run.

## Tests

```sh
cargo test
```

The replacement logic is covered by unit tests, including URL forms, casing,
repeated occurrences, and the lookalike domains that must not change.
