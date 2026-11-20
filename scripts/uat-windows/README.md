# Windows UAT for Kōrero's local AI (1.49)

Drives the real build on a Windows laptop without touching the installed Kōrero's data:

1. Build: `bun run tauri build --no-bundle` in a worktree.
2. Copy `src-tauri\target\release\korero.exe` into an empty folder `<uat>\app\` and write the
   portable marker: `[IO.File]::WriteAllText("<uat>\app\portable", "Handy Portable Mode")`.
   All settings, history, logs and the local AI then live in `<uat>\app\Data`.
3. Quit the installed Kōrero (one instance at a time), then `launch.ps1 -Uat <uat>` (add
   `-Proxy` to test with a dead proxy set). It points `LOCALAPPDATA` at an empty folder and
   takes Ollama off `PATH`, so the build sees a computer with no Ollama; and it opens the
   webview to a driver on 127.0.0.1:9333 for this run only.
4. Drive and read the page with `node cdp.mjs <action>` from a checkout with `node_modules`
   (`shot`, `text`, `click`, `clickwatch`, `goto`, `eval`).
5. `block.ps1 -Uat <uat>` denies this user "execute" in the test copy's runtime folder, as a
   stand-in for AppLocker or WDAC; `-Undo` removes it.

Not isolated: API keys are in the Windows credential store, shared with the installed app. Do
not clear a key in the test build.

Run on legion-26 on 18 Nov 2026: see `docs/TESTING.md`, "Local AI (1.49)".
