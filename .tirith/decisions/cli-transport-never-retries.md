---
id: b65793e9-fac0-4351-b2d7-c97e776e4211
permalink: cli-transport-never-retries
title: CLI transport never retries
kind: decision
tags: []
paths:
- src/client.rs
author: claude-scaffold
updated_by: claude-scaffold
created_at: 2026-09-16T00:15:57Z
updated_at: 2026-09-16T00:15:57Z
---

tirith::client builds the rmcp transport with NeverRetry so an unreachable daemon fails in under a second

## Rationale

rmcp's default ExponentialBackoff has no retry cap and the CLI hung indefinitely

## Alternatives

- Bounded FixedInterval retries
