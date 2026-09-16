---
id: 8dfa1033-4b67-4265-a24b-d5bfba93df1f
permalink: claim-paths-cover-everything-beneath-them
title: Claim paths cover everything beneath them
kind: decision
tags: []
paths:
- src/types.rs
- src/claims.rs
author: claude-scaffold
updated_by: claude-scaffold
created_at: 2026-09-16T00:15:57Z
updated_at: 2026-09-16T00:15:57Z
---

RepoPath strips trailing slashes; overlap is equality or ancestor-at-segment-boundary, so a claim on src/auth covers src/auth/login.rs but not src/authz

## Rationale

Agents should not have to remember a trailing-slash convention to protect a directory
