# Contributing to UltraVox

Bug reports, focused fixes and documentation improvements are welcome.

## Before you start

- Search [existing issues](https://github.com/michael-berardi/ultravox/issues) first.
- Describe the user-visible behavior, your operating system version and the
  steps to reproduce it.
- For anything larger than a small fix, open an issue first so the approach
  can be agreed before you spend time on it.
- Report security problems privately as described in [SECURITY.md](SECURITY.md).

## Making a change

1. Fork the repository and clone it with submodules
   (`git clone --recurse-submodules`).
2. Create a branch from `main` and keep the change focused on one problem.
3. Add tests for new observable behavior, and update the README or `docs/`
   when behavior changes.
4. Run the checks below.

```sh
pnpm install --frozen-lockfile
pnpm desktop:check
cargo test --workspace
pnpm test:all
```

The open-source build excludes the Pro module and links stubs in its place;
see [Build from source](README.md#build-from-source).

## Pull requests

Describe what changed and why, how you tested it, and which platforms you
tried. Never include recordings, transcripts, credentials, private URLs or
personal system details in code, fixtures, issues or screenshots.

By contributing you agree that your contribution is licensed under the
project's [MIT License](LICENSE).
