<!--
Thanks for contributing! Please read CONTRIBUTING.md if you have not yet.
The PR title must be a Conventional Commit, e.g. "fix(engine): handle 416 on empty files".
It becomes the squash commit on main.
-->

## What and why

<!-- What does this change, and why? Link the issue. -->

Fixes #

## How it was tested

<!-- Commands run, scenarios covered, hardware/network for performance changes. -->

## Screenshots / recording

<!-- Required for UI changes: dark and light theme. Delete this section otherwise. -->

## Checklist

- [ ] PR title follows [Conventional Commits](https://github.com/boltget/boltget/blob/main/CONTRIBUTING.md#commit-messages-conventional-commits) (`type(scope): subject`, ≤ 72 chars)
- [ ] Every commit is signed off (`git commit -s`, [DCO](https://github.com/boltget/boltget/blob/main/CONTRIBUTING.md#developer-certificate-of-origin-dco))
- [ ] One logical change; linked issue for non-trivial work (RFC for architectural changes)
- [ ] Tests added or updated (regression test for bug fixes)
- [ ] Docs updated (`docs/` design docs, CLI help, user docs) where behaviour changed
- [ ] `CHANGELOG.md` updated under `Unreleased` for user-visible changes
- [ ] User-facing strings come from the string catalog (no hard-coded UI text)
- [ ] UI changes: screenshots attached; checked in dark and light themes, keyboard-only and with reduced motion
- [ ] Performance changes: before/after numbers included
- [ ] New dependencies are GPL-3.0-compatible and justified in the description
- [ ] If AI tools were used: I understand, have reviewed and have tested all of this code ([policy](https://github.com/boltget/boltget/blob/main/CONTRIBUTING.md#ai-assisted-contributions))
