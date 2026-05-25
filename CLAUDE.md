# Mantybird — Project Conventions

Cross-platform IMAP/SMTP desktop client (Tauri 2 + React 18 + Vite + TypeScript).

## Git Workflow: git-flow

Long-lived branches:
- `main` — production releases only. **No direct commits.**
- `develop` — integration branch. **No direct commits.**

Topic branches:
- `feature/<name>` — branched from `develop`, merged back to `develop` via `--no-ff`.
- `release/<x.y.z>` — branched from `develop`, merged to both `main` and `develop`.
- `hotfix/<name>` — branched from `main`, merged to both `main` and `develop`.

Rules:
- Always branch features from an up-to-date `develop`.
- Use `--no-ff` when merging feature / release / hotfix branches back so the
  branch boundary stays visible in `git log --graph`.
- Tag releases on `main` as `vX.Y.Z` (SemVer).
- **Confirm with the user before merging or pushing to `develop` or `main`.**
- Never force-push `main` or `develop`. Never bypass hooks (`--no-verify`).

Typical feature flow:
```bash
git checkout develop && git pull
git checkout -b feature/<name>
# … work, commit …
git push -u origin feature/<name>
# (after review)
git checkout develop && git merge --no-ff feature/<name>
git push origin develop
git branch -d feature/<name>
git push origin :feature/<name>     # delete remote
```

## Build

See `README.md` and `Makefile`:
- `make install` → `npm install`
- `make dev` → `npm run tauri dev`
- `make package` → production `.app` + `.dmg`

## Verification before committing

- Frontend type check: `npx tsc --noEmit`
- Rust check: `cd src-tauri && cargo check`

## Notes

- Frontend lives in `src/` (React).
- Backend lives in `src-tauri/` (Rust commands invoked via `@tauri-apps/api`).
- All passwords are stored in the OS keychain (`config::save_password`); never
  log or persist passwords elsewhere.
