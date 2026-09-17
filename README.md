# GridLock launcher and site

Demo distribution for GridLock: testers install a small launcher, sign in, and the launcher keeps
the game up to date by downloading only the files that changed.

- `site/` Next.js on Vercel. Clerk handles accounts (open registration). `/api/manifest` hands a signed-in
  launcher the newest build manifest with presigned Cloudflare R2 URLs; `/launcher/login` mints the
  launcher's 30-day token and passes it back over loopback.
- `launcher/` Tauri 2 app (Rust + plain HTML/JS). Loopback sign-in, content-addressed downloads with
  resume and SHA-256 checks, verify, play; updates itself from this repo's GitHub Releases.
- Game builds are published by the game repo's `Package` workflow (`.github/scripts/publish_build.py`
  there) into R2 as `blobs/<sha256>` + `builds/<platform>/<version>.json`. A blob is the file cut into
  128 KiB blocks behind a table of per-block hashes (`launcher/src-tauri/src/delta.rs` documents the
  layout); an update keeps every block the old copy of a file still holds, wherever it moved to, and
  fetches the rest with Range requests, so a new build costs tens of MB instead of the 2.4 GB .ucas + exe.
  Manifests without `format` point at raw `objects/<sha256>` (builds before 2026-09-16, the server's
  deploy script); the launcher handles both.

## Site

```bash
cd site && cp .env.example .env.local   # fill in Clerk + R2 + LAUNCHER_TOKEN_SECRET
npm run dev
```

## Launcher

```bash
cd launcher && npm ci && npm run dev    # needs Rust and, on Linux, the Tauri webkit2gtk deps
GRIDLOCK_SITE=https://preview.vercel.app npm run dev   # point it at another deployment
```

Release: bump `version` in `launcher/src-tauri/tauri.conf.json`, `launcher/src-tauri/Cargo.toml` and
`launcher/package.json`, then
`git tag launcher-v<version> && git push --tags`; the Launcher workflow builds both platforms into a draft
release, and its last job writes `latest.json` for the updater and publishes the draft, so the site and the
launchers only ever see a complete release. Signing key: `TAURI_SIGNING_PRIVATE_KEY` secret.
