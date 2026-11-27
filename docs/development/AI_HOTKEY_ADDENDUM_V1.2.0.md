[SYNCUP AI HARNESS v1.2.0 — UPDATED OPERATING CONTRACT]

The local keyboard automation has been upgraded. Preserve all existing project, phase, architecture, safety and scope rules. This addendum changes only how local JSON payloads are executed.

1. JSON OUTPUT
Whenever local execution is needed, return exactly one fenced, valid JSON object using the established schema:
{
  "status": "preflight | need_context | ready | blocked",
  "summary": "...",
  "assumptions": [],
  "questions": [],
  "files": [],
  "edits": [],
  "patch": "",
  "commands": []
}

2. STATUS BEHAVIOR
- preflight: plan only. Empty files, edits, patch and commands. No execution.
- need_context: read-only context commands only. One shortcut press executes them and returns the output to the clipboard.
- ready: one shortcut press performs dry-run, write-only application, prints a concise Git status/diff summary, then automatically runs the verification commands. There is NO manual diff-review pause and NO second shortcut press in the default workflow.
- blocked: nothing executes.

A ready payload is used only after the human has approved the plan and authorized implementation. Ensure questions is an empty array before returning ready.

3. VISIBLE PROGRESS
The hotkey opens a visible terminal. The user sees timestamped stage messages, each command, streamed output, exit codes, and a heartbeat message every few seconds when a long command produces no output. Do not ask the user to wait for a silent background process. When complete, ai-run-output.txt opens in Mousepad, is copied to the clipboard, and a notification asks the user to paste it back into this chat.

4. COMMAND RULES
- Put each command in a separate commands array item.
- Never use shell operators: |, ;, &, <, >, backticks or $().
- Never use command chaining or shell scripts.
- Use project-relative paths only.
- need_context commands must be read-only and must use the local harness-supported read-only executables: rg, grep, cat, head, tail, ls, find, tree, pwd, stat, file, and read-only git commands.
- Do not use sed, sudo, rm, python, or write-capable commands in need_context.
- ready verification commands must use executables allowed by the local harness (for example cargo, npm, node, git read-only commands, rg, grep, cat, head, tail, ls, find, tree, pwd, stat, file, diff, patch, gio, echo).
- Cargo commands must explicitly use --manifest-path src-tauri/Cargo.toml.
- npm commands run from the repository root.
- Do not include git add, git commit, git push, git reset, git restore, git checkout or git clean commands in payloads.

5. SPECTA BINDINGS SPECIAL CASE
If and only if the ready payload's commands array contains the exact command "npm run specta:check", the local orchestrator automatically:
1. Confirms src/lib/ipc.ts had no pre-existing Git changes before this run.
2. Runs "npm run gen:bindings".
3. Stages ONLY "src/lib/ipc.ts" with "git add -- src/lib/ipc.ts".
4. Runs the ready payload verification commands, including npm run specta:check.

This is a narrow generated-file staging exception required because specta:check compares src/lib/ipc.ts against the Git index. It never stages other files and never commits. If src/lib/ipc.ts already had pre-existing changes, the orchestrator stops and asks for manual handling. Do not ask the user to manually stage this file when the automation reports that it handled it successfully.

6. VERIFICATION DISCIPLINE
- Include the complete phase gate in ready.commands, one command per item.
- Do not claim tests passed until the pasted output confirms actual EXIT=0 results.
- If a command fails, stop and analyze the actual output. Request additional read-only context when needed, then return a new ready payload for a narrow correction.
- Do not make a commit or claim commit authorization. The human remains responsible for Git commits and pushes.
- The automation does not automatically roll back changes after failed tests. Existing per-file backups and Git history remain the recovery mechanisms.

7. OPTIONAL MANUAL REVIEW
The human can explicitly request the older two-press manual review mode for an unusually sensitive change. Do not make it the default and do not instruct the user to inspect ai-review-diff.txt during ordinary ready workflows.

Acknowledge this addendum with one line and continue using the established Phase 6 context and rules.
