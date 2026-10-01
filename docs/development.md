# Developing ChronoDesk

Building, testing, releasing and regenerating the pictures. For how the app is designed, see
[architecture.md](architecture.md).

## Building

Requires Rust 1.95 or newer.

```sh
cargo run --release              # build and run the overlay
cargo test                       # unit tests
pwsh -File scripts/package.ps1   # dist\: setup and portable exe, SHA256SUMS.txt
```

## End-to-end tests

The scripts in `scripts/` test the running app end to end. They need a build with
`--features instrument`, which lets them drive the app over a local connection instead of moving
the real mouse; a normal build contains none of this. Each script runs against its own config file
and leaves an overlay you already have running alone. Run them with PowerShell 7 (`pwsh`).

| Script | Checks |
| --- | --- |
| `instrument-test.ps1` | frame rate in each mode, hover controls, layout caching |
| `placement-test.ps1` | a saved position outside every screen is moved back into view |
| `lifecycle-test.ps1` | one instance per config file, *Start with Windows* |
| `alarm-test.ps1` | the alarm rings on time behind a sleeping mode, is silenced, and is dropped when missed |
| `install-test.ps1` | install, upgrade and uninstall into a scratch folder and registry key |
| `welcome-test.ps1` | the welcome card on first launch |
| `resume-test.ps1` | a running stopwatch or timer survives the app being killed |
| `chime-test.ps1` | the timer chime (counted, not played) |
| `pomodoro-test.ps1` | the Pomodoro cycle moves on, waits, survives a kill, and resets |
| `startup-time.ps1` | how long a launch takes to put its first frame up |

## Releasing

Push a tag `vX.Y.Z` matching the version in `Cargo.toml`; the release workflow builds the exes and
publishes them with `SHA256SUMS.txt`. Then sign the release on the machine that holds the signing
key, which the app needs before it installs the update by itself:

```sh
pwsh -File scripts/sign-release.ps1 -Tag vX.Y.Z   # checks the published files, uploads release.sig
```

The key never goes to GitHub or CI. `-Init` creates it and prints the public half that
`src/update.rs` carries.

## Screenshots and the website

The images in `docs/screenshots` and on the website are rendered by the app itself, off screen,
with the clock pinned to a fixed time, so they can be regenerated after a visual change:

```sh
cargo test shots -- --ignored        # renders to target/shots
python scripts/compose-shots.py      # composes docs/screenshots (needs Pillow)
python scripts/site-assets.py        # turns the renders into site/img/
python scripts/site-check.py         # checks the website's links, images and budgets
```

The website in `site/` is published by `.github/workflows/pages.yml` on every push to `master`.
