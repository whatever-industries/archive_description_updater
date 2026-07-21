# Archive.org Redump Link Updater

Rewrites **`redump.org` → `redump.info`** in the descriptions of your own
archive.org uploads, links included — `redump.org/disc/123` becomes
`redump.info/disc/123`.

## Download

Grab your platform's file from the
[latest release](https://github.com/whatever-industries/archive_description_updater/releases/latest)
— macOS, Windows, or Linux. Nothing to install.

The app isn't signed with a paid developer certificate, so you get one warning
the first time:

- **macOS** — right-click the app and choose Open (double-clicking is refused)
- **Windows** — More info → Run anyway
- **Linux** — `chmod +x` the AppImage first

## Using it

Sign in, press **Scan**, check the before/after previews, press **Apply**.

Only the description field is ever touched. Casing is preserved, lookalikes
like `notredump.org` are left alone, and items already on `redump.info` are
skipped. Failures appear in the Log and never stop the run.

<details>
<summary>Signing in with S3 keys instead of a password</summary>

Archive.org issues every account a pair of API keys at
[archive.org/account/s3.php](https://archive.org/account/s3.php) — called "S3
keys" only because their API mimics Amazon's, nothing to do with Amazon.

The app ends up using them either way: a password can't write metadata on its
own, so signing in with one just exchanges it for these keys behind the scenes.

Use the keys directly if you'd rather not type your password into someone
else's program, if you want a credential you can revoke on its own, or if your
account signs in through Google and has no archive.org password at all.

</details>
