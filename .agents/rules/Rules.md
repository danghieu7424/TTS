---

## Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

## 5. Git Workflow (Incremental Commit & Push)

**Commit small, verified units of work immediately.**

- **Scope:** Commit and push immediately after completing and verifying each isolated task (one bug fix, one function, or one logical change). Never bundle unrelated changes into a single commit.
- **Pre-commit check:** Ensure code compiles/passes tests and `graphify update .` has been run (if code files changed) before staging.
- **Stage surgically:** Only stage files modified for the current task (`git add <file>`). Avoid `git add .` if unrelated untracked/modified files exist.
- **Commit message:** Write a concise message describing *what* changed and *why*.
- **Push:** Run `git push` to the current upstream branch right after committing. Never use `--force`. Only use `--force-with-lease` on personal feature branches immediately after an explicit `git commit --amend` or `rebase`—never to bypass non-fast-forward rejections, and never on `main`/`master`.

## 6. Versioning (Semantic Versioning)

**Bump `Cargo.toml` version intentionally based on scope (`MAJOR.MINOR.PATCH`).**

- **PATCH (`0.0.x`):** Bug fixes, internal refactors, or docs/test updates with no new public API or breaking changes.
- **MINOR (`0.x.0`):** New backward-compatible public APIs, features, or format support.
- **MAJOR (`x.0.0`):** Breaking changes to public APIs, CLI flags/output formats, or `#[non_exhaustive]` assumptions.
- **Pre-`1.0.0` (`0.y.z`):** Treat minor/breaking changes during initial development carefully; explicitly note breaking changes.
- **Tagging:** Only create an annotated Git tag (`git tag -a vX.Y.Z -m "..."`) after updating `Cargo.toml` and `CHANGELOG.md`, verifying `cargo test` passes, and running `graphify update .`.

## 7. Changelog Maintenance (Keep a Changelog)

**Keep `CHANGELOG.md` synchronized with every versioned or user-facing change.**

- **Format & Language:** Follow [Keep a Changelog 1.1.0](https://keepachangelog.com/vi/1.1.0/) in **Vietnamese**, matching the existing file structure (`## [X.Y.Z] — YYYY-MM-DD`).
- **Standard Sections:** Group entries strictly under existing Vietnamese headings: `### Thêm`, `### Thay đổi`, `### Sửa`, `### Gỡ bỏ`, `### Bảo mật`, and `### Thay đổi phá vỡ tương thích`.
- **Entry Style:** Write concise, technical bullet points naming exact public items, modules, CLI subcommands, or formats affected (e.g., `` `read_mp4` ``, `` `Mkv::cues_text` ``).
- **Workflow:** Update `CHANGELOG.md` (under `## [Unreleased]` during development or the target `## [X.Y.Z] — YYYY-MM-DD` on release) *before* committing in Mục 5.

## 8. Repository Hygiene, Quality Gates & Documentation Sync

**Keep the repository clean, enforce strict lint/property-test gates, and prevent documentation drift.**

- **Repository Hygiene & Review Logs:**
  - Remove temporary local verification scripts/logs (e.g., `verify.bat`, `verify.log`) before committing.
  - Keep local tool artifacts (e.g., `graphify-out/`, local `.cargo-*` caches) in `.gitignore`. If already tracked, untrack via `git rm -r --cached <path>` while preserving local files on disk.
  - Archive detailed session evaluations under `docs/reviews/YYYY-MM-DD-session-log.md` and keep root `REVIEW.md` concise with verified current status only (passing test count, audit status).

- **Naming & Strict Lint Compliance:**
  - Follow ecosystem naming conventions (e.g., `kebab-case` for Rust package/crate names) and keep all workspace/example internal dependencies synchronized.
  - Enforce zero-warning builds across all targets (`cargo clippy --workspace --all-targets -- -D warnings`).

- **Property-Based Testing for Core Logic:**
  - Use property-based testing (e.g., `proptest`) on low-level binary codecs, parsers, serialization, and complex state/page structures to cover boundary values (such as multi-byte integer limits and page splits) beyond hand-written unit tests.

- **Zero-Drift Documentation (`README.md`):**
  - Document exact local toolchain/environment setup prerequisites (e.g., Windows MSVC `vcvars64.bat`, custom `CARGO_HOME`).
  - Explicitly state current technical limitations and unimplemented features; never document planned behavior as implemented.
  - Keep the workspace crate/module table and test/lint/audit commands (`cargo fmt`, `cargo test`, `cargo clippy`, `cargo audit`) 100% synchronized with the codebase.

- **Final Verification Gate:**
  - Before concluding a task or milestone, run the full verification suite inside the project's configured toolchain environment: formatting check (`cargo fmt --all -- --check`), workspace tests (`cargo test --workspace`), strict lints (`cargo clippy --workspace --all-targets -- -D warnings`), and dependency security audit (`cargo audit`).

## 9. Evaluation Reports & Repository Review (`evaluate` & `REVIEW.md`)

**Separate empirical evaluation logs from the concise repository status summary, and back every claim with reproducible evidence.**

- **Two-Tier Structure (`evaluate` vs `REVIEW.md`):**
  - **Evaluation Log (`docs/reviews/YYYY-MM-DD-evaluate.md` or session `evaluate.md`):** Record detailed, date-stamped empirical benchmarks, dataset comparisons, regression checks, and exact verification commands for a specific run.
  - **Root `REVIEW.md`:** Maintain a concise, up-to-date snapshot of the repository's current health (passing test counts, clippy/fmt/audit status, active architectural constraints, and a pointer to the latest evaluation report). Do not duplicate raw per-sample tables in `REVIEW.md`.

- **Data Integrity & Artifact Discipline in Evaluations:**
  - Explicitly confirm that ground-truth/reference inputs were unmodified (e.g., byte-identical hash check) and temporary `scratch/` directories were cleaned up after comparison.
  - Store retained run outputs (logs, generated files, visual/crop evidence) under dated output directories (e.g., `output/<run-id>/`) excluded from Git.

- **Quantitative Metrics & Regression Verification:**
  - Present per-item results in a Markdown table with a **Total** summary row, followed by exact metric definitions, alignment/matching thresholds, and the exact CLI flags used.
  - Warn explicitly whenever current normalization/configuration flags differ from historical runs so incompatible metrics are not compared directly.
  - Verify reported bugs individually against concrete artifacts (file paths, timestamps, exact output strings, or confidence scores).

- **Scope Separation & Provenance Honesty:**
  - Clearly separate results of the current run from previous runs or distinct test subsets.
  - If artifacts from an earlier run are absent from the current working copy, state this limitation explicitly—never substitute historical statistics or unverified assumptions for fresh verification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.