# Governance

BoltGet is a young project with a strong point of view. Its governance is designed to keep it coherent while it grows, and to become more distributed as the contributor base does.

## Model

BoltGet uses a **BDFL-style model** ("benevolent dictator for life", in practice "project lead with final say") with a group of maintainers.

## Roles

### Project lead

- The owner of the `boltget` GitHub organization.
- Sets the product direction ([docs/VISION.md](docs/VISION.md)), owns the roadmap, and has the final say on any decision when consensus cannot be reached.
- Owns the project's accounts and secrets: GitHub org, domain (boltget.app), code-signing identity, updater signing keys, store developer accounts. Critical secrets have a documented backup held by the project lead (see [docs/RELEASE.md](docs/RELEASE.md#auto-update)).
- Appoints and removes maintainers.

### Maintainers

- Members of the `@boltget/maintainers` team on GitHub.
- Review and merge pull requests, triage issues, cut releases, moderate community spaces under the [Code of Conduct](CODE_OF_CONDUCT.md).
- May own specific areas (engine, desktop, extension, web, release/infra). Area ownership is recorded in [.github/CODEOWNERS](.github/CODEOWNERS); while the team is small, everything is owned by `@boltget/maintainers`.
- Are expected to follow [CONTRIBUTING.md](CONTRIBUTING.md) like everyone else: their own PRs need review from another maintainer once there are at least two.

### Triagers

- Contributors with GitHub "Triage" permission: they label, deduplicate and close issues, but do not merge.
- A good first step toward maintainership.

### Contributors

- Anyone who opens an issue, comments in a discussion, reviews, or sends a pull request. Contributors keep the copyright in their contributions, licensed to everyone under GPL-3.0-or-later (see [License of contributions](CONTRIBUTING.md#license-of-contributions)).

## Becoming a maintainer

There is no application form. Maintainers are invited by the project lead, typically after a contributor has, over a sustained period (usually several months):

- made multiple non-trivial, high-quality contributions in an area,
- reviewed other people's pull requests constructively,
- shown good judgment about the project's principles and non-goals,
- been consistently respectful in community spaces.

Triager permissions usually come first. Existing maintainers can nominate someone; the project lead decides after consulting the maintainers.

Maintainers who become inactive for six months (no reviews, merges or triage) are moved to emeritus status, with thanks. They can return by asking. Maintainer status can also be removed by the project lead for violations of the Code of Conduct or repeated disregard for the contribution rules.

## Decision making

1. **Lazy consensus by default.** Most decisions happen in pull requests and issues. If a proposal has maintainer approval and no unresolved objections after reasonable time, it goes ahead.
2. **RFCs for significant changes.** Architecture, protocols, data model, security model, tech stack, licensing and governance changes go through the [RFC process](CONTRIBUTING.md#rfcs). Comment period: at least one week for non-trivial RFCs.
3. **Maintainer discussion** when there are objections: the maintainers try to reach consensus in the PR, issue or RFC thread, in public.
4. **Project lead decides** when consensus cannot be reached, and writes down the reasoning in the thread.

Decisions already recorded in [docs/TECH-STACK.md](docs/TECH-STACK.md) and [docs/VISION.md](docs/VISION.md) stand until changed through an RFC.

Some things are **not up for a vote**: the [principles in VISION.md](docs/VISION.md#principles), in particular no ads, no bundled offers, no telemetry without opt-in, and the GPL license. Changing those would be a different project.

## Money

BoltGet does not currently accept donations. If it does (for example via GitHub Sponsors), funds pay for project costs first (code signing, domain, hosting, test hardware), and how they are spent is published. Money never buys features, priority or roadmap influence.

## Changes to this document

Changes to governance go through an RFC and require approval by the project lead.
