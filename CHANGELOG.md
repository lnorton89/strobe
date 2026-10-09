# Changelog

All notable changes to Strobe, newest first.

**This file is generated.** It is rebuilt from the git history by
`cargo xtask changelog`, so released sections are rewritten in
place and edits to them do not survive. See `CONTRIBUTING.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- adaptive render scale controller ([`c6c59a2`](https://github.com/lnorton89/strobe/commit/c6c59a2d1b5b7195945a2ee80fe5d0a572243708))
- headless benchmark, 10-scene base library, and agent planning docs ([`f84b564`](https://github.com/lnorton89/strobe/commit/f84b5648124db83bf718b28cd7f46d36e3c9001a))
- workspace, SSF scene model, GLSL library, and headless GL renderer ([`39645c1`](https://github.com/lnorton89/strobe/commit/39645c19d96e0f2f3dc6b1b89b52c7da5ce259a9))

### Changed

- move synthetic audio into strobe-scene so the app can share it ([`1936f14`](https://github.com/lnorton89/strobe/commit/1936f1460101cc9b7a887a102afb8a833a59d874))

### Documentation

- document xtask, the release process and the CI/release decisions (D50-D59) ([`dfb4f38`](https://github.com/lnorton89/strobe/commit/dfb4f38cac45b74fa492483333905ed89ea85af7))
- add CODEOWNERS, dependabot config, issue forms, PR template and SECURITY policy ([`af273dc`](https://github.com/lnorton89/strobe/commit/af273dc3008d00bfe27a812030627a1de63dc9f3))
- deepen UI research with interaction and runtime findings ([`16fe415`](https://github.com/lnorton89/strobe/commit/16fe41572d4d94b9be4edf56a0f56fbda32b2820))
- add detailed UI implementation handoff ([`05c5e66`](https://github.com/lnorton89/strobe/commit/05c5e66fd9fc3e22a1aed457845feb1940fbad4f))
- add second-round UI research conclusions ([`d107add`](https://github.com/lnorton89/strobe/commit/d107add243148897cc149906aec98d4582649aca))
- add interactive HTML UI mockup directions ([`b365d50`](https://github.com/lnorton89/strobe/commit/b365d50ed694b746d194f3341e95b91fa67fcfcc))
- add UI mockup and design decision workflow ([`ca6dd46`](https://github.com/lnorton89/strobe/commit/ca6dd46faacdf5c05ef215e4f503e39dc13348f7))
- clean UI research citations for GitHub ([`ef48a02`](https://github.com/lnorton89/strobe/commit/ef48a02f0aa097a8452f9a74dcad8678d08a8dde))
- link agent research to UI architecture ([`07aec68`](https://github.com/lnorton89/strobe/commit/07aec681254291d681875c5e74fd595d0fd79c2a))
- add UI design guide to agent reading list ([`9ec6031`](https://github.com/lnorton89/strobe/commit/9ec60311bd11940beea1aa3fc542707275d2b094))
- add modern UI architecture and interaction plan ([`0aa899e`](https://github.com/lnorton89/strobe/commit/0aa899e02e3cc89d0f463b191960af8fd0639699))
- point agents to integration research guide ([`d4771f8`](https://github.com/lnorton89/strobe/commit/d4771f8895bf4e45be0af1fdd719829c5d1055f2))
- add deep agent research and integration guide ([`f1a0494`](https://github.com/lnorton89/strobe/commit/f1a0494ffbda8b36c62e9a7a45e7418b8d87c75a))
- track diagnostics and repo/CI agents in the roadmap ([`66a9676`](https://github.com/lnorton89/strobe/commit/66a96767263ab780b1af55467881de106c5e1ed1))
- baseline benchmarks and in-flight work in the roadmap ([`2143d7b`](https://github.com/lnorton89/strobe/commit/2143d7bf254d2a50088caa35e840b8043c735d10))
- add README, design plan, and repository base files ([`d5dc4f4`](https://github.com/lnorton89/strobe/commit/d5dc4f49eb0997d93945177ab1b9399aaea755c8))

### Internal

- add release workflow, macOS job, changelog check, bench frame artifacts and pin-checking tests ([`bbaf41f`](https://github.com/lnorton89/strobe/commit/bbaf41fb1881ca03c543a7f86de8294bc85d4bc6))
- add xtask crate with ci, changelog and bench subcommands ([`8980bb0`](https://github.com/lnorton89/strobe/commit/8980bb02f6f5d1e582f064b830b1748327e7b6ef))
- build every workspace binary and run on claude/** pushes ([`bda6a9a`](https://github.com/lnorton89/strobe/commit/bda6a9ade69ad27a2d45236cb80e5d34c0ab8816))
- ignore agent worktrees ([`8ae14d9`](https://github.com/lnorton89/strobe/commit/8ae14d97945f78622b6ea7274914eda1dd4a1881))
- pinned, hash-checked ANGLE fetch script for the D3D11 fallback ([`cf14096`](https://github.com/lnorton89/strobe/commit/cf140967b61f5ff18c13a9fcfac3a8612ef4e2fa))

[Unreleased]: https://github.com/lnorton89/strobe/commits/HEAD
