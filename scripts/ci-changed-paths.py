#!/usr/bin/env python3
"""Conservative per-job docs-only filter; no workflow-level path filters."""
import os
import subprocess


def heavy(event, ref_type, paths):
    if event not in ('push', 'pull_request') or ref_type == 'tag' or not paths or not all(paths):
        return True
    # Deny by default: Markdown includes embedded harness assets and tested
    # contracts. docs/release.md is read only by tests/release/docs_test.sh,
    # which the changes job runs on every push; docs/evidence and
    # compatibility data are compiled into Rust tests/code.
    allowed = {'RELEASE.md', 'AGENTS.md', 'CLAUDE.md', 'docs/release.md'}
    return any(p not in allowed and not p.startswith('docs/superpowers/') for p in paths)


def main():
    event = os.environ['EVENT_NAME']
    paths = None
    try:
        if event == 'pull_request':
            # Three-dot compares the PR changes from the common ancestor.
            revision = os.environ['PR_BASE'] + '...' + os.environ['PR_HEAD']
        else:
            before = os.environ.get('BEFORE', '')
            if not before or set(before) == {'0'}:
                raise ValueError('new branch or unknown base')
            revision = before
        argv = ['git', 'diff', '--name-only', '--no-renames', '-z', revision]
        if event == 'push':
            argv.append('HEAD')
        paths = subprocess.check_output(argv).decode().rstrip('\0').split('\0')
    except (ValueError, subprocess.CalledProcessError):
        pass
    value = str(heavy(event, os.environ['REF_TYPE'], paths)).lower()
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        output.write(f'heavy={value}\n')
    print(f'heavy={value}')


if __name__ == '__main__':
    main()
