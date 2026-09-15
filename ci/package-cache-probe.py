#!/usr/bin/env python3
"""Produce an immutable library distribution without Cargo's integration fixtures."""

import argparse
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("revision")
    args = parser.parse_args()
    repository = Path(__file__).resolve().parent.parent

    def git(*arguments, env=None, data=None):
        return subprocess.run(
            ["git", "-C", str(repository), "-c", "commit.gpgsign=false", *arguments],
            input=data,
            stdout=subprocess.PIPE,
            check=True,
            env=env,
        ).stdout

    source = git("rev-parse", "--verify", f"{args.revision}^{{commit}}").decode().strip()
    timestamp = git("show", "-s", "--format=%ct", source).decode().strip()
    with tempfile.TemporaryDirectory(prefix="cargo-probe-package-") as temporary:
        env = dict(os.environ)
        env.update(
            GIT_INDEX_FILE=str(Path(temporary) / "index"),
            GIT_AUTHOR_NAME="Green-Got Cargo distribution",
            GIT_AUTHOR_EMAIL="cargo-distribution@green-got.com",
            GIT_COMMITTER_NAME="Green-Got Cargo distribution",
            GIT_COMMITTER_EMAIL="cargo-distribution@green-got.com",
            GIT_AUTHOR_DATE=f"{timestamp} +0000",
            GIT_COMMITTER_DATE=f"{timestamp} +0000",
        )
        git("read-tree", source, env=env)
        fixtures = git("ls-files", "-z", "--", "tests/testsuite", env=env)
        if not fixtures:
            parser.error("revision has no integration fixtures; expected the development source")
        git("update-index", "--force-remove", "-z", "--stdin", env=env, data=fixtures)
        tree = git("write-tree", env=env).decode().strip()
        message = f"Package cache-probe libraries from {source}\n\nOmit integration fixtures from recursive Git package discovery.\n"
        commit = git("commit-tree", tree, "-p", source, env=env, data=message.encode())
        print(commit.decode().strip())


if __name__ == "__main__":
    main()
