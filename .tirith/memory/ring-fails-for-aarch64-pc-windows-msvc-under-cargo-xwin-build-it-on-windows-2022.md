---
id: 38d492df-e933-4527-8920-b1febb173eef
permalink: ring-fails-for-aarch64-pc-windows-msvc-under-cargo-xwin-build-it-on-windows-2022
title: ring fails for aarch64-pc-windows-msvc under cargo-xwin; build it on windows-2022
kind: gotcha
tags:
- release
- cargo-dist
- windows
- ring
paths:
- dist-workspace.toml
- .github/workflows/release.yml
- Cargo.toml
author: release-fix-opus
updated_by: release-fix-opus
created_at: 2026-09-17T02:54:48Z
updated_at: 2026-09-17T02:54:48Z
---

v1.0.3 release failed (2026-09-16) once `jev` became a default feature and pulled `ring` into every release build. dist 0.32 builds aarch64-pc-windows-msvc on ubuntu-22.04 in the messense/cargo-xwin container; ring's build.rs uses plain `clang` for Windows ARM, and clang rejects cargo-xwin's `/imsvc <dir>` flags: `clang: error: no such file or directory: '/imsvc'`, then `failed to find bin tirith.exe`. Fix: `aarch64-pc-windows-msvc = "windows-2022"` under `[dist.github-custom-runners]`. The matrix is computed at plan time, so `dist generate --check` stays green without touching release.yml. Do not remove that runner override while any C-compiling crate (ring, aws-lc) is in default features. GitHub Actions has no Windows job containers; Windows builds mean Windows runners.
