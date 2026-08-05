#!/bin/sh
# Build the throwaway repository the demo tape edits (demo/redate.tape).
#
# The demo rewrites this history, so it has to be recreated from scratch
# before every render - that is why it is a script and not a fixture
# committed to the repo. Everything is pinned (identity, dates, file
# contents, no signing), so the commit ids come out the same on every
# run and the recording is reproducible.
#
# The story the dates tell: someone hacked on a toy todo CLI between
# 02:11 and 03:40 one night, and wants the history to say they did it
# during the day. Tag v0.1.0 sits INSIDE the rewritten range so the demo
# also shows a tag following its commit.
#
# Usage: sh make-demo-repo.sh <dir>   (the dir is wiped first)
set -e

dir=${1:?usage: make-demo-repo.sh <dir>}
rm -rf "$dir"
mkdir -p "$dir"
cd "$dir"

git init -q -b master .
# Repo-local identity: git-redate refuses to start without a committer
# (the reflog entry needs one), and the demo must not depend on - or
# leak - whatever identity the host or the container happens to have.
git config user.name "Ada Lovelace"
git config user.email "ada@example.com"
# The container has no keys; make sure a host-inherited signing config
# can never turn these into signed commits.
git config commit.gpgsign false
git config tag.gpgsign false

# commit <iso8601-date> <message> <file> <line>
commit() {
	printf '%s\n' "$4" >>"$3"
	git add -- "$3"
	GIT_AUTHOR_DATE="$1" GIT_COMMITTER_DATE="$1" git commit -q -m "$2"
}

commit "2026-01-13T23:41:12+08:00" "chore: initial commit" README.md "# todo"
commit "2026-01-14T02:11:05+08:00" "feat: parse the todo file" parse.py "def parse(text):"
commit "2026-01-14T02:14:47+08:00" "feat: add the add subcommand" cli.py "def add(item):"
git tag v0.1.0
commit "2026-01-14T02:29:33+08:00" "fix: keep blank lines out of the index" parse.py "    lines = [l for l in text.splitlines() if l]"
commit "2026-01-14T03:02:18+08:00" "test: cover the empty file case" test_parse.py "def test_empty():"
commit "2026-01-14T03:40:52+08:00" "docs: write the usage section" README.md "Usage: todo add <item>"
