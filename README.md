# GridLock launcher and site

Demo distribution for GridLock: testers install a small launcher, sign in, and the launcher keeps
the game up to date by downloading only the files that changed.

- `site/` Next.js on Vercel. Clerk handles accounts (open registration). `/api/manifest` hands a signed-in
  launcher the newest build manifest with presigned Cloudflare R2 URLs; `/launcher/login` mints the
  launcher's 30-day token and passes it back over loopback.
- `launcher/` Tauri 2 app (Rust + plain HTML/JS). Loopback sign-in, content-addressed downloads with
  resume and SHA-256 checks, verify, play; updates itself from this repo's GitHub Releases.
- Game builds are published by the game repo's `Package` workflow (`.github/scripts/publish_build.py`
  there) into R2 as `objects/<sha256>` + `builds/<platform>/<version>.json`.

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

Release: bump `version` in `launcher/src-tauri/tauri.conf.json` and `launcher/package.json`, then
`git tag launcher-v<version> && git push --tags`; the Launcher workflow builds both platforms and
publishes the release with `latest.json` for the updater. Signing key: `TAURI_SIGNING_PRIVATE_KEY` secret.
