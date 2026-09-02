#!/usr/bin/env bash
set -Eeuo pipefail

# Push reviewed Hanlin changes to GitHub.
# Usage: ./push_changes.sh [--yes] [commit message]
# Environment overrides: REMOTE=origin BRANCH=main

REMOTE="${REMOTE:-origin}"
BRANCH="${BRANCH:-main}"
AUTO_CONFIRM=0
DEFAULT_MESSAGE="feat: synchronize Hanlin version and add compound assignments"
COMMIT_MESSAGE=""

usage() {
    cat <<'EOF'
Usage: ./push_changes.sh [options] [commit message]

Options:
  --yes       Skip the final confirmation prompt.
  -h, --help  Show this help message.

Environment:
  REMOTE      Git remote name (default: origin)
  BRANCH      Branch to push (default: main)

The script runs formatting, tests, diff checks, stages all current changes,
creates a commit, and pushes it to the selected remote and branch.
EOF
}

while (($# > 0)); do
    case "$1" in
        --yes)
            AUTO_CONFIRM=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            if [[ -n "$COMMIT_MESSAGE" ]]; then
                echo "Error: commit message was provided more than once." >&2
                exit 2
            fi
            COMMIT_MESSAGE="$1"
            shift
            ;;
    esac
done

COMMIT_MESSAGE="${COMMIT_MESSAGE:-$DEFAULT_MESSAGE}"

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

require_command() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "Error: required command not found: $1" >&2
        exit 1
    }
}

require_command git
require_command cargo

if [[ ! -f Cargo.toml ]]; then
    echo "Error: Cargo.toml was not found in $SCRIPT_DIR" >&2
    exit 1
fi

if [[ "$(git rev-parse --show-toplevel)" != "$SCRIPT_DIR" ]]; then
    echo "Error: script must run from the repository root." >&2
    exit 1
fi

if ! git remote get-url "$REMOTE" >/dev/null 2>&1; then
    echo "Error: Git remote '$REMOTE' does not exist." >&2
    exit 1
fi

CURRENT_BRANCH="$(git branch --show-current)"
if [[ "$CURRENT_BRANCH" != "$BRANCH" ]]; then
    echo "Error: current branch is '$CURRENT_BRANCH', expected '$BRANCH'." >&2
    echo "Use BRANCH=<branch> ./push_changes.sh if this is intentional." >&2
    exit 1
fi

echo "==> Running formatting check"
if cargo fmt -- --check; then
    :
else
    echo "Formatting differs. Run 'cargo fmt' and review the changes before pushing." >&2
    exit 1
fi

echo "==> Running tests"
cargo test --all-targets

echo "==> Running whitespace/error checks"
git diff --check

echo "==> Checking working tree"
if [[ -z "$(git status --porcelain)" ]]; then
    echo "Nothing to commit; working tree is clean."
    exit 0
fi

git status --short

echo
echo "Remote:  $REMOTE ($(git remote get-url "$REMOTE"))"
echo "Branch:  $BRANCH"
echo "Commit:  $COMMIT_MESSAGE"
echo

git add -A
git diff --cached --check

echo "==> Staged changes"
git diff --cached --stat

echo

if (( ! AUTO_CONFIRM )); then
    read -r -p "Create this commit and push to $REMOTE/$BRANCH? [y/N] " answer
    case "$answer" in
        y|Y|yes|YES)
            ;;
        *)
            echo "Cancelled. No commit or push was performed."
            git reset
            exit 0
            ;;
    esac
fi

git commit -m "$COMMIT_MESSAGE"
git push "$REMOTE" "HEAD:$BRANCH"
echo "Push completed successfully."
