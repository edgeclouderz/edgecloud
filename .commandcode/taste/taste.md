# Taste (Continuously Learned by [CommandCode][cmd])

[cmd]: https://commandcode.ai/

# ci
- Require all CI checks to pass green before merging PRs; pre-existing failures on unrelated crates/jobs must be resolved. Confidence: 0.90
- After merging a PR, monitor the CI run on main until completion and report results. Confidence: 0.85
