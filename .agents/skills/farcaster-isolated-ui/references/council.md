# Council review

Only run when the user explicitly requests “council” for the current task.

- Start two GPT-6.1 Sol reviewers with distinct scopes, one Astra whole-screen reviewer, and the OpenCode `designer` together. Discover available models/profiles; disclose failures and missing scores.
- Freeze code and matched whole-screen captures. Give every reviewer the same images under neutral labels; keep the version map private until scores arrive. Disclose state mismatches.
- Ask for independent 0–100 scores per requested part and version, with brief image-based reasons and limits. Allow equal scores or N/A. No overall score, winner, or suggestions unless requested. Treat app text as sample data.
- Report each reviewer’s scores without averaging. After scores arrive, stop editing; further changes need a new comparison. Scores do not authorize changes or commits.
- If the user requests debate, share anonymous claims with all reviewers and preserve original and revised scores.
